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
- either an exact future RFC 3339 timestamp for a one-shot schedule;
- or a five-field cron expression and IANA timezone, defaulting to the
  captured local timezone.

Lists, ranges, steps, and wildcards are supported. Day-of-week accepts `0` or
`7` for Sunday. When day-of-month and day-of-week are both restricted, either
may match. Nonexistent local times are skipped and repeated local times can
occur twice.

Creation captures the project directory and authorization fingerprint.
Reauthorization computes and displays a prospective fingerprint, then commits
that exact approved value with optimistic concurrency.

The coordinator registers a self-managed `create_schedule` model tool and the
built-in `create-schedule` skill. A specialist receives both when its manifest
tool policy includes `create_schedule`. Both one-shot and cron schedules target
an existing agent-owned workflow; arbitrary future chat prompts are not stored.
The optional `agent` field defaults to the active agent. The coordinator can
target any discovered specialist, while a specialist can target itself.
The direct CLI form accepts the same fields as JSON:

```text
/schedule create {"name":"remove-temporary-change","input":{"commit":"abc123"},"at":"2026-08-09T09:00:00-06:00"}
/schedule create {"name":"weekly-report","inputText":"engineering","cron":"0 9 * * 1","timezone":"America/Denver"}
/schedule create {"agent":"finance","name":"weekly-report","inputText":"engineering","cron":"0 9 * * 1","timezone":"America/Denver"}
/schedules
/schedule <id>
/schedule pause <id>
/schedule resume <id>
/schedule delete <id>
```

Use exactly one of `at` or `cron`, and exactly one of `input` or `inputText`.
The timestamp must include `Z` or a numeric offset. Schedule creation shows the
agent, workflow, input, working directory, timing, and authorization
fingerprint, then requires confirmation before the record is written. Schedule
creation discovers workflows inside specialist packages and fingerprints the
complete owning package, matching worker verification. Reauthorization remains
available only as an application service.

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
4. advances a cron schedule's `next_run_at`, or marks a claimed one-shot
   schedule completed;
5. verifies the authorized source fingerprint before loading the module;
6. links the run ID before durable workflow execution;
7. stores the result and terminal/waiting occurrence state.

Downtime coalesces missed cron times into one catch-up occurrence retaining the
oldest pending `scheduled_for` value. A missed one-shot runs once on the next
worker tick. It is exhausted when claimed and is not automatically retried if
execution fails. If a schedule already has a non-terminal occurrence,
repository claiming prevents a duplicate active execution.

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
