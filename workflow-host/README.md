# Flowmation workflow host

This package is the trusted JavaScript/TypeScript compatibility layer for the
Rust application. It loads `WORKFLOW.js` and `WORKFLOW.ts` with Node.js
semantics, provides the `flowmation/workflow` authoring SDK, and communicates
with Rust through newline-delimited JSON-RPC 2.0 on stdin/stdout.

The protocol version is `1`. Rust starts with `host.handshake`, then uses
`workflow.inspect`, `workflow.run`, `workflow.cancel`, `callback.invoke`, and
`host.shutdown`. Workflow context operations are reverse requests named
`sdk.checkpoint`, `sdk.effect`, `sdk.exec`, `sdk.map`, `sdk.agent.create`,
`sdk.agent.fork`, `sdk.agent.run`, `sdk.human`, `sdk.elevate`, and `sdk.log`.
The host also emits `host.ready` and `workflow.event` notifications.

Only protocol messages are written to stdout. Runtime diagnostics go to
stderr. Rust must fingerprint and authorize the complete workflow directory
before asking the host to inspect or execute its entry module.

```sh
pnpm --dir workflow-host run build
pnpm --dir workflow-host test
node workflow-host/dist/index.js
```

The package exports `./workflow`, so installed authoring environments can use:

```ts
import { defineWorkflow } from "flowmation/workflow";
```
