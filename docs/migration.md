# Migration notes

The Rust port opens the existing `~/.work-agent/runs.sqlite` in place and uses
five ordered, additive migrations:

| Version | Name | Change |
| ---: | --- | --- |
| 1 | `workflow storage` | Creates legacy-compatible `workflow_runs`, `workflow_steps`, and the project/update index when absent. |
| 2 | `workflow agent and trigger metadata` | Adds `agent_name` and `trigger_json` when absent. Historical rows receive `main` and a manual trigger through SQLite defaults. |
| 3 | `schedule storage` | Creates schedules, unique occurrences, worker leases, notifications, and the due index. |
| 4 | `schedule run status trigger` | Mirrors scheduled run status/output/error into occurrences and creates terminal/waiting notifications. |
| 5 | `agent conversation storage` | Creates project/agent-scoped direct conversation storage. |

Each migration runs in an immediate transaction and is recorded in
`flowmation_migrations`. A database with a migration version newer than this
binary supports is rejected.

Rust retains the 5-second busy timeout, WAL journal mode, foreign-key
enforcement, legacy table/column names, status strings, timestamp shape, JSON
text formats, indexes, and schedule trigger behavior. The compatibility tests
open a TypeScript-era fixture and verify that existing run and step data is not
rewritten.

No manual migration command is required; every repository open verifies and
applies the sequence. Back up `runs.sqlite` before migration or before
downgrading to a version that predates these objects.

Conversation system messages are intentionally not retained. Flowmation stores
user, assistant, and tool history and rebuilds current prompts from
`SOUL.md`, `AGENTS.md`, context indexes, resources, skills, workflows, and the
tool policy. Existing top-level skills and workflows continue to belong to
`main`; configured agent packages remain opt-in.

The versioned `input-history.json` format and its legacy bakery-lock filenames
are also preserved. Rust reclaims abandoned `.choosing` and `.ticket` entries
and keeps concurrent appends from separate processes.
