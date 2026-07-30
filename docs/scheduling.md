# Workflow scheduling

The Rust scheduling core targets owned workflows rather than arbitrary
prompts. It includes schedule validation and reauthorization services, SQLite
repositories, renewable worker leases, occurrence recovery, and a runnable
worker adapter.

## Service model

`ScheduleService` accepts:

- owning agent;
- workflow name;
- schema-validated JSON input;
- five-field cron expression;
- IANA timezone, defaulting to the captured local timezone.

Lists, ranges, steps, and wildcards are supported. Day-of-week accepts `0` or
`7` for Sunday. When day-of-month and day-of-week are both restricted, either
may match. Nonexistent local times are skipped and repeated local times can
occur twice.

Creation captures the project directory and authorization fingerprint.
Reauthorization computes and displays a prospective fingerprint, then commits
that exact approved value with optimistic concurrency.

The current executable does not register schedule creation or reauthorization
as CLI/model tools. Existing records can be listed, inspected, paused, resumed,
or deleted:

```text
/schedules
/schedule <id>
/schedule pause <id>
/schedule resume <id>
/schedule delete <id>
```

The application service is available to future adapters without moving
scheduling logic into the UI.

## Running the worker

Start the Rust worker explicitly:

```sh
flowmation worker --once
flowmation worker
flowmation worker --database /absolute/path/to/runs.sqlite
```

`--once` performs one tick. The long-running form polls every 15 seconds until
Ctrl+C. The interactive CLI does not detach it or install an operating-system
boot service; use a supervisor for unattended operation.

Each tick obtains a 45-second renewable SQLite lease and:

1. recovers non-terminal occurrences from an earlier process;
2. finds active schedules whose `next_run_at` is due;
3. creates a unique `(schedule_id, scheduled_for)` occurrence;
4. advances `next_run_at`;
5. verifies the authorized source fingerprint before loading the module;
6. links the run ID before durable workflow execution;
7. stores the result and terminal/waiting occurrence state.

Downtime coalesces missed times into one catch-up occurrence retaining the
oldest pending `scheduled_for` value. If a schedule already has a non-terminal
occurrence, repository claiming prevents a duplicate active execution.

The worker resolves main workflows from project then global top-level workflow
directories. Specialist schedules resolve the project then global agent
package and load from its `workflows/` directory. Specialist package
fingerprints cover the whole package.

## Source changes and human input

A fingerprint mismatch invalidates the occurrence and moves the schedule to
`needs-reauthorization` before host/module evaluation. There is still a small
hash/import time-of-check/time-of-use window; do not edit an authorized source
concurrently with execution.

The worker is non-interactive. A human callback makes the run and occurrence
`waiting` and creates an unread SQLite notification. A main/top-level
workflow can be continued with `/resume <run-id>` in a foreground CLI scoped
to the same project and agent.

Interactive agent-package-local workflow discovery is not implemented, so the
current CLI cannot resume a specialist-owned run whose source exists only
under `agents/<name>/workflows/`.

Schedules, occurrences, workflow runs, leases, results, and notifications all
use the WAL-backed `~/.work-agent/runs.sqlite` database. Notifications are
persisted, but the current CLI does not summarize or mark them read.
