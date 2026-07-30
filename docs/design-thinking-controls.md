# Workflow agent thinking controls

Status: implemented
Scope: workflow-created agent sessions and the Ollama-compatible provider

## Public SDK

Workflow authors can select reasoning behavior per agent turn:

```ts
type WorkflowThinking =
  | "default"
  | "off"
  | "on"
  | "low"
  | "medium"
  | "high";

interface WorkflowAgentRunOptions {
  tools?: "default" | "none";
  thinking?: WorkflowThinking;
}
```

```ts
const agent = await context.agents.create({ model: "fast-model" });

await agent.run("Classify these files.", { thinking: "off" });
await agent.run("Review the architecture.", { thinking: "high" });
```

The option belongs to `run`, so successive turns on one session can use
different modes. It is forwarded to every provider request in that turn,
including requests after tool results, and does not become sticky.

## Ollama mapping

| SDK value | Ollama top-level `think` |
| --- | --- |
| omitted or `"default"` | field omitted |
| `"off"` | `false` |
| `"on"` | `true` |
| `"low"` | `"low"` |
| `"medium"` | `"medium"` |
| `"high"` | `"high"` |

Flowmation does not infer model capability, clamp a level, or retry with a
different value. Provider rejection follows the normal model error path.

## Rust ownership and history

The Node host carries `WorkflowThinking` over the versioned callback protocol.
`WorkflowCallbackServices` forwards it to `ManagedWorkflowAgentRuntime`,
`AgentSession` applies it to one turn, and `flowmation-ollama` maps the
provider-neutral value onto the wire request.

Ollama response thinking is retained in internal chat history and replayed in
later provider requests, session snapshots, and forks. It is not exposed as
workflow response content. Tool-free turns strip ignored tool calls while
preserving thinking.

Elevation-level thinking becomes the default for session runs inside the
elevation operation. An explicit `session.run(..., { thinking })` overrides it,
and the elevation default is removed when the operation finishes.

## Parity tests

- `flowmation-ollama/src/lib.rs::maps_thinking_modes_to_top_level_think_field`
- `flowmation-ollama/src/lib.rs::retains_response_and_historical_thinking`
- `flowmation-application/src/agent.rs::thinking_applies_to_every_request_in_tool_loop`
- `flowmation-application/src/agent.rs::tool_free_history_retains_thinking_and_strips_tool_calls`
- `flowmation-application/src/agent.rs::session_thinking_override_is_not_sticky`
- `flowmation-application/src/agent.rs::copied_session_history_retains_prior_thinking`
- `flowmation-application/src/workflow_tests.rs::agent_callback_forwards_thinking_and_tools_modes`
- `flowmation-application/src/workflow_tests.rs::elevation_thinking_is_scoped_to_operation_session_runs`
