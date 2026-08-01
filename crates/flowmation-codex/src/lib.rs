use std::path::PathBuf;

use async_trait::async_trait;
use flowmation_application::{
    ChatCompletionRequest, ChatCompletionResult, ChatMessage, ChatRole, ModelProvider,
    ProviderError, ToolCall,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::app_server::AppServerConnection;

mod app_server;
mod discovery;

pub use discovery::{CodexAccountStatus, CodexDeviceLogin, CodexModel};

pub const OPENAI_SUBSCRIPTION_PROVIDER_NAME: &str = "openai";

const PROVIDER_PROMPT: &str = "\
Act only as a language-model backend for Flowmation. Do not use Codex built-in tools, \
inspect the filesystem, change files, or run commands. Read the supplied conversation and \
available tool definitions. Return a direct assistant response. If a tool is needed, place \
the requested calls in toolCalls and do not pretend that the tool already ran. Encode each \
tool call's arguments field as a JSON object string.";

#[derive(Debug)]
pub struct CodexProvider {
    executable: PathBuf,
}

impl Default for CodexProvider {
    fn default() -> Self {
        Self::new(
            std::env::var_os("FLOWMATION_CODEX_BIN")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("codex")),
        )
    }
}

impl CodexProvider {
    #[must_use]
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }

    async fn connect(&self) -> Result<AppServerConnection, ProviderError> {
        AppServerConnection::spawn(&self.executable)
            .await
            .map_err(|error| {
                ProviderError::Unavailable(format!(
                    "Could not start the OpenAI Codex app server with \"{}\": {error}. \
Install the Codex CLI and run `codex login --device-auth`, or set \
FLOWMATION_CODEX_BIN to its executable path.",
                    self.executable.display()
                ))
            })
    }

    pub async fn account_status(&self) -> Result<CodexAccountStatus, ProviderError> {
        let cancellation = CancellationToken::new();
        let mut connection = self.connect().await?;
        connection.initialize(&cancellation).await?;
        let result = connection.account_status(&cancellation).await;
        connection.stop().await;
        result
    }

    pub async fn model_catalog(
        &self,
    ) -> Result<(CodexAccountStatus, Vec<CodexModel>), ProviderError> {
        let cancellation = CancellationToken::new();
        let mut connection = self.connect().await?;
        connection.initialize(&cancellation).await?;
        let result = async {
            let account = connection.account_status(&cancellation).await?;
            let models = if account.uses_chatgpt_subscription() {
                connection.list_models(&cancellation).await?
            } else {
                Vec::new()
            };
            Ok((account, models))
        }
        .await;
        connection.stop().await;
        result
    }

    pub async fn list_models(&self) -> Result<Vec<CodexModel>, ProviderError> {
        let cancellation = CancellationToken::new();
        let mut connection = self.connect().await?;
        connection.initialize(&cancellation).await?;
        require_chatgpt_subscription(&mut connection, &cancellation).await?;
        let result = connection.list_models(&cancellation).await;
        connection.stop().await;
        result
    }

    pub async fn login_with_device_code(
        &self,
        show_instructions: impl FnOnce(&CodexDeviceLogin),
    ) -> Result<(), ProviderError> {
        let cancellation = CancellationToken::new();
        let mut connection = self.connect().await?;
        connection.initialize(&cancellation).await?;
        let login = connection.start_device_login(&cancellation).await?;
        show_instructions(&login);
        let result = connection
            .wait_for_login(&login.login_id, &cancellation)
            .await;
        connection.stop().await;
        result
    }
}

#[async_trait]
impl ModelProvider for CodexProvider {
    fn id(&self) -> &str {
        OPENAI_SUBSCRIPTION_PROVIDER_NAME
    }

    async fn chat(
        &self,
        request: ChatCompletionRequest,
        cancellation: &CancellationToken,
    ) -> Result<ChatCompletionResult, ProviderError> {
        let mut connection = self.connect().await?;
        connection.initialize(cancellation).await?;
        require_chatgpt_subscription(&mut connection, cancellation).await?;
        let thread_id = connection
            .start_thread(&request.model, cancellation)
            .await?;
        let result = connection
            .run_turn(&thread_id, &request, cancellation)
            .await;
        connection.stop().await;
        result
    }
}

async fn require_chatgpt_subscription(
    connection: &mut AppServerConnection,
    cancellation: &CancellationToken,
) -> Result<(), ProviderError> {
    let account = connection.account_status(cancellation).await?;
    if account.uses_chatgpt_subscription() {
        return Ok(());
    }
    let current = account.account_type.as_deref().unwrap_or("signed out");
    Err(ProviderError::Unavailable(format!(
        "Flowmation's OpenAI subscription provider requires ChatGPT authentication through \
Codex, but Codex is currently {current}. API-key billing is not used by this provider."
    )))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelOutput {
    content: String,
    #[serde(default)]
    tool_calls: Vec<ModelToolCall>,
}

#[derive(Deserialize)]
struct ModelToolCall {
    name: String,
    arguments: String,
}

fn build_prompt(request: &ChatCompletionRequest) -> Result<String, ProviderError> {
    let payload = json!({
        "messages": request.messages,
        "tools": request.tools
    });
    let payload = serde_json::to_string(&payload)
        .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
    Ok(format!("{PROVIDER_PROMPT}\n\nInput JSON:\n{payload}"))
}

fn output_schema(request: &ChatCompletionRequest) -> Value {
    let names = request
        .tools
        .iter()
        .map(|tool| Value::String(tool.function.name.clone()))
        .collect::<Vec<_>>();
    let name_schema = if names.is_empty() {
        json!({ "type": "string" })
    } else {
        json!({ "type": "string", "enum": names })
    };
    json!({
        "type": "object",
        "properties": {
            "content": { "type": "string" },
            "toolCalls": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "name": name_schema,
                        "arguments": {
                            "type": "string",
                            "description": "A JSON-encoded object containing the tool arguments"
                        }
                    },
                    "required": ["name", "arguments"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["content", "toolCalls"],
        "additionalProperties": false
    })
}

fn parse_model_output(text: &str) -> Result<ChatCompletionResult, ProviderError> {
    let output: ModelOutput = serde_json::from_str(text).map_err(|error| {
        ProviderError::InvalidResponse(format!(
            "Codex returned an invalid structured response: {error}"
        ))
    })?;
    let tool_calls = output
        .tool_calls
        .into_iter()
        .map(|call| {
            Ok(ToolCall {
                id: format!("call_{}", Uuid::new_v4()),
                name: call.name,
                arguments: parse_tool_arguments(&call.arguments)?,
            })
        })
        .collect::<Result<Vec<_>, ProviderError>>()?;
    Ok(ChatCompletionResult {
        message: ChatMessage {
            role: ChatRole::Assistant,
            content: output.content,
            thinking: None,
            tool_calls,
            tool_call_id: None,
            tool_name: None,
        },
    })
}

fn parse_tool_arguments(arguments: &str) -> Result<Map<String, Value>, ProviderError> {
    let value = serde_json::from_str::<Value>(arguments).map_err(|error| {
        ProviderError::InvalidResponse(format!(
            "Codex returned invalid JSON tool arguments: {error}"
        ))
    })?;
    value.as_object().cloned().ok_or_else(|| {
        ProviderError::InvalidResponse(
            "Codex returned tool arguments that were not a JSON object".to_owned(),
        )
    })
}

#[cfg(test)]
mod tests {
    use flowmation_application::{
        ChatCompletionOptions, ChatCompletionRequest, ChatMessage, ChatRole, ToolDefinition,
    };
    use serde_json::{Value, json};

    use super::{build_prompt, output_schema, parse_model_output};

    fn request() -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "gpt-5.6".to_owned(),
            messages: vec![ChatMessage::new(ChatRole::User, "read the file")],
            tools: vec![
                serde_json::from_value::<ToolDefinition>(json!({
                    "type": "function",
                    "function": {
                        "name": "read_file",
                        "description": "Read one file",
                        "parameters": {
                            "type": "object",
                            "properties": { "path": { "type": "string" } }
                        }
                    }
                }))
                .unwrap_or_else(|error| panic!("test tool definition must be valid: {error}")),
            ],
            options: ChatCompletionOptions::default(),
        }
    }

    #[test]
    fn prompt_contains_conversation_and_tools() -> Result<(), Box<dyn std::error::Error>> {
        let prompt = build_prompt(&request())?;
        assert!(prompt.contains("read the file"));
        assert!(prompt.contains("read_file"));
        assert!(prompt.contains("Do not use Codex built-in tools"));
        Ok(())
    }

    #[test]
    fn schema_restricts_calls_to_available_tools() {
        let schema = output_schema(&request());
        assert_eq!(
            schema["properties"]["toolCalls"]["items"]["properties"]["name"]["enum"][0],
            "read_file"
        );
        assert_eq!(
            schema["properties"]["toolCalls"]["items"]["properties"]["arguments"]["type"],
            "string"
        );
        assert_strict_object_schemas(&schema);
    }

    #[test]
    fn parses_content_and_tool_calls() -> Result<(), Box<dyn std::error::Error>> {
        let result = parse_model_output(
            r#"{"content":"","toolCalls":[{"name":"read_file","arguments":"{\"path\":\"README.md\"}"}]}"#,
        )?;
        assert_eq!(result.message.role, ChatRole::Assistant);
        assert_eq!(result.message.tool_calls.len(), 1);
        assert_eq!(result.message.tool_calls[0].name, "read_file");
        assert_eq!(result.message.tool_calls[0].arguments["path"], "README.md");
        Ok(())
    }

    #[test]
    fn rejects_invalid_structured_output() {
        assert!(parse_model_output("not json").is_err());
        assert!(
            parse_model_output(
                r#"{"content":"","toolCalls":[{"name":"read_file","arguments":"[]"}]}"#
            )
            .is_err()
        );
    }

    fn assert_strict_object_schemas(value: &Value) {
        if value.get("type").and_then(Value::as_str) == Some("object") {
            assert_eq!(value.get("additionalProperties"), Some(&Value::Bool(false)));
            let properties = value
                .get("properties")
                .and_then(Value::as_object)
                .unwrap_or_else(|| panic!("object schema must define properties"));
            let required = value
                .get("required")
                .and_then(Value::as_array)
                .unwrap_or_else(|| panic!("object schema must define required properties"));
            for name in properties.keys() {
                assert!(required.iter().any(|entry| entry.as_str() == Some(name)));
            }
        }
        match value {
            Value::Array(values) => {
                for value in values {
                    assert_strict_object_schemas(value);
                }
            }
            Value::Object(values) => {
                for value in values.values() {
                    assert_strict_object_schemas(value);
                }
            }
            _ => {}
        }
    }
}
