# Migration notes

Configured agents and scheduling use additive SQLite migrations. Existing
`runs.sqlite` workflow data remains in place.

On startup Flowmation:

- adds `agent_name` and `trigger_json` to existing `workflow_runs`, defaulting
  historical runs to main/manual;
- creates agent conversation, schedule, occurrence, lease, and notification
  tables when missing;
- keeps existing workflow-step and source-fingerprint recovery behavior;
- installs a trigger that mirrors scheduled run status and output into its
  occurrence.

No manual migration command is required. WAL mode and foreign-key enforcement
remain enabled. Back up `~/.work-agent/runs.sqlite` before downgrading to a
version that predates these tables.

Conversation system messages are intentionally not retained. Flowmation stores
only user, assistant, and tool history and reconstructs current system prompts
from `SOUL.md`, `AGENTS.md`, context indexes, resource metadata, and tool
policy. This prevents stale package instructions surviving configuration
changes.

Existing top-level skills and workflows continue to belong to `main`.
Configured agent packages are opt-in and do not change main-agent behavior
until added under an `agents/<name>` directory.
