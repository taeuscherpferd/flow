---
name: create-schedule
description: Create and durably save one-shot or recurring cron schedules for existing Flowmation workflows through the create_schedule tool. Use when a user wants a workflow to run once at an exact future timestamp, repeat on a five-field cron cadence, inspect the proposed timing and input, or learn how to keep the Flowmation worker running.
---

# Create Schedule

Create a schedule only for an existing workflow listed by Flowmation. Scheduling stores the owning agent, workflow input, and authorization fingerprint; it does not store an arbitrary future chat prompt.

## Gather the exact request

1. Identify the owning agent, workflow name, and whether its input is plain text or an object. Omit `agent` to target the active agent. The coordinator can explicitly target a specialist; specialists can target themselves when their tool policy permits schedule creation.
2. For a one-shot schedule, require an exact future RFC 3339 timestamp containing `Z` or a numeric offset, such as `2026-08-09T09:00:00-06:00`.
3. For a recurring schedule, require a five-field cron expression and an IANA timezone such as `America/Denver`. Omit the timezone only when the user's local timezone is intended.
4. Do not infer a missing timestamp, timezone, workflow, or consequential input.

## Create the schedule

When the required values are available, call `create_schedule`; do not stop at
instructions or a command example. Use exactly one timing mode:

- One shot: optional `agent`, `name`, `at`, and either `input` or `inputText`.
- Recurring: optional `agent`, `name`, `cron`, optional `timezone`, and either `input` or `inputText`.

Use `input` only for object-schema workflows. Use `inputText` for string workflows. Never provide both `at` and `cron`, or both input forms.

Flowmation shows the exact workflow, input, working directory, timing, and package fingerprint for approval before saving the schedule. If the user declines, report that no schedule was created. Do not claim success until the tool returns the durable schedule record and its schedule ID.

## Direct command alternative

Use `/schedule create <json>` when the user wants an explicit command:

```text
/schedule create {"name":"remove-temporary-change","input":{"commit":"abc123"},"at":"2026-08-09T09:00:00-06:00"}
```

```text
/schedule create {"name":"weekly-report","inputText":"engineering","cron":"0 9 * * 1","timezone":"America/Denver"}
```

From the coordinator, include `"agent":"finance"` to schedule a workflow from the finance package.

## Explain execution behavior

- The separate `flowmation worker` process must be running. It polls every 15 seconds.
- A missed one-shot time runs once on the worker's next tick.
- A one-shot schedule is exhausted when its occurrence is claimed and is not automatically retried after failure.
- Cron schedules continue computing their next occurrence and coalesce downtime into one catch-up run.
- Source changes require schedule reauthorization before execution.
- Use `/schedules` to list schedules and `/schedule <id>` to inspect one. Use `/schedule pause|resume|delete <id>` to manage recurring schedules.
