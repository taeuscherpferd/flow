import readline from "node:readline";
import type { InputHistory } from "#src/ui/InputHistory.js";

const DIM = "\x1b[2m";
const RESET = "\x1b[0m";

export const EOF = Symbol("eof");

export interface GhostPromptInput extends NodeJS.ReadableStream {
  readonly isTTY: boolean;
  setRawMode(mode: boolean): void;
}

export interface GhostPromptOutput extends NodeJS.WritableStream {
  readonly columns?: number;
}

export interface GhostPromptOptions {
  prompt: string;
  getCommands: () => string[];
  history?: InputHistory;
  input?: GhostPromptInput;
  output?: GhostPromptOutput;
}

export function ghostPrompt(
  opts: GhostPromptOptions,
): Promise<string | typeof EOF> {
  const input: GhostPromptInput = opts.input ?? process.stdin;
  const output: GhostPromptOutput = opts.output ?? process.stdout;
  const { prompt } = opts;

  if (!input.isTTY) {
    return new Promise((resolve) => {
      const rl = readline.createInterface({ input, output });
      rl.question(prompt, (answer) => {
        rl.close();
        resolve(answer);
      });
      rl.on("close", () => resolve(EOF));
    });
  }

  return new Promise((resolve) => {
    let buffer = "";
    let cursor = 0;
    let renderedCursorRow = 0;
    const historyNavigator = opts.history?.startNavigation();

    function ghost(): string {
      if (cursor !== buffer.length) return "";
      if (!buffer.startsWith("/")) return "";
      const typed = buffer.slice(1);
      if (typed.length === 0 || /\s/.test(typed)) return "";
      const match = opts
        .getCommands()
        .filter((c) => c.startsWith(typed) && c !== typed)
        .sort()[0];
      return match ? match.slice(typed.length) : "";
    }

    function render(): void {
      const g = ghost();
      const columns = Math.max(output.columns ?? 80, 1);
      const endOffset = prompt.length + buffer.length + g.length;
      const targetOffset = prompt.length + cursor;
      const endRow = Math.floor(endOffset / columns);
      const endColumn = endOffset % columns;
      const targetRow = Math.floor(targetOffset / columns);
      const targetColumn = targetOffset % columns;

      readline.cursorTo(output, 0);
      if (renderedCursorRow > 0) {
        readline.moveCursor(output, 0, -renderedCursorRow);
      }
      readline.clearScreenDown(output);
      output.write(prompt + buffer);
      if (g) output.write(DIM + g + RESET);
      if (endOffset > 0 && endColumn === 0) output.write(" ");
      readline.cursorTo(output, targetColumn);
      if (targetRow !== endRow) {
        readline.moveCursor(output, 0, targetRow - endRow);
      }
      renderedCursorRow = targetRow;
    }

    function acceptGhost(): boolean {
      const g = ghost();
      if (!g) return false;
      buffer += g;
      cursor = buffer.length;
      return true;
    }

    function cleanup(): void {
      input.removeListener("keypress", onKey);
      if (input.isTTY) input.setRawMode(false);
      input.pause();
    }

    function finish(value: string | typeof EOF): void {
      cleanup();
      output.write("\n");
      resolve(value);
    }

    function onKey(str: string | undefined, key: readline.Key): void {
      if (key.ctrl && key.name === "c") {
        cleanup();
        output.write("\n");
        process.exit(130);
      }
      if (key.ctrl && key.name === "d") {
        if (buffer.length === 0) return finish(EOF);
        return;
      }

      switch (key.name) {
        case "return":
        case "enter":
          return finish(buffer);
        case "tab":
          acceptGhost();
          return render();
        case "right":
          if (cursor < buffer.length) cursor += 1;
          else acceptGhost();
          return render();
        case "left":
          if (cursor > 0) cursor -= 1;
          return render();
        case "home":
          cursor = 0;
          return render();
        case "end":
          cursor = buffer.length;
          return render();
        case "up":
          if (historyNavigator) {
            buffer = historyNavigator.previous(buffer);
            cursor = buffer.length;
          }
          return render();
        case "down":
          if (historyNavigator) {
            buffer = historyNavigator.next(buffer);
            cursor = buffer.length;
          }
          return render();
        case "backspace":
          if (cursor > 0) {
            buffer = buffer.slice(0, cursor - 1) + buffer.slice(cursor);
            cursor -= 1;
          }
          return render();
        case "delete":
          if (cursor < buffer.length) {
            buffer = buffer.slice(0, cursor) + buffer.slice(cursor + 1);
          }
          return render();
        default:
          break;
      }

      if (
        str &&
        !key.ctrl &&
        !key.meta &&
        str >= " " &&
        !str.includes("\x1b")
      ) {
        buffer = buffer.slice(0, cursor) + str + buffer.slice(cursor);
        cursor += str.length;
        render();
      }
    }

    readline.emitKeypressEvents(input);
    input.setRawMode(true);
    input.resume();
    input.on("keypress", onKey);
    render();
  });
}
