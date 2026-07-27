import { ThinkingMode } from "#src/providers/types.js";
import type { WorkflowAgentRunOptions } from "#src/workflows/types.js";
import assert from "node:assert/strict";
import test from "node:test";

test("accepts every workflow thinking mode", () => {
  const thinkingModes = Object.values(ThinkingMode);
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
