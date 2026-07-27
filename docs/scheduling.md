# Workflow scheduling

Flowmation v1 schedules owned workflows only. Arbitrary prompts are not
schedule targets.

## Creating a schedule

The `create_schedule` agent tool accepts:

- `agent`: owning agent, defaulting to the active direct conversation;
- `workflow`: a workflow owned by that agent;
- `input`: JSON validated against the workflow schema;
- `cron`: five fields: minute, hour, day-of-month, month, day-of-week;
- `timezone`: an IANA timezone, defaulting to the captured local timezone.

Lists, ranges, steps, and wildcards are supported. Day-of-week accepts `0` or
`7` for Sunday. When day-of-month and day-of-week are both restricted, either
may match, following traditional cron behavior. Matching uses the captured
timezone across daylight-saving transitions. A nonexistent local time is
skipped; a repeated local time can occur twice.

Creation requires foreground confirmation of the complete authorization
record. Use `/schedules` to list records and `/schedule <id>` to inspect a
record plus retained occurrences.

```text
/schedule pause <id>
/schedule resume <id>
/schedule delete <id>
/schedule reauthorize <id>
```

Reauthorization is required after any specialist package file changes and
prompts for approval again.

## Worker behavior

Flowmation launches a detached Node worker on CLI startup and schedule
creation. The worker survives normal CLI exit. V1 does not install an OS boot
service, so launch Flowmation once after a machine reboot.

The worker uses a renewable SQLite lease and a unique
`(schedule_id, scheduled_for)` occurrence. These prevent duplicate execution
across workers and restarts. The workflow run ID is attached before execution
starts, allowing crash recovery to resume the durable run.

Operational defaults in v1 are deliberate:

- downtime coalesces all missed times into one catch-up occurrence;
- catch-up retains the oldest pending `scheduledFor` time;
- if an earlier occurrence is running or waiting, a later due occurrence is
  recorded as `skipped`;
- completed, failed, waiting, skipped, and invalidated occurrences remain
  inspectable;
- waiting workflows resume through the normal `/resume` flow;
- compact unread completion/failure/waiting/invalidation counts appear on CLI
  startup.

Schedules, occurrences, workflow runs, leases, results, and notifications use
the existing WAL-backed `~/.work-agent/runs.sqlite` database.

To resume a specialist-owned waiting run, switch to its owner with
`/agent <name>` before running `/resume <run-id>`.
