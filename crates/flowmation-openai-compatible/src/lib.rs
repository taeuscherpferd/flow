use std::fmt::Debug;
use std::sync::Arc;

use async_trait::async_trait;
use flowmation_application::{
    ChatCompletionRequest, ChatCompletionResult, ChatMessage, ChatRole, ModelProvider,
    ProviderError, ThinkingMode, ToolCall,
};
use flowmation_domain::config::CredentialSource;
use flowmation_http::{HttpTransport, HttpTransportError, ReqwestTransport};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

#[derive(Debug, thiserror::Error)]
pub enum SecretResolveError {
    #[error("Environment variable \"{name}\" is not set.")]
    MissingEnvironment { name: String },
    #[error("Environment variable \"{name}\" is empty.")]
    EmptyEnvironment { name: String },
    #[error("Environment variable \"{name}\" is not valid Unicode.")]
    InvalidEnvironment { name: String },
}

pub trait SecretResolver: Debug + Send + Sync {
    fn resolve(&self, source: &CredentialSource) -> Result<String, SecretResolveError>;
}

#[derive(Debug, Default)]
pub struct EnvironmentSecretResolver;

impl SecretResolver for EnvironmentSecretResolver {
    fn resolve(&self, source: &CredentialSource) -> Result<String, SecretResolveError> {
        let CredentialSource::Environment { name } = source;
        match std::env::var(name) {
            Ok(value) if value.is_empty() => {
                Err(SecretResolveError::EmptyEnvironment { name: name.clone() })
            }
            Ok(value) => Ok(value),
            Err(std::env::VarError::NotPresent) => {
                Err(SecretResolveError::MissingEnvironment { name: name.clone() })
            }
            Err(std::env::VarError::NotUnicode(_)) => {
                Err(SecretResolveError::InvalidEnvironment { name: name.clone() })
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct WireFunction {
    name: String,
    arguments: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct WireToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    function: WireFunction,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct WireMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_tool_calls",
        skip_serializing_if = "Vec::is_empty"
    )]
    tool_calls: Vec<WireToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refusal: Option<String>,
}

fn deserialize_tool_calls<'de, D>(deserializer: D) -> Result<Vec<WireToolCall>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<Vec<WireToolCall>>::deserialize(deserializer).map(Option::unwrap_or_default)
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct WireRequest {
    model: String,
    messages: Vec<WireMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<flowmation_application::ToolDefinition>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
struct WireChoice {
    message: WireMessage,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
struct WireResponse {
    choices: Vec<WireChoice>,
}

#[derive(Debug)]
pub struct OpenAiCompatibleProvider {
    id: String,
    base_url: String,
    token_source: Option<CredentialSource>,
    secrets: Arc<dyn SecretResolver>,
    transport: Arc<dyn HttpTransport>,
}

impl OpenAiCompatibleProvider {
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        base_url: impl Into<String>,
        token_source: Option<CredentialSource>,
    ) -> Self {
        Self::with_dependencies(
            id,
            base_url,
            token_source,
            Arc::new(EnvironmentSecretResolver),
            Arc::new(ReqwestTransport::default()),
        )
    }

    #[must_use]
    pub fn with_dependencies(
        id: impl Into<String>,
        base_url: impl Into<String>,
        token_source: Option<CredentialSource>,
        secrets: Arc<dyn SecretResolver>,
        transport: Arc<dyn HttpTransport>,
    ) -> Self {
        Self {
            id: id.into(),
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            token_source,
            secrets,
            transport,
        }
    }

    fn resolve_token(&self) -> Result<Option<String>, ProviderError> {
        self.token_source
            .as_ref()
            .map(|source| {
                self.secrets
                    .resolve(source)
                    .map_err(|error| ProviderError::Unavailable(error.to_string()))
            })
            .transpose()
    }

    fn wire_request(request: ChatCompletionRequest) -> Result<WireRequest, ProviderError> {
        let reasoning_effort = match request.options.thinking {
            Some(ThinkingMode::Off) => Some("none"),
            Some(ThinkingMode::Low) => Some("low"),
            Some(ThinkingMode::Medium) => Some("medium"),
            Some(ThinkingMode::High) => Some("high"),
            None | Some(ThinkingMode::Default | ThinkingMode::On) => None,
        }
        .map(str::to_owned);
        let messages = request
            .messages
            .into_iter()
            .map(wire_message)
            .collect::<Result<Vec<_>, ProviderError>>()?;
        Ok(WireRequest {
            model: request.model,
            messages,
            tools: request.tools,
            stream: false,
            reasoning_effort,
        })
    }

    fn parse_response(body: &str) -> Result<ChatCompletionResult, ProviderError> {
        let mut response: WireResponse = serde_json::from_str(body).map_err(|error| {
            ProviderError::InvalidResponse(format!(
                "OpenAI-compatible endpoint returned invalid JSON: {error}"
            ))
        })?;
        let choice = response.choices.drain(..).next().ok_or_else(|| {
            ProviderError::InvalidResponse(
                "OpenAI-compatible endpoint returned no choices.".to_owned(),
            )
        })?;
        let role = parse_role(&choice.message.role)?;
        let tool_calls = choice
            .message
            .tool_calls
            .into_iter()
            .map(|call| {
                let arguments = serde_json::from_str::<Value>(&call.function.arguments)
                    .map_err(|error| {
                        ProviderError::InvalidResponse(format!(
                            "OpenAI-compatible endpoint returned invalid tool arguments: {error}"
                        ))
                    })?
                    .as_object()
                    .cloned()
                    .ok_or_else(|| {
                        ProviderError::InvalidResponse(
                            "OpenAI-compatible endpoint returned non-object tool arguments."
                                .to_owned(),
                        )
                    })?;
                Ok(ToolCall {
                    id: call.id,
                    name: call.function.name,
                    arguments,
                })
            })
            .collect::<Result<Vec<_>, ProviderError>>()?;
        Ok(ChatCompletionResult {
            message: ChatMessage {
                role,
                content: choice
                    .message
                    .content
                    .or(choice.message.refusal)
                    .unwrap_or_default(),
                thinking: None,
                tool_calls,
                tool_call_id: choice.message.tool_call_id,
                tool_name: None,
            },
        })
    }
}

fn wire_message(message: ChatMessage) -> Result<WireMessage, ProviderError> {
    let role = match message.role {
        ChatRole::System => "system",
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
        ChatRole::Tool => "tool",
    }
    .to_owned();
    let tool_calls = message
        .tool_calls
        .into_iter()
        .map(|call| {
            let arguments = serde_json::to_string(&call.arguments)
                .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
            Ok(WireToolCall {
                id: call.id,
                kind: "function".to_owned(),
                function: WireFunction {
                    name: call.name,
                    arguments,
                },
            })
        })
        .collect::<Result<Vec<_>, ProviderError>>()?;
    let content = if message.role == ChatRole::Assistant
        && message.content.is_empty()
        && !tool_calls.is_empty()
    {
        None
    } else {
        Some(message.content)
    };
    Ok(WireMessage {
        role,
        content,
        tool_calls,
        tool_call_id: message.tool_call_id,
        refusal: None,
    })
}

fn parse_role(role: &str) -> Result<ChatRole, ProviderError> {
    match role {
        "system" | "developer" => Ok(ChatRole::System),
        "user" => Ok(ChatRole::User),
        "assistant" => Ok(ChatRole::Assistant),
        "tool" => Ok(ChatRole::Tool),
        role => Err(ProviderError::InvalidResponse(format!(
            "OpenAI-compatible endpoint returned unsupported role \"{role}\"."
        ))),
    }
}

fn redact_secret(message: String, secret: Option<&str>) -> String {
    secret
        .filter(|value| !value.is_empty())
        .map_or(message.clone(), |secret| {
            message.replace(secret, "[redacted]")
        })
}

#[async_trait]
impl ModelProvider for OpenAiCompatibleProvider {
    fn id(&self) -> &str {
        &self.id
    }

    async fn chat(
        &self,
        request: ChatCompletionRequest,
        cancellation: &CancellationToken,
    ) -> Result<ChatCompletionResult, ProviderError> {
        let token = self.resolve_token()?;
        let body = serde_json::to_value(Self::wire_request(request)?)
            .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
        let response = self
            .transport
            .post_json(
                &format!("{}/chat/completions", self.base_url),
                token.as_deref(),
                body,
                cancellation,
            )
            .await
            .map_err(|error| match error {
                HttpTransportError::Cancelled => ProviderError::Cancelled,
                HttpTransportError::Request(message) => ProviderError::Unavailable(redact_secret(
                    format!(
                        "Could not reach OpenAI-compatible endpoint at {}: {message}",
                        self.base_url
                    ),
                    token.as_deref(),
                )),
            })?;
        if !response.status.is_success() {
            return Err(ProviderError::Unavailable(redact_secret(
                format!(
                    "OpenAI-compatible endpoint returned {}: {}",
                    response.status.as_u16(),
                    response.body
                ),
                token.as_deref(),
            )));
        }
        Self::parse_response(&response.body)
    }
}
