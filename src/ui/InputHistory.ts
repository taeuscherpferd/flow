export const DEFAULT_INPUT_HISTORY_LIMIT = 500;

export class InputHistory {
  private readonly entries: string[] = [];

  constructor(
    initialEntries: readonly string[] = [],
    readonly limit = DEFAULT_INPUT_HISTORY_LIMIT,
  ) {
    if (!Number.isInteger(limit) || limit < 1) {
      throw new RangeError("Input history limit must be a positive integer.");
    }
    for (const entry of initialEntries) this.record(entry);
  }

  record(input: string): void {
    if (input.length === 0) return;

    this.entries.push(input);
    if (this.entries.length > this.limit) this.entries.shift();
  }

  snapshot(): string[] {
    return [...this.entries];
  }

  startNavigation(): InputHistoryNavigator {
    return new InputHistoryNavigator(this.entries);
  }
}

export class InputHistoryNavigator {
  private index: number;
  private draft = "";

  constructor(private readonly entries: readonly string[]) {
    this.index = entries.length;
  }

  previous(currentInput: string): string {
    if (this.index === 0) return currentInput;
    if (this.index === this.entries.length) this.draft = currentInput;

    this.index -= 1;
    return this.entries[this.index] ?? currentInput;
  }

  next(currentInput: string): string {
    if (this.index === this.entries.length) return currentInput;

    this.index += 1;
    return this.index === this.entries.length
      ? this.draft
      : (this.entries[this.index] ?? currentInput);
  }
}
