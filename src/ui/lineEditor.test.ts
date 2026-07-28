import assert from "node:assert/strict";
import { PassThrough } from "node:stream";
import test from "node:test";
import { InputHistory } from "#src/ui/InputHistory.js";
import { EOF, ghostPrompt } from "#src/ui/lineEditor.js";

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

function pressKey(
  input: TestInput,
  name: string,
  text?: string,
  ctrl = false,
): void {
  input.emit("keypress", text, {
    sequence: text ?? "",
    name,
    ctrl,
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

test("clears entered text on ctrl+c without closing the prompt", async () => {
  const input = new TestInput();
  const output = new PassThrough();
  const answer = ghostPrompt({
    prompt: "> ",
    getCommands: () => [],
    input,
    output,
  });

  pressKey(input, "a", "draft");
  pressKey(input, "c", undefined, true);
  pressKey(input, "a", "replacement");
  pressKey(input, "return");

  assert.equal(await answer, "replacement");
});

test("requires ctrl+c twice to close an empty prompt", async () => {
  const input = new TestInput();
  const output = new PassThrough();
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

  pressKey(input, "c", undefined, true);
  assert.match(rendered, /Press ctrl \+ c again to exit\./);
  assert.equal(input.isRaw, true);
  pressKey(input, "c", undefined, true);

  assert.equal(await answer, EOF);
});

test("delegates ctrl+c to a foreground operation", async () => {
  const input = new TestInput();
  const output = new PassThrough();
  let interrupted = false;
  const answer = ghostPrompt({
    prompt: "> ",
    getCommands: () => [],
    input,
    output,
    onInterrupt: () => {
      interrupted = true;
      return true;
    },
  });

  pressKey(input, "c", undefined, true);

  assert.equal(interrupted, true);
  assert.equal(await answer, EOF);
  assert.equal(input.isRaw, false);
});
