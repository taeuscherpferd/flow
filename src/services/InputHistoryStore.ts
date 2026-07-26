import { randomUUID } from "node:crypto";
import {
  mkdir,
  readFile,
  readdir,
  rename,
  unlink,
  writeFile,
} from "node:fs/promises";
import path from "node:path";

const HISTORY_FILE_NAME = "input-history.json";
const HISTORY_FILE_VERSION = 1;
const LOCK_RETRY_INTERVAL_MS = 10;
const LOCK_WAIT_TIMEOUT_MS = 5_000;
const LOCK_CHOOSING_SUFFIX = ".choosing";
const LOCK_TICKET_SUFFIX = ".ticket";

interface SerializedInputHistory {
  version: number;
  entries: string[];
}

interface LockEntry {
  fileName: string;
  pid: number;
  ticket: number | undefined;
}

export class InputHistoryStoreError extends Error {}

export class InputHistoryStore {
  readonly filePath: string;
  private readonly lockDirectoryPath: string;

  constructor(globalDir: string) {
    this.filePath = path.join(globalDir, HISTORY_FILE_NAME);
    this.lockDirectoryPath = `${this.filePath}.locks`;
  }

  async load(): Promise<string[]> {
    let raw: string;
    try {
      raw = await readFile(this.filePath, "utf-8");
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return [];
      throw error;
    }

    try {
      const parsed = JSON.parse(raw) as Partial<SerializedInputHistory> | null;
      if (
        parsed === null ||
        parsed.version !== HISTORY_FILE_VERSION ||
        !Array.isArray(parsed.entries) ||
        !parsed.entries.every((entry) => typeof entry === "string")
      ) {
        throw new Error("Unsupported input history format.");
      }
      return [...parsed.entries];
    } catch (error) {
      throw new InputHistoryStoreError(
        `Failed to parse ${this.filePath}: ${String(error)}`,
      );
    }
  }

  async append(input: string, limit: number): Promise<void> {
    if (input.length === 0) return;
    if (!Number.isInteger(limit) || limit < 1) {
      throw new RangeError("Input history limit must be a positive integer.");
    }

    await mkdir(path.dirname(this.filePath), { recursive: true });
    const lockPath = await this.acquireLock();

    try {
      let entries: string[];
      try {
        entries = await this.load();
      } catch (error) {
        if (!(error instanceof InputHistoryStoreError)) throw error;
        entries = [];
      }

      entries.push(input);
      await this.writeAtomically(entries.slice(-limit));
    } finally {
      await this.removeFileIfExists(lockPath);
    }
  }

  private async writeAtomically(entries: readonly string[]): Promise<void> {
    const history: SerializedInputHistory = {
      version: HISTORY_FILE_VERSION,
      entries: [...entries],
    };
    const temporaryPath = `${this.filePath}.${randomUUID()}.tmp`;

    try {
      await writeFile(
        temporaryPath,
        `${JSON.stringify(history, null, 2)}\n`,
        { encoding: "utf-8", mode: 0o600 },
      );
      await rename(temporaryPath, this.filePath);
    } catch (error) {
      await this.removeFileIfExists(temporaryPath);
      throw error;
    }
  }

  private async acquireLock(): Promise<string> {
    await mkdir(this.lockDirectoryPath, { recursive: true });

    const participantId = `${process.pid}-${randomUUID()}`;
    const choosingPath = path.join(
      this.lockDirectoryPath,
      `${participantId}${LOCK_CHOOSING_SUFFIX}`,
    );
    let ticketPath: string | undefined;
    const deadline = Date.now() + LOCK_WAIT_TIMEOUT_MS;

    try {
      await writeFile(choosingPath, "", { flag: "wx", mode: 0o600 });
      await this.removeAbandonedLockEntries();

      const entries = await this.listLockEntries();
      const highestTicket = entries.reduce(
        (highest, entry) => Math.max(highest, entry.ticket ?? 0),
        0,
      );
      const ticket = highestTicket + 1;
      if (!Number.isSafeInteger(ticket)) {
        throw new InputHistoryStoreError(
          `Could not allocate a lock ticket for ${this.filePath}.`,
        );
      }

      ticketPath = path.join(
        this.lockDirectoryPath,
        `${ticket}-${participantId}${LOCK_TICKET_SUFFIX}`,
      );
      await writeFile(ticketPath, "", { flag: "wx", mode: 0o600 });
      await this.removeFileIfExists(choosingPath);

      for (;;) {
        await this.removeAbandonedLockEntries();
        const currentEntries = await this.listLockEntries();
        const anotherProcessIsChoosing = currentEntries.some(
          (entry) => entry.ticket === undefined,
        );
        const firstTicket = currentEntries
          .filter(
            (entry): entry is LockEntry & { ticket: number } =>
              entry.ticket !== undefined,
          )
          .sort(
            (left, right) =>
              left.ticket - right.ticket ||
              (left.fileName === right.fileName
                ? 0
                : left.fileName < right.fileName
                  ? -1
                  : 1),
          )[0];

        if (
          !anotherProcessIsChoosing &&
          firstTicket?.fileName === path.basename(ticketPath)
        ) {
          return ticketPath;
        }

        if (Date.now() >= deadline) {
          throw new InputHistoryStoreError(
            `Timed out waiting to update ${this.filePath}.`,
          );
        }

        await new Promise<void>((resolve) => {
          setTimeout(resolve, LOCK_RETRY_INTERVAL_MS);
        });
      }
    } catch (error) {
      await this.removeFileIfExists(choosingPath);
      if (ticketPath) await this.removeFileIfExists(ticketPath);
      throw error;
    }
  }

  private async removeAbandonedLockEntries(): Promise<void> {
    const fileNames = await readdir(this.lockDirectoryPath);
    for (const fileName of fileNames) {
      if (
        !fileName.endsWith(LOCK_CHOOSING_SUFFIX) &&
        !fileName.endsWith(LOCK_TICKET_SUFFIX)
      ) {
        continue;
      }

      const entry = this.parseLockEntry(fileName);
      if (entry && this.isProcessRunning(entry.pid)) continue;
      await this.removeFileIfExists(
        path.join(this.lockDirectoryPath, fileName),
      );
    }
  }

  private async listLockEntries(): Promise<LockEntry[]> {
    const fileNames = await readdir(this.lockDirectoryPath);
    const entries: LockEntry[] = [];
    for (const fileName of fileNames) {
      const entry = this.parseLockEntry(fileName);
      if (entry) entries.push(entry);
    }
    return entries;
  }

  private parseLockEntry(fileName: string): LockEntry | undefined {
    const choosingMatch = fileName.match(
      /^(\d+)-[0-9a-f-]+\.choosing$/i,
    );
    if (choosingMatch) {
      const pid = Number(choosingMatch[1]);
      return Number.isSafeInteger(pid) && pid > 0
        ? { fileName, pid, ticket: undefined }
        : undefined;
    }

    const ticketMatch = fileName.match(
      /^(\d+)-(\d+)-[0-9a-f-]+\.ticket$/i,
    );
    if (!ticketMatch) return undefined;

    const ticket = Number(ticketMatch[1]);
    const pid = Number(ticketMatch[2]);
    return Number.isSafeInteger(ticket) &&
      ticket > 0 &&
      Number.isSafeInteger(pid) &&
      pid > 0
      ? { fileName, pid, ticket }
      : undefined;
  }

  private isProcessRunning(pid: number): boolean {
    try {
      process.kill(pid, 0);
      return true;
    } catch (error) {
      return (error as NodeJS.ErrnoException).code !== "ESRCH";
    }
  }

  private async removeFileIfExists(filePath: string): Promise<void> {
    try {
      await unlink(filePath);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    }
  }
}
