import readline from "node:readline";

const DIM = "\x1b[2m";
const RESET = "\x1b[0m";

export const EOF = Symbol("eof");

export interface GhostPromptOptions {
  prompt: string;
  getCommands: () => string[];
  input?: NodeJS.ReadStream;
  output?: NodeJS.WriteStream;
}

export function ghostPrompt(
  opts: GhostPromptOptions,
): Promise<string | typeof EOF> {
  const input = opts.input ?? process.stdin;
  const output = opts.output ?? process.stdout;
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
      output.write("\r\x1b[K");
      output.write(prompt + buffer);
      if (g) output.write(DIM + g + RESET);
      const end = prompt.length + buffer.length + g.length;
      const target = prompt.length + cursor;
      if (end > target) output.write(`\x1b[${end - target}D`);
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
