use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::JsonValue;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingMode {
    #[default]
    Default,
    Off,
    On,
    Low,
    Medium,
    High,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

impl ChatMessage {
    #[must_use]
    pub fn new(role: ChatRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            thinking: None,
            tool_calls: Vec::new(),
            tool_call_id: None,
            tool_name: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub arguments: serde_json::Map<String, JsonValue>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub kind: ToolDefinitionKind,
    pub function: FunctionDefinition,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolDefinitionKind {
    Function,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FunctionDefinition {
    pub name: String,
    pub description: String,
    pub parameters: JsonSchema,
}

pub type ToolFunctionDefinition = FunctionDefinition;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct JsonSchema {
    #[serde(rename = "type")]
    pub kind: JsonSchemaType,
    pub properties: BTreeMap<String, JsonSchemaProperty>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum JsonSchemaType {
    One(JsonValueType),
    Many(Vec<JsonValueType>),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JsonValueType {
    String,
    Number,
    Boolean,
    Object,
    Array,
    Null,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct JsonSchemaProperty {
    #[serde(rename = "type")]
    pub kind: JsonSchemaType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "enum", default, skip_serializing_if = "Option::is_none")]
    pub allowed_values: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Box<Self>>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatCompletionOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub num_ctx: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingMode>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinition>,
    #[serde(default)]
    pub options: ChatCompletionOptions,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ChatCompletionResult {
    pub message: ChatMessage,
}

#[cfg(test)]
mod tests {
    use super::ThinkingMode;

    // Legacy: src/workflows/types.test.ts — accepts every workflow thinking mode.
    #[test]
    fn accepts_every_workflow_thinking_mode() -> Result<(), Box<dyn std::error::Error>> {
        let modes: Vec<ThinkingMode> =
            serde_json::from_str(r#"["default","off","on","low","medium","high"]"#)?;

        assert_eq!(
            modes,
            vec![
                ThinkingMode::Default,
                ThinkingMode::Off,
                ThinkingMode::On,
                ThinkingMode::Low,
                ThinkingMode::Medium,
                ThinkingMode::High,
            ]
        );
        Ok(())
    }

    #[test]
    fn tool_definition_kind_serializes_as_function() -> Result<(), Box<dyn std::error::Error>> {
        let kind = serde_json::to_string(&super::ToolDefinitionKind::Function)?;

        assert_eq!(kind, "\"function\"");
        Ok(())
    }
}
