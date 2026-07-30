# Workflow SDK and host boundary

Flowmation preserves user-authored JavaScript and TypeScript workflows instead
of translating them to Rust. Workflow files are trusted Node.js modules. Rust
owns discovery, validation, durable records, agent and process callbacks,
cancellation, human input, and final run status.

Node.js 24 or newer is required for every workflow discovery or execution
path. The current CLI initializes discovery before its first normal agent turn
to register `run_workflow`, so interactive chat also requires the host even
when the eventual turn does not invoke a workflow.

## Build the host

```sh
pnpm install --frozen-lockfile
pnpm --dir workflow-host run build
pnpm --dir workflow-host test
```

The Rust CLI uses `workflow-host/dist/index.js` from the checkout by default.
Set an absolute path when packaging the executable separately:

```sh
FLOWMATION_WORKFLOW_HOST=/absolute/path/to/workflow-host/dist/index.js flowmation
```

## Discovery

The interactive CLI scans:

```text
~/.work-agent/workflows/
<project>/.work-agent/workflows/
```

Project records replace same-named global records. Interactive
`agents/<name>/workflows/` discovery is not implemented.

Each lowercase kebab-case directory must contain exactly one entry:

```text
review-change/
  WORKFLOW.js
```

or:

```text
review-change/
  WORKFLOW.ts
```

The default export's name must match the directory, its description must be
non-empty, and it must provide a `run` function. Symbolic entry paths are
rejected by the host, and Rust rejects symbolic links anywhere in the complete
fingerprinted directory.

Rust fingerprints the directory during discovery and verifies it before a new
or resumed execution. A mismatch rejects the run; a resume is persisted as
`version-mismatch`. There is necessarily a small time-of-check/time-of-import
window between the final hash and the Node module import. Workflow modules are
trusted local code, so do not mutate an authorized workflow concurrently with
execution.

## Definition

```ts
import { defineWorkflow } from "flowmation/workflow";

export default defineWorkflow({
  name: "hello",
  description: "Returns a greeting",
  input: {
    schema: {
      type: "object",
      properties: {
        name: { type: "string", minLength: 1 },
      },
      required: ["name"],
      additionalProperties: false,
    },
  },
  agentInvocation: "confirm",
  presentation: "direct",
  async run(context, input) {
    await context.log("creating greeting", input);
    return context.output.direct({ message: `Hello ${input.name}` });
  },
});
```

`defineWorkflow` is a typed identity function. The host also exports
`isWorkflowOutput`, `workflowOutputApi`, and the SDK types.

```ts
interface WorkflowDefinition<TInput = string, TOutput = JsonValue> {
  name: string;
  description: string;
  input?: { schema: WorkflowRootSchema };
  agentInvocation?: "disabled" | "confirm" | "automatic";
  presentation?: "direct" | "agent";
  run(
    context: WorkflowContext,
    input: TInput,
  ): Promise<TOutput | WorkflowOutputValue<TOutput>>;
}
```

`agentInvocation` defaults to `disabled`. After discovery, Rust registers the
model-facing `run_workflow` tool for agents whose allowlist includes it:

- `disabled` workflows are not eligible;
- `confirm` workflows show the description and exact input for approval;
- `automatic` workflows run without that workflow-specific confirmation.

The tool resolves the record again at execution, so a cached tool uses the
current workflow policy. Direct commands can run any discovered workflow.

`presentation` defaults to `direct`. `context.output.direct` prints the value;
`context.output.agent` presents the durable JSON through a fresh, tool-free
agent session without changing direct conversation history.

## Input schemas

Without `input`, Rust passes the command remainder as a string. With a string
root schema, it validates that string. With an object root schema, it parses
the remainder as JSON, using `{}` for an empty remainder, and validates it.

Supported schema nodes are:

```ts
type WorkflowSchema =
  | { type: "string"; description?: string; enum?: string[]; minLength?: number }
  | { type: "number"; description?: string; minimum?: number; maximum?: number }
  | { type: "boolean"; description?: string }
  | { type: "array"; description?: string; items: WorkflowSchema }
  | {
      type: "object";
      description?: string;
      properties: Record<string, WorkflowSchema>;
      required?: string[];
      additionalProperties?: boolean;
    };
```

Only `string` and `object` are valid roots. Nested object, array, number,
boolean, and string schemas are supported.

## Workflow context

```ts
interface WorkflowContext {
  readonly runId: string;
  readonly projectDir: string;
  readonly signal: AbortSignal;
  readonly output: WorkflowOutputApi;
  readonly agents: WorkflowAgentsApi;
  readonly human: WorkflowHumanApi;
  checkpoint<TValue extends JsonValue>(
    key: string,
    operation: () => Promise<TValue>,
  ): Promise<TValue>;
  effect<TValue extends JsonValue>(
    key: string,
    options: WorkflowEffectOptions<TValue>,
  ): Promise<TValue>;
  exec(
    command: string,
    args?: string[],
    options?: WorkflowExecOptions,
  ): Promise<WorkflowExecResult>;
  map<TItem extends JsonValue, TResult extends JsonValue>(
    items: readonly TItem[],
    options: WorkflowMapOptions<TItem, TResult>,
  ): Promise<TResult[]>;
  elevate<TValue extends JsonValue, TFallback extends JsonValue = TValue>(
    options: ElevationOptions<TValue, TFallback>,
  ): Promise<TValue | TFallback>;
  log(message: string, data?: JsonValue): Promise<void>;
}
```

Protocol values must be JSON values. Numbers must be finite, objects must be
plain objects, and symbol-keyed or circular values are rejected.

### Agent sessions

`context.agents.create` starts empty history. Repeated `run` calls share that
history. `fork` copies the current history into a distinct session. A model can
be provider-qualified, an unambiguous bare name, or an alias; omission inherits
the active agent model.

Per-run `tools: "default" | "none"` and all workflow thinking modes are
forwarded to Rust and are not sticky. The current implementation uses the
active agent's profile and tool registry rather than selecting a separate
package by workflow ownership.

### Checkpoints and effects

Rust stores each step's kind, stable input, state, and JSON output. Repeating a
completed key in the same durable run returns its stored output. Reusing a key
with a different kind or input fails. Keys may contain ASCII letters, numbers,
`.`, `_`, and `-`.

`/resume <run-id>` executes the stored input again through the current module.
Completed checkpoints and effects are reused, completed human answers are
replayed, and ordinary agent calls run again. An interrupted effect can run
again before its local completion commits, so external systems must honor the
provided idempotency key.

### Human requests

`context.human.approve`, `choose`, and `ask` are serialized before reaching the
terminal broker. A prompt failure does not poison the queue.

If the broker returns no response, Rust persists the incomplete human step and
marks the run `waiting`. `/resume` prompts again for that same occurrence and
stores the response; later replay reuses it.

### Map and elevation

`context.map` defaults to concurrency `4`, enforces a positive limit, and
restores input order after bounded parallel execution.

`context.elevate` supports positive attempt counts, checks, fallback, and
`fresh`, `fork`, or `reuse` session context. `reuse` retargets the supplied
session; `fork` copies it. Elevation-level thinking is scoped to operation
session runs, and an explicit per-run thinking option overrides it.

### Process execution

`context.exec` supports `cwd`, string-valued `env`, stdin `input`, positive
`timeoutMs`, positive `maxOutputBytes`, and `allowFailure`. Rust captures
stdout/stderr and returns command, arguments, output, and exit code. Failed
commands reject unless `allowFailure` is true.

Cancellation terminates the child and, on Unix, its process group so
descendants do not survive. The descendant-termination integration test is
Unix-only, matching the legacy Windows test exclusion; Windows descendant-tree
behavior is not covered by this suite.

### Logging

`context.log` sends a typed request to Rust. `/workflow-debug on` displays
foreground callback logs on stderr; `/workflow-debug off` hides them. The
setting is process-local.

## Durable run commands

Foreground execution:

1. verifies source and validates input;
2. inserts a queued manual run;
3. marks it running;
4. executes through the host and Rust callback services;
5. stores `completed`, `failed`, `waiting`, `cancelled`, or
   `version-mismatch`.

`/runs` lists the 20 most recently updated runs for the current project and
active agent. `/run <id>` inspects one in that scope. `/resume <id>` resumes
queued, running, waiting, or interrupted records after source verification.

`/cancel <id>` rejects terminal runs and changes a scoped non-terminal record
to `cancelled`. It does not have an inter-process control channel, so it cannot
interrupt a host or subprocess currently executing in another CLI/worker
process.

## Protocol version 1

The Rust client spawns `node <host-entry>` with piped stdin/stdout/stderr and
negotiates version `1` using `host.handshake`. Messages are newline-delimited
JSON-RPC 2.0.

Rust-to-host requests are `host.handshake`, `workflow.inspect`,
`workflow.run`, `workflow.cancel`, `callback.invoke`, and `host.shutdown`.
Host-to-Rust requests are `sdk.checkpoint`, `sdk.effect`, `sdk.exec`,
`sdk.map`, `sdk.agent.create`, `sdk.agent.fork`, `sdk.agent.run`, `sdk.human`,
`sdk.elevate`, and `sdk.log`.
