import assert from "node:assert/strict";
import test from "node:test";
import { InputHistory } from "#src/ui/InputHistory.js";

test("navigates from newest input to oldest input", () => {
  const history = new InputHistory();
  history.record("first");
  history.record("second");
  const navigator = history.startNavigation();

  assert.equal(navigator.previous(""), "second");
  assert.equal(navigator.previous("second"), "first");
  assert.equal(navigator.previous("first"), "first");
});

test("navigates forward and restores the current draft", () => {
  const history = new InputHistory();
  history.record("first");
  history.record("second");
  const navigator = history.startNavigation();

  assert.equal(navigator.previous("unfinished draft"), "second");
  assert.equal(navigator.previous("second"), "first");
  assert.equal(navigator.next("first"), "second");
  assert.equal(navigator.next("second"), "unfinished draft");
  assert.equal(navigator.next("unfinished draft"), "unfinished draft");
});

test("ignores empty input", () => {
  const history = new InputHistory();
  history.record("");
  const navigator = history.startNavigation();

  assert.equal(navigator.previous("draft"), "draft");
});

test("loads existing entries and retains only the configured limit", () => {
  const history = new InputHistory(["first", "second", "third"], 2);

  assert.deepEqual(history.snapshot(), ["second", "third"]);

  history.record("fourth");

  assert.deepEqual(history.snapshot(), ["third", "fourth"]);
});

test("rejects invalid history limits", () => {
  assert.throws(
    () => new InputHistory([], 0),
    /Input history limit must be a positive integer/,
  );
});
