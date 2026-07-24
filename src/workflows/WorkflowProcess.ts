import { spawn } from "node:child_process";
import type {
  WorkflowExecOptions,
  WorkflowExecResult,
} from "#src/workflows/types.js";
import { terminateProcessTree } from "#src/workflows/WorkflowProcessTree.js";

const DEFAULT_MAX_OUTPUT_BYTES = 5 * 1024 * 1024;

export class WorkflowCommandError extends Error {
  constructor(readonly result: WorkflowExecResult) {
    super(
      result.stderr.trim() ||
        `"${result.command}" exited with code ${result.exitCode}.`,
    );
  }
}

export function runWorkflowCommand(
  projectDir: string,
  signal: AbortSignal,
  command: string,
  args: string[] = [],
  options: WorkflowExecOptions = {},
): Promise<WorkflowExecResult> {
  signal.throwIfAborted();
  const maxOutputBytes =
    options.maxOutputBytes ?? DEFAULT_MAX_OUTPUT_BYTES;
  if (!Number.isInteger(maxOutputBytes) || maxOutputBytes < 1) {
    throw new Error("Command maxOutputBytes must be a positive integer.");
  }
  if (
    options.timeoutMs !== undefined &&
    (!Number.isInteger(options.timeoutMs) || options.timeoutMs < 1)
  ) {
    throw new Error("Command timeoutMs must be a positive integer.");
  }

  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? projectDir,
      windowsHide: true,
      detached: process.platform !== "win32",
      env: { ...process.env, ...options.env },
    });
    const stdout: Buffer[] = [];
    const stderr: Buffer[] = [];
    let outputBytes = 0;
    let settled = false;
    let stopping = false;
    let timeout: NodeJS.Timeout | undefined;

    const finish = (
      complete: () => void,
    ): void => {
      if (settled) return;
      settled = true;
      signal.removeEventListener("abort", abort);
      if (timeout) clearTimeout(timeout);
      complete();
    };
    const stop = (error: Error): void => {
      if (settled || stopping) return;
      stopping = true;
      void terminateProcessTree(child).finally(() =>
        finish(() => reject(error)),
      );
    };
    const abort = (): void =>
      stop(new Error(`Command "${command}" was cancelled.`));
    const collect = (target: Buffer[], chunk: Buffer): void => {
      if (stopping) return;
      outputBytes += chunk.byteLength;
      if (outputBytes > maxOutputBytes) {
        stop(
          new Error(
            `Command "${command}" exceeded the ${maxOutputBytes}-byte output limit.`,
          ),
        );
        return;
      }
      target.push(chunk);
    };

    signal.addEventListener("abort", abort, { once: true });
    child.stdout.on("data", (chunk: Buffer) => collect(stdout, chunk));
    child.stderr.on("data", (chunk: Buffer) => collect(stderr, chunk));
    child.stdin.on("error", () => undefined);
    child.on("error", (error) => finish(() => reject(error)));
    child.on("close", (code) => {
      if (stopping) return;
      const result: WorkflowExecResult = {
        command,
        args: [...args],
        stdout: Buffer.concat(stdout).toString("utf-8"),
        stderr: Buffer.concat(stderr).toString("utf-8"),
        exitCode: code ?? -1,
      };
      finish(() => {
        if (result.exitCode === 0 || options.allowFailure) {
          resolve(result);
        } else {
          reject(new WorkflowCommandError(result));
        }
      });
    });

    if (options.timeoutMs !== undefined) {
      timeout = setTimeout(
        () =>
          stop(
            new Error(
              `Command "${command}" timed out after ${options.timeoutMs}ms.`,
            ),
          ),
        options.timeoutMs,
      );
      timeout.unref();
    }
    child.stdin.end(options.input ?? "");
  });
}
