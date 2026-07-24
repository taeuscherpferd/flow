# Design: workflow agent thinking controls

Status: proposed  
Scope: workflow-created agent sessions using the Ollama provider

## Summary

Add a provider-neutral `thinking` option to `WorkflowAgentSession.run()`. The
option applies to every model request made during that run, including requests
after tool results. The Ollama provider translates the normalized option to its
native top-level `think` request field.

The first version supports explicit on/off control and Ollama's three graded
levels:

```ts
type WorkflowThinking = "default" | "off" | "on" | "low" | "medium" | "high";
```

`default` preserves current behavior by omitting the provider field. Thinking
output is retained in internal conversation history so multi-turn and tool-call
requests can replay it, but it is not exposed in `WorkflowAgentResponse` in
this version.

## Goals

- Let workflow authors disable thinking for low-complexity, latency-sensitive
  agent calls.
- Let workflow authors request thinking or a supported reasoning level for
  complex calls.
- Keep workflow code independent of Ollama's wire format.
- Apply one setting consistently across an entire agent turn and its tool loop.
- Preserve reasoning content in session history.
- Keep all existing workflows behaviorally compatible.

## Non-goals

- Automatically choose a thinking level from the prompt.
- Detect model thinking capabilities before making a request.
- Add thinking controls to the main interactive agent.
- Display or return the model's reasoning trace to workflow authors.
- Add token-budget controls.
- Silently clamp unsupported levels.

## Public SDK

Add this exported type in `src/workflows/types.ts`:

```ts
export type WorkflowThinking =
  | "default"
  | "off"
  | "on"
  | "low"
  | "medium"
  | "high";
```

Extend the existing run options:

```ts
export interface WorkflowAgentRunOptions {
  tools?: "default" | "none";
  thinking?: WorkflowThinking;
}
```

Usage:

```ts
const agent = await context.agents.create({ model: "fast-model" });

const files = await agent.run("Classify these changed files.", {
  thinking: "off",
});

const review = await agent.run("Review the architectural consequences.", {
  thinking: "high",
});
```

The option belongs on `run()`, not `agents.create()`. A single session may
need inexpensive direct responses and deeper reasoning at different points,
and placing it on `run()` matches the existing per-turn `tools` option.

## Semantics

| SDK value | Provider-neutral meaning | Ollama `think` value |
| --- | --- | --- |
| omitted | Preserve current behavior | Field omitted |
| `"default"` | Preserve provider/model default | Field omitted |
| `"off"` | Disable thinking when supported | `false` |
| `"on"` | Enable thinking when supported | `true` |
| `"low"` | Request low reasoning effort | `"low"` |
| `"medium"` | Request medium reasoning effort | `"medium"` |
| `"high"` | Request high reasoning effort | `"high"` |

Ollama model support varies. Most thinking models use boolean control. GPT-OSS
uses graded levels and does not support disabling thinking. Flowmation should
send the requested value without guessing from the model name. If Ollama
rejects a value, `OllamaProvider` raises `OllamaProviderError` and the existing
`AgentComsService` behavior converts it to agent response content beginning
with `Error:`.

No fallback or clamping is allowed because silently changing reasoning behavior
would make workflows unpredictable.

## Internal provider contract

Add a provider-neutral type in `src/providers/types.ts`:

```ts
export type ThinkingMode =
  | "default"
  | "off"
  | "on"
  | "low"
  | "medium"
  | "high";

export interface ChatCompletionOptions {
  numCtx?: number;
  thinking?: ThinkingMode;
}

export interface ChatCompletionRequest {
  model: string;
  messages: ChatMessage[];
  tools?: ToolDef[];
  options?: ChatCompletionOptions;
  signal?: AbortSignal;
}
```

`WorkflowThinking` and `ThinkingMode` intentionally have the same values but
belong to separate layers. The workflow layer is public SDK; the provider layer
is the internal transport contract. Do not import workflow types into provider
types.

A small mapper at the session boundary should make the relationship explicit:

```ts
function toProviderThinking(
  thinking: WorkflowThinking | undefined,
): ThinkingMode | undefined {
  return thinking;
}
```

If the project prefers one shared type, move it to a neutral model-types module;
do not make `src/providers` depend on `src/workflows`.

## Request flow

```text
workflow session.run(prompt, { thinking })
  -> WorkflowAgentCoordinator
  -> AgentSession.run
  -> AgentComsService.handleUserMessage
  -> ModelProvider.chat
  -> OllamaProvider POST /api/chat { think: ... }
```

### `WorkflowAgentCoordinator`

No behavioral logic is required. It already forwards
`WorkflowAgentRunOptions` to `AgentSession.run()`. Update tests to ensure the
new option is not dropped.

### `AgentSession`

Pass `options.thinking` to `AgentComsService.handleUserMessage()` alongside
the existing tools setting.

Prefer changing the service signature to one options object instead of adding
another positional argument:

```ts
interface AgentTurnOptions {
  tools?: "default" | "none";
  thinking?: ThinkingMode;
}

handleUserMessage(
  userText: string,
  options?: AgentTurnOptions,
  signal?: AbortSignal,
): Promise<string>
```

Update all current callers in one change. This avoids an expanding list of
positional request controls.

### `AgentComsService`

For every `provider.chat()` call in the tool loop, include the same thinking
value:

```ts
options: {
  numCtx: this.contextWindow,
  ...(turnOptions.thinking === undefined
    ? {}
    : { thinking: turnOptions.thinking }),
},
```

The value must remain unchanged across all iterations after tool calls. A
single `session.run()` is one turn even if it makes multiple model requests.

### `OllamaProvider`

Extend the wire request body:

```ts
interface OllamaChatRequestBody {
  model: string;
  messages: OllamaWireMessage[];
  tools?: ToolDef[];
  stream: false;
  think?: boolean | "low" | "medium" | "high";
  options?: { num_ctx: number };
}
```

Use a dedicated exhaustive mapper:

```ts
function toOllamaThinking(
  thinking: ThinkingMode | undefined,
): boolean | "low" | "medium" | "high" | undefined {
  switch (thinking) {
    case undefined:
    case "default":
      return undefined;
    case "off":
      return false;
    case "on":
      return true;
    case "low":
    case "medium":
    case "high":
      return thinking;
  }
}
```

Only add `body.think` when the mapped value is not `undefined`. In particular,
do not lose `false` through a truthiness check.

## Thinking history

Ollama returns reasoning separately as `message.thinking`. Preserve it in the
internal message model even though workflows only receive final content.

Extend both message types:

```ts
export interface ChatMessage {
  role: ChatRole;
  content: string;
  thinking?: string;
  toolCalls?: ToolCall[];
  toolCallId?: string;
  toolName?: string;
}

interface OllamaWireMessage {
  role: string;
  content: string;
  thinking?: string;
  tool_calls?: OllamaWireToolCall[];
}
```

Update `toWireMessage()` and `fromWireMessage()` to copy non-empty thinking
content. This ensures history snapshots, forks, retargets, and subsequent tool
iterations retain reasoning.

When `tools: "none"`, `AgentComsService` currently creates a reduced history
message containing only `role` and `content`. Include `thinking` in that
reduced message when present.

Do not include thinking text in `WorkflowAgentResponse.content`. The response
continues to contain the final answer only. Exposing reasoning can be considered
separately because it has UI, storage, privacy, and provider-compatibility
implications.

## Errors and compatibility

- Omitting `thinking` must generate exactly the same Ollama request as today.
- `"default"` must also omit `think`.
- Unsupported model/value combinations follow the existing error path:
  `OllamaProviderError` is converted by `AgentComsService` to response content
  beginning with `Error:`; Flowmation must not retry with a different level.
- Non-reasoning models may ignore the option or return an Ollama error.
- Existing model configuration files require no migration.
- Existing workflow source requires no migration.
- Tool access and thinking mode are independent options.
- Cancellation behavior is unchanged.

## Tests

### `src/providers/OllamaProvider.test.ts`

Add request-body tests using a mocked `fetch`:

1. Omitted thinking does not add `think`.
2. `default` does not add `think`.
3. `off` sends `think: false`.
4. `on` sends `think: true`.
5. Low, medium, and high send their string values.
6. A response `message.thinking` is retained in the returned `ChatMessage`.
7. A historical `ChatMessage.thinking` is replayed in the next wire request.

### `src/services/AgentComsService.test.ts`

1. The selected thinking value is present on every provider request in a
   multi-iteration tool loop.
2. Thinking content is retained in history with tools enabled.
3. Thinking content is retained in history with `tools: "none"`.
4. Existing default behavior sends no thinking option.

### `src/classes/Agent.test.ts` or `src/classes/AgentSession.test.ts`

1. `session.run(..., { thinking: "off" })` reaches the provider as `off`.
2. Forked sessions retain prior thinking content in copied history.
3. Thinking mode is per run and does not become a sticky session default.

### Type-level/API coverage

Update workflow test fixtures so every allowed `WorkflowThinking` value
type-checks and an invalid value does not.

## Documentation

Update `docs/workflows.md`:

- Add `thinking?: WorkflowThinking` to `WorkflowAgentRunOptions`.
- Add the value/behavior table from this design.
- State that the option applies to every request in the turn's tool loop.
- State that support depends on the selected model.
- State that reasoning traces are not returned to workflow code.

Update the README workflow example only if it benefits from a deliberate
thinking choice; avoid adding the option to examples where it distracts from
the workflow concept.

## Acceptance criteria

- A workflow can call `session.run(prompt, { thinking: "off" })`.
- Ollama receives a top-level `"think": false`.
- A workflow can request `on`, `low`, `medium`, or `high` with the exact
  mapping defined above.
- Omitted and `default` thinking produce no `think` field.
- The selected mode applies to every Ollama request generated by that run.
- Ollama thinking output survives session history, tool iterations, snapshots,
  and forks.
- Final workflow agent content remains the model's `message.content`.
- Existing tests pass and new request/history tests cover the feature.
- `pnpm test` and `pnpm run build` pass.

## Suggested implementation order

1. Add public and internal thinking types.
2. Refactor `AgentComsService.handleUserMessage()` to accept an options object.
3. Thread the option through coordinator, session, and service.
4. Add Ollama request mapping.
5. Preserve thinking in message conversion and history.
6. Add provider, service, and session tests.
7. Update SDK documentation.
