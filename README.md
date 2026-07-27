# Flowmation

Flowmation is a terminal-based coding agent with programmable developer
workflows. It connects to Ollama-compatible model providers and lets normal
JavaScript or TypeScript coordinate agents, commands, model escalation,
bounded parallel work, human decisions, named specialists, and durable
workflow schedules.

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

## Configured agents

Named agents are self-contained packages under
`~/.work-agent/agents/<name>` or
`<project>/.work-agent/agents/<name>`. A project package atomically replaces a
global package with the same name; package directories never merge.

```text
agents/finance/
  AGENT.yaml
  SOUL.md
  AGENTS.md
  CONTEXT.md
  context/
  skills/
  workflows/
```

Use `/agent` to list packages, `/agent finance` to enter a specialist's
persistent project-scoped conversation, and `/agent main` to return to the
coordinator. Coordinator delegation uses a fresh isolated specialist session
and never reads or changes that specialist's direct-chat history.

Specialist resources use canonical IDs such as
`finance/reconcile-transactions`; their bodies remain lazy-loaded. See
[docs/agents.md](docs/agents.md) for the package and runtime model.

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

## Scheduling

Schedules run configured workflows, never arbitrary prompts. They capture the
owning agent, validated input, project working directory, five-field cron
expression, IANA timezone, and complete package fingerprint. Creation and
reauthorization are the approval boundaries for unattended execution.
The detached worker checks that fingerprint before loading workflow code.

The detached worker survives CLI exit, but v1 does not install an OS boot
service: launch Flowmation once after a reboot. Missed occurrences coalesce
into one catch-up run, and later occurrences skip while an earlier run remains
non-terminal.

See [docs/scheduling.md](docs/scheduling.md),
[docs/security.md](docs/security.md), and
[docs/migration.md](docs/migration.md).

## Commands

- `/help` shows command help.
- `/agent [name]` lists agents or switches the active conversation.
- `/clear` clears only the active conversation.
- `/model` lists, configures, or switches the active conversation's model.
- `/workflows` lists workflows owned by the active agent.
- `/workflow <name> [input]` runs an active-agent workflow in the foreground.
- `/<workflow-name> [input]` invokes a workflow directly.
- `/runs` lists recent project runs.
- `/workflow-debug on|off` toggles live workflow and sub-agent status logging.
- `/run <id>` inspects a run and presents completed output.
- `/resume <id>` resumes a waiting, interrupted, or stale running run.
- `/cancel <id>` cancels a run.
- `/schedules` lists project schedules.
- `/schedule <id>` inspects a schedule and its occurrences.
- `/schedule pause|resume|delete|reauthorize <id>` manages a schedule.
- `/<skill-name>` loads an active-agent skill.
- `/<agent>/<skill>` loads a specialist skill from main.
- `/exit` or `/quit` exits Flowmation.

Use the Up and Down arrow keys at the main prompt to browse earlier input. The
most recent 500 entries persist across sessions in
`~/.work-agent/input-history.json`. If you start typing before browsing,
returning to the newest position restores that draft. Setup answers and
workflow approval responses are not included.

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
