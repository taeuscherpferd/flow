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
