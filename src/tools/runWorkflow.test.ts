import assert from "node:assert/strict";
import test from "node:test";
import {
  buildWorkflowSystemContext,
  createRunWorkflowTool,
  type WorkflowToolRuntime,
} from "./runWorkflow.js";
import type { WorkflowRecord } from "../workflows/types.js";

function createRecord(
  name: string,
  policy: "disabled" | "confirm" | "automatic",
  objectInput = false,
): WorkflowRecord {
  return {
    definition: {
      name,
      description: `${name} workflow`,
      agentInvocation: policy,
      ...(objectInput
        ? {
            input: {
              schema: {
                type: "object" as const,
                properties: { value: { type: "string" as const } },
                required: ["value"],
              },
            },
          }
        : {}),
      async run() {
        return null;
      },
    },
    directory: name,
    entryPath: `${name}/WORKFLOW.js`,
    fingerprint: name,
    source: "global",
  };
}

test("runs eligible workflows and confirms when required", async () => {
  const invocations: string[] = [];
  let confirmations = 0;
  const runtime: WorkflowToolRuntime = {
    async resolve(name) {
      return records.find((record) => record.definition.name === name);
    },
    async invoke(record, input) {
      invocations.push(`${record.definition.name}:${JSON.stringify(input)}`);
      return "completed";
    },
    async confirm() {
      confirmations += 1;
      return true;
    },
  };
  const records = [
    createRecord("hidden", "disabled"),
    createRecord("review", "confirm"),
    createRecord("structured", "automatic", true),
  ];
  const tool = createRunWorkflowTool(records, runtime);

  const review = await tool.execute(
    { name: "review", inputText: "change" },
    {
      cwd: process.cwd(),
      requestPermission: async () => true,
      secrets: { get: () => undefined, has: () => false },
    },
  );
  const structured = await tool.execute(
    { name: "structured", input: { value: "ok" } },
    {
      cwd: process.cwd(),
      requestPermission: async () => true,
      secrets: { get: () => undefined, has: () => false },
    },
  );

  assert.equal(review.ok, true);
  assert.equal(structured.ok, true);
  assert.equal(confirmations, 1);
  assert.deepEqual(invocations, [
    'review:"change"',
    'structured:{"value":"ok"}',
  ]);
  assert.deepEqual(
    tool.parameters.properties["name"]?.enum,
    ["review", "structured"],
  );
  assert.match(
    tool.parameters.properties["input"]?.description ?? "",
    /structured:.*"required":\["value"\]/,
  );
  assert.match(
    buildWorkflowSystemContext(records),
    /structured input matching.*"required":\["value"\]/,
  );
});

test("uses the current workflow policy when executing a cached tool", async () => {
  const original = createRecord("deploy", "automatic");
  const current = createRecord("deploy", "confirm");
  let confirmations = 0;
  const tool = createRunWorkflowTool([original], {
    async resolve() {
      return current;
    },
    async invoke() {
      return "completed";
    },
    async confirm() {
      confirmations += 1;
      return true;
    },
  });

  const result = await tool.execute(
    { name: "deploy", inputText: "production" },
    {
      cwd: process.cwd(),
      requestPermission: async () => true,
      secrets: { get: () => undefined, has: () => false },
    },
  );

  assert.equal(result.ok, true);
  assert.equal(confirmations, 1);
});
