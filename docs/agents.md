# Configured agents

Configured agents are named, isolated packages. The main agent coordinates
work, while each specialist owns its prompt, model defaults, tool policy,
skills, workflows, context, persistent direct conversation, and scheduled
workflow executions.

## Package layout

Create a package in either location:

```text
~/.work-agent/agents/finance/
<project>/.work-agent/agents/finance/
```

The directory is atomic. If both locations define `finance`, the project
directory replaces the global directory completely. Files and permissions are
never merged across packages.

```text
finance/
  AGENT.yaml
  SOUL.md
  AGENTS.md
  CONTEXT.md
  context/
    accounting-policy.md
  skills/
    reconcile-transactions/
      SKILL.md
  workflows/
    monthly-close/
      WORKFLOW.ts
```

`AGENT.yaml`, `SOUL.md`, and `AGENTS.md` are required. `CONTEXT.md`,
`context/`, `skills/`, and `workflows/` are optional. The complete directory is
SHA-256 fingerprinted in deterministic path order.

## Manifest version 1

```yaml
version: 1
name: finance
description: Manages budgeting, reporting, reconciliation, and finance operations
model: finance-model
thinking: medium
tools:
  - read_file
  - write_file
  - run_command
```

`version`, `name`, and `description` are required. The name must match its
lowercase kebab-case directory. `model` accepts a provider-qualified model, an
unambiguous name, or an existing alias; omission inherits the main default.
`thinking` accepts `default`, `off`, `on`, `low`, `medium`, or `high`.

`tools` is an allowlist. When omitted it defaults to `read_file` and
`load_skill`. Direct specialist chat also receives schedule-management tools;
delegated, workflow, and scheduled sessions do not.

`SOUL.md` defines persona, priorities, and judgment. `AGENTS.md` defines
operational rules. `CONTEXT.md` is placed in the initial prompt, while files
under `context/` are listed by path and read only when needed.

## Resources

Agent-local skills and workflows are discovered under their respective
directories. Canonical IDs are `<agent>/<resource>`, such as
`finance/reconcile-transactions` and `finance/monthly-close`.

Inside direct finance chat, `/reconcile-transactions` uses the finance skill.
From main, `/finance/reconcile-transactions` loads it. A short name is accepted
from main only when globally unambiguous. No package inheritance,
cross-agent resource reference, or directory merge exists in v1.

Agent-local skills must be self-contained. The coordinator can load their
bodies without loading the specialist's persona or context.

## Conversations and delegation

`/agent` lists agents and marks the active identity. `/agent finance` switches
to finance and changes the prompt to `[finance] >`; `/agent main` returns to
the coordinator.

Each direct conversation is stored in `runs.sqlite` under the launch project
directory and agent name. `/clear` and `/model` affect only the active
conversation. Stored transcripts exclude system messages, so Flowmation
rebuilds the current package prompt on startup while preserving
user/assistant/tool history. Oversized history is compacted before a model
call.

The coordinator's `list_agents` tool discovers specialists.
`delegate_agent({ agent, task })` creates a fresh isolated specialist session,
passes only the explicit task and package context, and returns only its final
result. It does not consume or change either direct-chat history. Specialists
cannot recursively delegate in v1.

Specialist workflows use that specialist's prompt, model, thinking default,
skills, and tool policy. A workflow agent session inherits the specialist's
active model when `context.agents.create({})` omits `model`.
