# flowmation-workflow-host

Rust client for Flowmation's versioned, bidirectional JSON-RPC workflow host.

`WorkflowHost::spawn` launches Node and negotiates the protocol.
`inspect`, `run`, and `cancel` are typed host operations.
`WorkflowCallbackHandler` receives typed reverse SDK requests and can use the
provided `CallbackInvoker` for nested JavaScript callbacks. `shutdown` first
requests a graceful exit, then terminates the host process group when the
deadline expires. All wire types and the protocol version are exported from
`flowmation_workflow_host::protocol`.
