use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const JSON_RPC_VERSION: &str = "2.0";
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(untagged)]
pub enum RpcId {
    Number(u64),
    String(String),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;
    pub const VERSION_MISMATCH: i32 = -32001;
    pub const CANCELLED: i32 = -32002;

    #[must_use]
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    #[must_use]
    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandshakeParams {
    pub protocol_version: u32,
    pub client_name: String,
    pub client_version: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandshakeResult {
    pub protocol_version: u32,
    pub host_name: String,
    pub host_version: String,
    pub runtime: String,
    pub capabilities: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostReady {
    pub protocol_version: u32,
    pub runtime: String,
}

pub type JsonSchema = Value;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkflowPresentation {
    Direct,
    Agent,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentInvocationPolicy {
    Disabled,
    Confirm,
    Automatic,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowMetadata {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<JsonSchema>,
    pub agent_invocation: AgentInvocationPolicy,
    pub presentation: WorkflowPresentation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectWorkflowParams {
    pub entry_path: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectWorkflowResult {
    pub metadata: WorkflowMetadata,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunWorkflowParams {
    pub entry_path: String,
    pub run_id: String,
    pub project_dir: String,
    pub input: Value,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunWorkflowResult {
    pub value: Value,
    pub presentation: WorkflowPresentation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelWorkflowParams {
    pub run_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelWorkflowResult {
    pub cancelled: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvokeCallbackParams {
    pub callback_id: String,
    pub arguments: Vec<Value>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallbackRef {
    pub callback_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointCallback {
    pub run_id: String,
    pub key: String,
    pub callback_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectCallback {
    pub run_id: String,
    pub key: String,
    pub idempotency_key: String,
    pub callback_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecCallback {
    pub run_id: String,
    pub command: String,
    pub args: Vec<String>,
    pub options: Value,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapCallback {
    pub run_id: String,
    pub items: Vec<Value>,
    pub concurrency: u32,
    pub callback_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCreateCallback {
    pub run_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentForkCallback {
    pub run_id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkflowTools {
    Default,
    None,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkflowThinking {
    Default,
    Off,
    On,
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<WorkflowTools>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<WorkflowThinking>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunCallback {
    pub run_id: String,
    pub session_id: String,
    pub prompt: String,
    pub options: AgentRunOptions,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRef {
    pub provider: String,
    pub model: String,
    pub active: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSession {
    pub id: String,
    pub model: ModelRef,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunResult {
    pub content: String,
    pub model: ModelRef,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HumanRequestKind {
    Approval,
    Choice,
    Text,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HumanChoice {
    pub value: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HumanCallback {
    pub run_id: String,
    pub kind: HumanRequestKind,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<HumanChoice>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ElevationCallback {
    pub run_id: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<WorkflowThinking>,
    pub attempts: u32,
    pub context: Value,
    pub operation_callback_id: String,
    pub check_callback_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_callback_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogCallback {
    pub run_id: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkflowCallbackRequest {
    Checkpoint(CheckpointCallback),
    Effect(EffectCallback),
    Exec(ExecCallback),
    Map(MapCallback),
    AgentCreate(AgentCreateCallback),
    AgentFork(AgentForkCallback),
    AgentRun(AgentRunCallback),
    Human(HumanCallback),
    Elevate(ElevationCallback),
    Log(LogCallback),
    Unknown { method: String, params: Value },
}

impl WorkflowCallbackRequest {
    pub(crate) fn from_method(method: String, params: Value) -> Result<Self, serde_json::Error> {
        Ok(match method.as_str() {
            "sdk.checkpoint" => Self::Checkpoint(serde_json::from_value(params)?),
            "sdk.effect" => Self::Effect(serde_json::from_value(params)?),
            "sdk.exec" => Self::Exec(serde_json::from_value(params)?),
            "sdk.map" => Self::Map(serde_json::from_value(params)?),
            "sdk.agent.create" => Self::AgentCreate(serde_json::from_value(params)?),
            "sdk.agent.fork" => Self::AgentFork(serde_json::from_value(params)?),
            "sdk.agent.run" => Self::AgentRun(serde_json::from_value(params)?),
            "sdk.human" => Self::Human(serde_json::from_value(params)?),
            "sdk.elevate" => Self::Elevate(serde_json::from_value(params)?),
            "sdk.log" => Self::Log(serde_json::from_value(params)?),
            _ => Self::Unknown { method, params },
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowEvent {
    pub run_id: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostNotification {
    Ready(HostReady),
    Event(WorkflowEvent),
    Unknown { method: String, params: Value },
}

impl HostNotification {
    pub(crate) fn from_method(method: String, params: Value) -> Result<Self, serde_json::Error> {
        Ok(match method.as_str() {
            "host.ready" => Self::Ready(serde_json::from_value(params)?),
            "workflow.event" => Self::Event(serde_json::from_value(params)?),
            _ => Self::Unknown { method, params },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_messages_use_camel_case_wire_fields() {
        let message = serde_json::to_value(RunWorkflowParams {
            entry_path: "/tmp/WORKFLOW.js".into(),
            run_id: "run-1".into(),
            project_dir: "/tmp/project".into(),
            input: Value::String("hello".into()),
        })
        .unwrap_or_else(|error| panic!("serialization failed: {error}"));

        assert_eq!(message["entryPath"], "/tmp/WORKFLOW.js");
        assert_eq!(message["runId"], "run-1");
        assert_eq!(message["projectDir"], "/tmp/project");
    }

    #[test]
    fn callback_requests_are_decoded_by_method() {
        let request = WorkflowCallbackRequest::from_method(
            "sdk.checkpoint".into(),
            serde_json::json!({
                "runId": "run-1",
                "key": "draft",
                "callbackId": "callback-1"
            }),
        )
        .unwrap_or_else(|error| panic!("callback decoding failed: {error}"));

        assert!(matches!(
            request,
            WorkflowCallbackRequest::Checkpoint(CheckpointCallback { key, .. })
                if key == "draft"
        ));
    }
}
