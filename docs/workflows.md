# Workflow SDK reference

Workflow files import the SDK from `flowmation/workflow`:

```ts
import { defineWorkflow } from "flowmation/workflow";
```

The SDK exports `defineWorkflow`, `isWorkflowOutput`, `workflowOutputApi`, and
all of the types listed below.

## API at a glance

| Function | Used for |
| --- | --- |
| `defineWorkflow(definition)` | Defines and type-checks the workflow object that the workflow file default-exports. |
| `isWorkflowOutput(result)` | Checks whether a value was created by an output presentation helper. |
| `workflowOutputApi.direct(value)` | Wraps a JSON value so Flowmation prints it directly. |
| `workflowOutputApi.agent(value)` | Wraps a JSON value so the main agent presents it to the user. |
| `definition.run(context, input)` | Implements the workflow entry point that Flowmation calls with the execution context and validated input. |
| `context.output.direct(value)` | Selects direct CLI presentation for the current workflow result. |
| `context.output.agent(value)` | Selects main-agent presentation for the current workflow result. |
| `context.agents.create(options)` | Creates a new agent session with empty conversation history. |
| `context.agents.fork(session, options?)` | Creates a separate agent session by copying an existing session's history. |
| `session.run(prompt, options?)` | Sends a prompt to an agent session and returns its text response and model. |
| `context.human.approve(request)` | Asks the user a yes-or-no question and returns a boolean. |
| `context.human.choose(request)` | Asks the user to select from supplied choices and returns the selected value. |
| `context.human.ask(request)` | Asks the user for free-form text and returns the answer. |
| `context.checkpoint(key, operation)` | Runs an operation once per workflow run and reuses its stored JSON result after resume. |
| `context.effect(key, options)` | Runs an idempotent external action and reuses its stored JSON result after completion. |
| `effectOptions.run(effectContext)` | Implements the external action passed to `context.effect`, receiving its idempotency key and cancellation signal. |
| `context.exec(command, args?, options?)` | Runs a cancellation-aware child process and captures its exit code and output. |
| `context.map(items, options)` | Processes items with bounded parallelism while preserving result order. |
| `mapOptions.run(item, index)` | Implements the per-item asynchronous work passed to `context.map`. |
| `context.elevate(options)` | Repeats a model-backed operation until its check passes or its fallback handles failure. |
| `elevationOptions.operation(attempt)` | Produces one candidate result using the selected agent session and prior failed results. |
| `elevationOptions.check(value)` | Decides whether an elevation result passes and may attach a message or JSON diagnostic data. |
| `elevationOptions.fallback(failure)` | Optionally produces a final value after every elevation attempt fails its check. |
| `context.log(message, data?)` | Emits a workflow debug event visible when workflow debug logging is enabled. |
| `humanAdapter.request(prompt)` | Implements the runtime integration that collects a typed human response or allows the run to wait. |

## `defineWorkflow`

```ts
defineWorkflow<TInput = string, TOutput = JsonValue>(
  definition: WorkflowDefinition<TInput, TOutput>,
): WorkflowDefinition<TInput, TOutput>
```

This is a typed identity function. It returns the definition unchanged. A
workflow must default-export its result:

```ts
export default defineWorkflow({
  name: "example",
  description: "An example workflow",
  async run(context, input) {
    return { input };
  },
});
```

## `WorkflowDefinition`

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

`name`, the workflow directory name, and the command alias must match. Names
must be lowercase kebab-case. `presentation` defaults to `direct`.

`agentInvocation` defaults to `disabled`:

| Value | Behavior |
| --- | --- |
| `disabled` | The main agent cannot call the workflow. |
| `confirm` | The main agent can propose it, but the user must approve each run. |
| `automatic` | The main agent can run it without workflow-specific approval. |

## Input schemas

`input.schema` determines the type passed to `run` and validates it before the
run starts. Without `input`, the input is the command remainder as a string.
The root schema must be a `string` or `object` schema.

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

For an object schema, the command input must be JSON:

```text
/workflow example {"name":"Ada"}
```

## `WorkflowContext`

Every `run` receives this object:

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
  map<TItem, TResult>(
    items: readonly TItem[],
    options: WorkflowMapOptions<TItem, TResult>,
  ): Promise<TResult[]>;
  elevate<TValue extends JsonValue, TFallback extends JsonValue = TValue>(
    options: ElevationOptions<TValue, TFallback>,
  ): Promise<TValue | TFallback>;
  log(message: string, data?: JsonValue): Promise<void>;
}
```

### Context properties

| Property | Meaning |
| --- | --- |
| `runId` | Unique ID of the current run. |
| `projectDir` | Absolute launch/project directory. `exec` uses it by default. |
| `signal` | Aborted when the run is cancelled. Pass it to cancellable external APIs. |
| `output` | Helpers to choose how the completed result is presented. |
| `agents` | Create and fork agent sessions. |
| `human` | Ask the user for approval, a choice, or text. |

### `context.output`

```ts
interface WorkflowOutputApi {
  direct<TValue extends JsonValue>(value: TValue): WorkflowOutputValue<TValue>;
  agent<TValue extends JsonValue>(value: TValue): WorkflowOutputValue<TValue>;
}
```

Both helpers return the value unchanged and attach a presentation mode for this
run. `direct` prints JSON to the CLI. `agent` asks the main agent to present
the value. A normal return uses the definition's `presentation`.

### `context.agents`

```ts
interface WorkflowAgentsApi {
  create(options: { model: string }): Promise<WorkflowAgentSession>;
  fork(
    session: WorkflowAgentSession,
    options?: { model?: string },
  ): Promise<WorkflowAgentSession>;
}

interface WorkflowAgentSession {
  readonly id: string;
  readonly model: ModelRef;
  run(
    prompt: string,
    options?: { tools?: "default" | "none" },
  ): Promise<{ content: string; model: ModelRef }>;
}

interface ModelRef {
  provider: string;
  model: string;
  active: boolean;
}
```

`create` starts an empty session. Repeated `run` calls share that session's
history. `fork` copies the history and creates a separate session. `model`
may be a configured alias, `provider/model`, or a unique model name.

### `context.human`

```ts
interface WorkflowHumanApi {
  approve(request: {
    prompt: string;
    details?: string;
  }): Promise<boolean>;
  choose(request: {
    prompt: string;
    choices: HumanChoice[];
  }): Promise<string>;
  ask(request: {
    prompt: string;
    description?: string;
  }): Promise<string>;
}

interface HumanChoice {
  value: string;
  label: string;
  description?: string;
}
```

`approve` returns `true` for approval. `choose` returns one of the supplied
choice values. `ask` returns free-form text. Prompts are saved as run steps;
resuming a run reuses an already answered prompt.

If no human adapter is available, the run becomes `waiting` and can be resumed
through the CLI.

### `context.checkpoint`

```ts
checkpoint<TValue extends JsonValue>(
  key: string,
  operation: () => Promise<TValue>,
): Promise<TValue>
```

Runs `operation` once per run and stores its JSON result. A completed step with
the same key is returned on resume without calling `operation` again. Keys may
contain letters, numbers, `.`, `_`, and `-`, and must start with a letter or
number. Reusing a key with a different step kind or input fails.

### `context.effect`

```ts
interface WorkflowEffectOptions<TValue extends JsonValue> {
  idempotencyKey: string;
  run(context: {
    readonly idempotencyKey: string;
    readonly signal: AbortSignal;
  }): Promise<TValue>;
}

effect<TValue extends JsonValue>(
  key: string,
  options: WorkflowEffectOptions<TValue>,
): Promise<TValue>
```

Like a checkpoint, `effect` stores a completed JSON result and reuses it on
resume. If the process stops while `run` is executing, it may execute again;
the external system must use `idempotencyKey` to make the action safe to retry.

### `context.exec`

```ts
interface WorkflowExecOptions {
  cwd?: string;
  env?: Record<string, string>;
  input?: string;
  timeoutMs?: number;
  maxOutputBytes?: number;
  allowFailure?: boolean;
}

interface WorkflowExecResult {
  command: string;
  args: string[];
  stdout: string;
  stderr: string;
  exitCode: number;
}
```

`exec` runs a child process with `projectDir` as the default working
directory. The default output limit is 5 MiB. A non-zero exit code rejects the
promise unless `allowFailure` is `true`. `timeoutMs` and `maxOutputBytes` must
be positive integers. Cancellation stops the process.

### `context.map`

```ts
interface WorkflowMapOptions<TItem, TResult> {
  concurrency?: number;
  run(item: TItem, index: number): Promise<TResult>;
}

map<TItem, TResult>(
  items: readonly TItem[],
  options: WorkflowMapOptions<TItem, TResult>,
): Promise<TResult[]>
```

Runs up to `concurrency` callbacks at once. The default is `4`. Results keep
the input order. A callback error rejects the map; cancellation stops further
work.

### `context.elevate`

```ts
type ElevationContext =
  | { mode: "fresh" }
  | { mode: "reuse"; session: WorkflowAgentSession }
  | { mode: "fork"; session: WorkflowAgentSession };

interface ElevationOptions<TValue extends JsonValue, TFallback extends JsonValue = TValue> {
  model: string;
  attempts: number;
  context: ElevationContext;
  operation(attempt: {
    attempt: number;
    previousResults: TValue[];
    session: WorkflowAgentSession;
  }): Promise<TValue>;
  check(value: TValue): boolean | CheckDetails | Promise<boolean | CheckDetails>;
  fallback?(failure: {
    results: TValue[];
    checks: CheckDetails[];
    session: WorkflowAgentSession;
  }): Promise<TFallback>;
}

interface CheckDetails {
  passed: boolean;
  message?: string;
  data?: JsonValue;
}
```

`attempts` must be a positive integer. The operation runs up to that many
times. `check` may return a boolean or a `CheckDetails` object; `passed`
controls success. On success, `elevate` returns the checked value. After all
attempts fail, it calls `fallback` if supplied; otherwise it rejects.

The session mode controls history: `fresh` creates a new session, `reuse`
retargets the supplied session to `model`, and `fork` copies the supplied
session history into a new session using `model`.

### `context.log`

```ts
log(message: string, data?: JsonValue): Promise<void>
```

Emits a debug event. It is shown only while `/workflow-debug on` is enabled.

## JSON values

Workflow inputs, outputs, checkpoint values, effect values, human responses,
and log data must be JSON values:

```ts
type JsonValue =
  | string | number | boolean | null
  | JsonValue[]
  | { [key: string]: JsonValue };
```

Numbers must be finite. Objects must be plain objects, and circular values are
rejected.

## Run commands

```text
/workflows
/workflow <name> [input]
/<workflow-name> [input]
/runs
/run <id>
/resume <id>
/cancel <id>
/workflow-debug on|off
```

Runs are stored in `~/.work-agent/runs.sqlite` and scoped to the project
directory. Workflow files are fingerprinted when a run starts. If the files
change before resume, the run becomes `version-mismatch` and must be started
again.
