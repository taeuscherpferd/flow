import assert from "node:assert/strict";
import { PassThrough } from "node:stream";
import test from "node:test";
import { InputHistory } from "#src/ui/InputHistory.js";
import { ghostPrompt } from "#src/ui/lineEditor.js";

class TestInput extends PassThrough {
  readonly isTTY = true;
  isRaw = false;

  setRawMode(mode: boolean): void {
    this.isRaw = mode;
  }
}

class TestOutput extends PassThrough {
  constructor(readonly columns: number) {
    super();
  }
}

function pressKey(input: TestInput, name: string, text?: string): void {
  input.emit("keypress", text, {
    sequence: text ?? "",
    name,
    ctrl: false,
    meta: false,
    shift: false,
  });
}

test("uses arrow keys to browse history and restore a draft", async () => {
  const history = new InputHistory();
  history.record("first");
  history.record("second");
  const input = new TestInput();
  const output = new PassThrough();
  const answer = ghostPrompt({
    prompt: "> ",
    getCommands: () => [],
    history,
    input,
    output,
  });

  pressKey(input, "d", "draft");
  pressKey(input, "up");
  pressKey(input, "up");
  pressKey(input, "down");
  pressKey(input, "down");
  pressKey(input, "return");

  assert.equal(await answer, "draft");
  assert.equal(input.isRaw, false);
});

test("clears every previously rendered row when input wraps", async () => {
  const input = new TestInput();
  const output = new TestOutput(10);
  let rendered = "";
  output.on("data", (chunk: Buffer) => {
    rendered += chunk.toString();
  });
  const answer = ghostPrompt({
    prompt: "> ",
    getCommands: () => [],
    input,
    output,
  });

  pressKey(input, "a", "123456789");
  const beforeNextRender = rendered.length;
  pressKey(input, "b", "0");
  pressKey(input, "return");

  const nextRender = rendered.slice(beforeNextRender);
  assert.match(nextRender, /\x1b\[1A\x1b\[0J> 1234567890/);
  assert.equal(await answer, "1234567890");
});
