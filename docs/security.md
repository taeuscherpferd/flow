# Authority and security

Flowmation separates trusted workflow code from model-requested tool calls.

## Tool effects

Tools declare one of five effect classes:

- `read`: local inspection with no mutation; runs automatically.
- `write`: filesystem mutation; requires foreground approval.
- `command`: process execution; requires foreground approval.
- `external`: another effectful action; requires foreground approval.
- `schedule`: schedule creation or mutation; requires foreground approval.

The active agent's allowlist is applied before tools are shown to the model.
An unlisted tool cannot run. Delegated effects pause foreground delegation and
use the same approval prompt as direct chat.

The `run_workflow` tool manages permission through each workflow's
`agentInvocation` policy. `confirm` workflows prompt once, while `automatic`
workflows use their configured authorization without an additional generic
tool prompt.

Scheduled agent sessions have no interactive authority. Read-only calls may
run, but every effectful model tool call is denied. Schedule-management tools
are registered only in direct coordinator and specialist conversations.
Delegated and scheduled specialists cannot recursively delegate.

## Trusted workflows

`WORKFLOW.ts` and `WORKFLOW.js` are trusted local code. They can use Node APIs,
execute commands, and perform effects. Creating or reauthorizing a schedule is
therefore the approval boundary for unattended workflow execution. The
confirmation includes:

- agent and owned workflow;
- validated JSON input;
- project working directory;
- IANA timezone and cron cadence;
- complete agent-package fingerprint.

Changing any specialist package file invalidates its schedules before another
occurrence executes. Main schedules use the owned workflow fingerprint.
Reauthorization validates current input and captures the new fingerprint.
The worker verifies this fingerprint before evaluating the workflow module.
Symbolic links are rejected inside fingerprinted agent packages and workflow
directories so linked content cannot change outside the authorization record.

Workflow effects should still use `context.effect` with an external
idempotency key. A crash can occur after an external system accepts an action
but before the local completion record commits.

## Human input

A scheduled workflow cannot open an interactive prompt. If it calls
`context.human`, the durable run enters `waiting`, its occurrence remains
non-terminal, and Flowmation records an unread notification. Resume it with
`/resume <run-id>` in the foreground. Stored human steps and the schedule
occurrence update as the run continues.
