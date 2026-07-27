import { InputHistoryStore } from "#src/services/InputHistoryStore.js";
import { InputHistory } from "#src/ui/InputHistory.js";

export class PersistentInputHistory {
  private hasWarned = false;

  private constructor(
    readonly history: InputHistory,
    private readonly store: InputHistoryStore,
  ) {}

  static async create(globalDir: string): Promise<PersistentInputHistory> {
    const store = new InputHistoryStore(globalDir);
    const persistent = new PersistentInputHistory(new InputHistory(), store);
    try {
      return new PersistentInputHistory(
        new InputHistory(await store.load()),
        store,
      );
    } catch (error) {
      persistent.warn("load", String(error));
      return persistent;
    }
  }

  async record(line: string): Promise<void> {
    this.history.record(line);
    try {
      await this.store.append(line, this.history.limit);
    } catch (error) {
      this.warn("save", String(error));
    }
  }

  private warn(action: string, error: string): void {
    if (this.hasWarned) return;
    this.hasWarned = true;
    console.warn(`Warning: could not ${action} input history: ${error}`);
  }
}
