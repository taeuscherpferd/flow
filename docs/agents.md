# Configured agents

Configured agents are Rust-owned package records with isolated prompts, model
defaults, tool allowlists, skills, fingerprints, and persistent direct
conversations.

## Package layout

Create a package in either location:

```text
~/.work-agent/agents/finance/
<project>/.work-agent/agents/finance/
```

The directory is atomic. If both locations define `finance`, the project
directory replaces the global directory completely. If that project package
is invalid, Flowmation skips it instead of falling back to the global package.

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
```

`AGENT.yaml`, `SOUL.md`, and `AGENTS.md` are required. `CONTEXT.md`,
`context/`, and `skills/` are optional.

Rust fingerprints every regular file under the package. The legacy algorithm
sorts portable relative paths, then hashes each relative path, a NUL byte, the
file contents, and another NUL byte with SHA-256. Symbolic links and
non-file/non-directory entries are rejected.

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
  - load_skill
  - run_workflow
```

`version`, `name`, and `description` are required. `version` must be `1`, and
the name must match its lowercase kebab-case directory. `model` accepts a
provider-qualified model, an unambiguous bare model name, or an alias; omission
inherits the configured default model. `thinking` accepts `default`, `off`,
`on`, `low`, `medium`, or `high`.

When `tools` is omitted, it defaults to `read_file` and `load_skill`. The CLI
registers:

| Tool | Effect and behavior |
| --- | --- |
| `read_file` | Read effect; reads text relative to the launch project unless given an absolute path. |
| `write_file` | Write effect; creates parents and overwrites the target without a per-call prompt. |
| `run_command` | Command effect; runs without a per-call prompt, uses the platform shell, has a 30-second timeout, and retains at most 500 KB from each output stream. |
| `load_skill` | Read effect; returns the rendered body of an exact active-agent skill. |
| `run_workflow` | Self-managed external effect; exposes eligible top-level workflows and honors their `disabled`, `confirm`, or `automatic` policy. |

The manifest format also recognizes delegation and schedule tool names, but
the CLI does not register those implementations.

`SOUL.md` supplies persona and priorities. `AGENTS.md` supplies operational
instructions. `CONTEXT.md` is inserted into the system prompt. Files under
`context/` are listed by absolute path for on-demand access rather than eagerly
inserted.

## Skills

Agent-local skills are discovered under `skills/<name>/SKILL.md`. The
frontmatter name must be lowercase kebab-case and match its directory. Metadata
appears in the system prompt, while the body is loaded only through
`/<skill-name>` or `load_skill`.

The active specialist can load its own skills. Cross-agent skill lookup and
coordinator short-name resolution are not implemented.

All files in an agent package contribute to its authorization fingerprint.
The interactive workflow registry still scans only top-level global and
project `workflows/` directories; it does not discover
`agents/<name>/workflows/`. The schedule worker can resolve an already
authorized specialist schedule from the owning package's `workflows/`
directory.

## Conversations

`/agent` lists the coordinator and valid packages. `/agent finance` persists
the current conversation before switching, and `/agent main` returns to the
coordinator.

Each direct conversation is keyed by canonical launch project and agent name
in `~/.work-agent/runs.sqlite`. `/clear` and `/model` affect only the active
conversation.

Rust stores user, assistant, and tool messages but filters out system
messages. On load it rebuilds the prompt from the current soul, instructions,
context index, context-file list, agent/skill metadata, workflow metadata, and
tool allowlist before appending non-system history.

## Workflow sessions and delegation boundary

Workflow callbacks can create an empty agent session, run multiple turns with
shared history, fork a session by copying its history, and retarget a reused
session to another model. Per-run tool and thinking options do not become
sticky. Workflow result presentation uses a fresh, tool-free session and does
not overwrite the direct conversation.

These sessions use the active agent profile and tool registry. Coordinator
`list_agents`/`delegate_agent` tools, fresh specialist delegation, and
recursive-delegation rules are not current user-facing capabilities.
