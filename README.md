# Flowmation

Flowmation is a terminal-based coding agent with programmable developer
workflows. It connects to Ollama-compatible model providers and lets normal
JavaScript or TypeScript coordinate agents, commands, model escalation,
bounded parallel work, and human decisions.

## Getting started

```sh
pnpm install
pnpm dev
```

To install the built CLI as a global command on your machine:

```powershell
pnpm build
pnpm add --global .
```

After linking, run Flowmation from any directory with:

```powershell
flowmation
```

If the command is not found, run `pnpm setup`, restart your terminal, and run
`pnpm add --global .` again.

Flowmation requires Node.js 24 or newer. On first launch it creates
`~/.work-agent` and asks you to configure a provider with `/model`.

Configuration is loaded from both `~/.work-agent` and
`<launch-directory>/.work-agent`, with project values taking precedence.

## Workflows

For the full workflow authoring guide, see [docs/workflows.md](docs/workflows.md).

Put each workflow in a directory named after the workflow:

```text
~/.work-agent/workflows/review-change/WORKFLOW.ts
<project>/.work-agent/workflows/review-change/WORKFLOW.ts
```

`WORKFLOW.js` is also supported. A project workflow overrides a global
workflow with the same name. Directory and exported workflow names must match
and use lowercase kebab-case. Flowmation creates the ESM and TypeScript
configuration needed for the virtual `flowmation/workflow` import; workflow
authors only maintain `WORKFLOW.ts`.

```ts
import { defineWorkflow } from "flowmation/workflow";

interface ReviewInput {
  request: string;
}

export default defineWorkflow<ReviewInput>({
  name: "review-change",
  description: "Implements a change and independently reviews it",
  input: {
    schema: {
      type: "object",
      properties: {
        request: { type: "string", minLength: 1 },
      },
      required: ["request"],
      additionalProperties: false,
    },
  },
  agentInvocation: "confirm",
  presentation: "agent",
  async run(context, input) {
    const diff = await context.exec("git", ["diff"]);
    const implementation = await context.checkpoint("implementation", async () => {
      const implementer = await context.agents.create({
        model: "implementer",
      });
      return (await implementer.run(
        `${input.request}\n\nCurrent diff:\n${diff.stdout}`,
      )).content;
    });

    const result = await context.elevate({
      model: "reviewer",
      thinking: "high",
      attempts: 2,
      context: { mode: "fresh" },
      operation: async ({ session, attempt, previousResults }) => ({
        attempt,
        review: (
          await session.run(
            `Review this result:\n${implementation}\n` +
              `Previous rejected reviews: ${JSON.stringify(previousResults)}`,
          )
        ).content,
      }),
      check: ({ review }) => ({
        passed: review.includes("APPROVED"),
        message: "The reviewer must explicitly approve the result.",
      }),
      fallback: async ({ results }) => {
        const approved = await context.human.approve({
          prompt: "The automated review failed. Accept the latest result?",
          details: results.at(-1)?.review,
        });
        return { approved, review: results.at(-1)?.review ?? "" };
      },
    });

    return context.output.agent({
      implementation,
      review: result,
    });
  },
});
```

Omit `input` to receive the command remainder as a string. Object-schema
workflows receive validated JSON:

```text
/review-change {"request":"add workflow support"}
```

### Agent sessions

- `context.agents.create(...)` creates a clean session.
- Calling `run` repeatedly on the same session shares its history.
- `context.agents.fork(session, ...)` copies the current history and then
  diverges.
- Each `run` can select `thinking: "off"`, `"on"`, `"low"`, `"medium"`, or
  `"high"` when the model supports it. See [docs/workflows.md](docs/workflows.md)
  for the full per-turn behavior.

Model selectors accept `provider/model`, a unique bare model name, or an
unlimited user-defined alias:

```json
{
  "modelAliases": {
    "implementer": "local/qwen3:8b",
    "reviewer": "local/qwen3:32b"
  }
}
```

### Execution and recovery

Workflows normally execute from top to bottom. Runs, status, and final outputs
are stored in `~/.work-agent/runs.sqlite`, keyed by project directory.
Agent calls, commands, checks, and mapped work run normally and are not
automatically replayed or cached.

Recovery is explicit:

- `context.checkpoint(key, operation)` stores an expensive JSON result. A
  resumed run reuses it.
- `context.effect(key, { idempotencyKey, run })` stores a completed external
  action. If the process stops while the action is running, it may run again
  with the same idempotency key. The operation must pass that key to the
  external system or check whether the action already happened.
- Human responses are retained so resuming does not ask the same unchanged
  question twice.
- `context.exec(command, args, options)` runs a cancellation-aware process
  with time and output limits.
- `context.map(items, { concurrency, run })` performs bounded parallel work
  while preserving result order.

Run inspection, resume, and cancellation commands are scoped to the current
project directory.

Workflow source is fingerprinted when a run starts, including every file in
the workflow directory. A paused run cannot resume against changed source;
restore the original workflow or start a new run.

Workflow files are trusted local code and may use normal Node.js APIs.
Filesystem, network, process, time, randomness, and other direct side effects
are not cached unless the workflow deliberately wraps their JSON result in a
checkpoint.

## Commands

- `/help` shows command help.
- `/clear` clears the main conversation context.
- `/model` lists, configures, or switches models.
- `/workflows` lists discovered workflows.
- `/workflow <name> [input]` runs a workflow in the foreground.
- `/<workflow-name> [input]` invokes a workflow directly.
- `/runs` lists recent project runs.
- `/workflow-debug on|off` toggles live workflow and sub-agent status logging.
- `/run <id>` inspects a run and presents completed output.
- `/resume <id>` resumes a waiting, interrupted, or stale running run.
- `/cancel <id>` cancels a run.
- `/<skill-name>` loads a configured skill.
- `/exit` or `/quit` exits Flowmation.

Workflows may set `agentInvocation` to `disabled`, `confirm`, or `automatic`.
Eligible workflow descriptions are given to the main agent through its
`run_workflow` tool. Confirmed workflows show their proposed input and require
user approval before running.


## Development

```sh
pnpm test
pnpm run build
```

The CLI entry point is `src/index.ts`. Workflow discovery, storage, agent
coordination, and execution live under `src/workflows`. Internal modules use the
`#src/*` package import alias; relative module imports are reserved for generated
workflow fixtures that exercise workflow-local dependencies.
