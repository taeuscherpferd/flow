import assert from "node:assert/strict";
import test from "node:test";
import type {
  WorkflowAgentRunOptions,
  WorkflowThinking,
} from "#src/workflows/types.js";

test("accepts every workflow thinking mode", () => {
  const thinkingModes = [
    "default",
    "off",
    "on",
    "low",
    "medium",
    "high",
  ] satisfies WorkflowThinking[];
  const options = thinkingModes.map(
    (thinking): WorkflowAgentRunOptions => ({ thinking }),
  );

  assert.deepEqual(
    options.map(({ thinking }) => thinking),
    thinkingModes,
  );
});

const invalidOptions: WorkflowAgentRunOptions = {
  // @ts-expect-error Invalid thinking modes must fail workflow type-checking.
  thinking: "extreme",
};
void invalidOptions;
