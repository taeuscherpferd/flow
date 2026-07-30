# Authority and security

Flowmation separates model-requested tools from trusted workflow modules.

## Model tool effects

Tools declare one of five effect classes:

- `read`: local inspection; allowed automatically.
- `write`: filesystem mutation; requires foreground approval.
- `command`: process execution; requires foreground approval.
- `external`: another effectful action; requires foreground approval.
- `schedule`: schedule creation or mutation; requires foreground approval.

The active agent's allowlist is applied before tools are shown to the model.
An unlisted or unregistered tool cannot run. Concurrent terminal permission
prompts are serialized.

`run_workflow` is self-managed. `disabled` workflows are unavailable,
`confirm` workflows prompt with the exact input, and `automatic` workflows do
not add a workflow-specific prompt. The record and policy are resolved again
when the call executes.

The CLI does not currently register delegation or schedule-management model
tools. The scheduling application service still applies a separate complete
authorization record.

## Trusted workflow code

`WORKFLOW.ts` and `WORKFLOW.js` are trusted local code. The Node host preserves
Node APIs and workflow-local imports, so a module can perform actions outside
Rust tool approval. Review workflow code before running or scheduling it.

Rust rejects symbolic links in fingerprinted workflow and agent-package
directories. It verifies the stored SHA-256 fingerprint before new, resumed,
and scheduled execution. A schedule fingerprint covers the workflow directory
for `main` and the complete package for a specialist.

Fingerprinting and Node import are separate operations. A local actor can
modify bytes in the short interval after the final hash and before/during
module import. This is a time-of-check/time-of-use limitation, not a sandbox
boundary.

Workflow effects should use `context.effect` and an external idempotency key.
A crash can occur after an external system accepts an action but before the
local completion record commits.

## Scheduled authority

The Rust worker uses a non-interactive authorization policy: read-only model
tools are allowed and effectful model tools are denied. Workflow code itself
remains trusted.

Schedule creation and reauthorization services include:

- owning agent and workflow;
- validated input;
- project working directory;
- IANA timezone and cron cadence;
- complete authorization fingerprint.

The current CLI does not expose those creation/reauthorization services, so
existing records must have been created by a compatible adapter or prior
version. Fingerprint mismatch invalidates them before normal module evaluation.

## Cancellation and subprocesses

In-process workflow cancellation calls `workflow.cancel` and cancels Rust
callbacks. `context.exec` terminates its child and uses a process group on Unix
to terminate descendants. The descendant test is Unix-only; Windows
descendant-tree behavior is not covered.

`/cancel` is a durable database transition, not an inter-process signal. It
cannot stop a workflow host or command currently owned by another CLI or
worker process.

## Human input

Foreground human and permission prompts are serialized. A scheduled workflow
cannot prompt: an unanswered human callback persists a waiting run,
occurrence, and unread notification. `/resume` can continue a discoverable
workflow in a foreground process and reuse completed durable human responses.
