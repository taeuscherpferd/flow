import assert from "node:assert/strict";
import test from "node:test";
import { buildWorkflowConfirmationDetails } from "./WorkflowCliController.js";
import type { WorkflowRecord } from "../workflows/types.js";

const workflow: WorkflowRecord = {
  definition: {
    name: "deploy",
    description: "Deploys an application",
    agentInvocation: "confirm",
    async run() {
      return null;
    },
  },
  directory: "deploy",
  entryPath: "deploy/WORKFLOW.js",
  fingerprint: "deploy",
  source: "global",
};

test("includes the workflow input in agent-invocation confirmations", () => {
  const details = buildWorkflowConfirmationDetails(workflow, {
    environment: "production",
    version: 42,
  });

  assert.match(details, /Deploys an application/);
  assert.match(details, /"environment": "production"/);
  assert.match(details, /"version": 42/);
});
