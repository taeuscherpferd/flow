---
name: create-workflow
description: Create or update trusted Flowmation TypeScript or JavaScript workflows using the flowmation/workflow SDK. Use when a user asks to automate a repeatable multi-step process, add durable checkpoints or effects, coordinate agent sessions, request human input, schedule-compatible work, or improve an existing WORKFLOW.ts or WORKFLOW.js module.
---

# Create Workflow

Build a small trusted Node.js module around the workflow SDK. Workflows can execute local code and dependencies, so keep every operation within the user's requested scope.

## Understand the process

1. Identify the trigger, input, ordered steps, output, failure behavior, and whether the work changes external state.
2. Decide which steps need replay safety, human input, agent reasoning, bounded parallelism, or process execution.
3. Inspect an existing same-named workflow before editing it. Preserve its authorization policy and durability keys unless the user requests a behavioral change.

## Choose the location and policy

- Default to `<project>/.work-agent/workflows/<workflow-name>/WORKFLOW.ts`.
- Use `~/.work-agent/workflows/<workflow-name>/WORKFLOW.ts` only when the user explicitly wants a cross-project workflow.
- Prefer TypeScript. Use lowercase kebab-case, and make the directory and exported workflow names match.
- Create exactly one entry file: `WORKFLOW.ts` or `WORKFLOW.js`.
- Keep `agentInvocation: "disabled"` unless agents should invoke the workflow. Use `"confirm"` for user-approved calls. Use `"automatic"` only for explicitly authorized, low-risk automation.
- Use `presentation: "direct"` for structured results and `"agent"` when a fresh tool-free agent should explain the final JSON.

## Start with a typed definition

```ts
import { defineWorkflow } from "flowmation/workflow";

interface ExampleInput {
  topic: string;
}

interface ExampleOutput {
  summary: string;
}

export default defineWorkflow<ExampleInput, ExampleOutput>({
  name: "example-workflow",
  description: "Create a summary for a required topic",
  input: {
    schema: {
      type: "object",
      properties: {
        topic: { type: "string", minLength: 1 },
      },
      required: ["topic"],
      additionalProperties: false,
    },
  },
  agentInvocation: "disabled",
  presentation: "direct",
  async run(context, input) {
    await context.log("creating summary", { topic: input.topic });
    return context.output.direct({ summary: input.topic });
  },
});
```

Without `input`, the workflow receives a string. Schema roots may be `string` or `object`; nested values may also use `number`, `boolean`, `array`, or `object`. Keep inputs and outputs JSON-compatible and numbers finite.

## Select SDK operations deliberately

- Use `context.checkpoint(key, operation)` for deterministic expensive work whose JSON result can be reused on resume.
- Use `context.effect(key, { idempotencyKey, run })` for external changes. Pass the idempotency key to the external system because an interrupted effect may run again before completion is recorded.
- Use `context.human.approve`, `choose`, or `ask` when execution must pause for a person. A missing response leaves a resumable waiting run.
- Use `context.agents.create` for a fresh agent session, repeated `session.run` calls for shared context, and `context.agents.fork` for an isolated copy.
- Use `context.map` for bounded parallel work while preserving input order.
- Use `context.exec` for local processes with explicit arguments, working directory, timeout, output limit, and failure policy.
- Use `context.elevate` only when attempts, checks, and fallback behavior justify a stronger model.
- Use stable unique durability keys containing only letters, numbers, `.`, `_`, and `-`. Never reuse a key for a different operation or input.
- Honor `context.signal` in custom asynchronous work and keep secrets out of logs and durable outputs.

## Validate and finish

1. Read every changed file back and check imports, types, schema, exported name, policy, and stable keys.
2. Run the repository's relevant formatter, type checker, and tests when available.
3. Explain what the workflow may execute and call out any external effects or automatic agent invocation.
4. Tell the user to restart Flowmation, run `/workflows` to confirm discovery, then test with `/workflow <name> <input>`. Workflow discovery occurs when the application starts.
