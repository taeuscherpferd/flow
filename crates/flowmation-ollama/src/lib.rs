use std::sync::Arc;

use async_trait::async_trait;
use flowmation_application::{
    ChatCompletionRequest, ChatCompletionResult, ChatMessage, ChatRole, ModelProvider,
    ProviderError, ThinkingMode, ToolCall,
};
use flowmation_http::{HttpTransport, HttpTransportError, ReqwestTransport};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct WireToolFunction {
    name: String,
    arguments: Map<String, Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct WireToolCall {
    function: WireToolFunction,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct WireMessage {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<WireToolCall>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
enum WireThinking {
    Enabled(bool),
    Level(String),
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct WireOptions {
    num_ctx: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct WireRequest {
    model: String,
    messages: Vec<WireMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<flowmation_application::ToolDefinition>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    think: Option<WireThinking>,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<WireOptions>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
struct WireResponse {
    message: WireMessage,
}

#[derive(Debug)]
pub struct OllamaProvider {
    base_url: String,
    transport: Arc<dyn HttpTransport>,
}

impl OllamaProvider {
    #[must_use]
    pub fn new(base_url: impl Into<String>) -> Self {
        Self::with_transport(base_url, Arc::new(ReqwestTransport::default()))
    }

    #[must_use]
    pub fn with_transport(base_url: impl Into<String>, transport: Arc<dyn HttpTransport>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            transport,
        }
    }

    fn wire_request(request: ChatCompletionRequest) -> WireRequest {
        WireRequest {
            model: request.model,
            messages: request
                .messages
                .into_iter()
                .map(|message| WireMessage {
                    role: match message.role {
                        ChatRole::System => "system",
                        ChatRole::User => "user",
                        ChatRole::Assistant => "assistant",
                        ChatRole::Tool => "tool",
                    }
                    .to_owned(),
                    content: message.content,
                    thinking: message.thinking.filter(|thinking| !thinking.is_empty()),
                    tool_calls: message
                        .tool_calls
                        .into_iter()
                        .map(|call| WireToolCall {
                            function: WireToolFunction {
                                name: call.name,
                                arguments: call.arguments,
                            },
                        })
                        .collect(),
                })
                .collect(),
            tools: request.tools,
            stream: false,
            think: match request.options.thinking {
                None | Some(ThinkingMode::Default) => None,
                Some(ThinkingMode::Off) => Some(WireThinking::Enabled(false)),
                Some(ThinkingMode::On) => Some(WireThinking::Enabled(true)),
                Some(ThinkingMode::Low) => Some(WireThinking::Level("low".to_owned())),
                Some(ThinkingMode::Medium) => Some(WireThinking::Level("medium".to_owned())),
                Some(ThinkingMode::High) => Some(WireThinking::Level("high".to_owned())),
            },
            options: request
                .options
                .num_ctx
                .map(|num_ctx| WireOptions { num_ctx }),
        }
    }

    fn parse_response(body: &str) -> Result<ChatCompletionResult, ProviderError> {
        let response: WireResponse = serde_json::from_str(body)
            .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
        let role = match response.message.role.as_str() {
            "system" => ChatRole::System,
            "user" => ChatRole::User,
            "assistant" => ChatRole::Assistant,
            "tool" => ChatRole::Tool,
            role => {
                return Err(ProviderError::InvalidResponse(format!(
                    "Ollama returned unsupported message role \"{role}\""
                )));
            }
        };
        Ok(ChatCompletionResult {
            message: ChatMessage {
                role,
                content: response.message.content,
                thinking: response
                    .message
                    .thinking
                    .filter(|thinking| !thinking.is_empty()),
                tool_calls: response
                    .message
                    .tool_calls
                    .into_iter()
                    .map(|call| ToolCall {
                        id: format!("call_{}", Uuid::new_v4()),
                        name: call.function.name,
                        arguments: call.function.arguments,
                    })
                    .collect(),
                tool_call_id: None,
                tool_name: None,
            },
        })
    }
}

#[async_trait]
impl ModelProvider for OllamaProvider {
    fn id(&self) -> &str {
        "ollama"
    }

    async fn chat(
        &self,
        request: ChatCompletionRequest,
        cancellation: &CancellationToken,
    ) -> Result<ChatCompletionResult, ProviderError> {
        let body = serde_json::to_value(Self::wire_request(request))
            .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
        let response = self
            .transport
            .post_json(
                &format!("{}/api/chat", self.base_url),
                None,
                body,
                cancellation,
            )
            .await
            .map_err(|error| match error {
                HttpTransportError::Cancelled => ProviderError::Cancelled,
                HttpTransportError::Request(message) => ProviderError::Unavailable(format!(
                    "Could not reach Ollama at {} — is it running? (ollama serve)\n{message}",
                    self.base_url
                )),
            })?;
        if !response.status.is_success() {
            return Err(ProviderError::Unavailable(format!(
                "Ollama returned {}: {}",
                response.status.as_u16(),
                response.body
            )));
        }
        Self::parse_response(&response.body)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use flowmation_application::{
        ChatCompletionOptions, ChatCompletionRequest, ChatMessage, ChatRole, ModelProvider,
        ThinkingMode,
    };
    use flowmation_http::{HttpResponse, HttpTransport, HttpTransportError};
    use http::StatusCode;
    use serde_json::Value;
    use tokio_util::sync::CancellationToken;

    use crate::OllamaProvider;

    #[derive(Debug)]
    struct RecordingTransport {
        bodies: Mutex<Vec<Value>>,
    }

    #[async_trait]
    impl HttpTransport for RecordingTransport {
        async fn post_json(
            &self,
            _url: &str,
            _bearer_token: Option<&str>,
            body: Value,
            _cancellation: &CancellationToken,
        ) -> Result<HttpResponse, HttpTransportError> {
            self.bodies
                .lock()
                .map_err(|error| HttpTransportError::Request(error.to_string()))?
                .push(body);
            Ok(HttpResponse {
                status: StatusCode::OK,
                body:
                    r#"{"message":{"role":"assistant","content":"answer","thinking":"reasoning"}}"#
                        .to_owned(),
            })
        }
    }

    #[tokio::test]
    async fn maps_thinking_modes_to_top_level_think_field() -> Result<(), Box<dyn std::error::Error>>
    {
        let cases = [
            (None, None),
            (Some(ThinkingMode::Default), None),
            (Some(ThinkingMode::Off), Some(Value::Bool(false))),
            (Some(ThinkingMode::On), Some(Value::Bool(true))),
            (
                Some(ThinkingMode::Low),
                Some(Value::String("low".to_owned())),
            ),
            (
                Some(ThinkingMode::Medium),
                Some(Value::String("medium".to_owned())),
            ),
            (
                Some(ThinkingMode::High),
                Some(Value::String("high".to_owned())),
            ),
        ];
        for (thinking, expected) in cases {
            let transport = Arc::new(RecordingTransport {
                bodies: Mutex::new(Vec::new()),
            });
            let provider = OllamaProvider::with_transport("http://ollama.test", transport.clone());
            provider
                .chat(
                    ChatCompletionRequest {
                        model: "test-model".to_owned(),
                        messages: vec![ChatMessage::new(ChatRole::User, "question")],
                        tools: Vec::new(),
                        options: ChatCompletionOptions {
                            num_ctx: None,
                            thinking,
                        },
                    },
                    &CancellationToken::new(),
                )
                .await?;
            let bodies = transport.bodies.lock().map_err(|error| error.to_string())?;
            assert_eq!(bodies[0].get("think").cloned(), expected);
        }
        Ok(())
    }

    #[tokio::test]
    async fn retains_response_and_historical_thinking() -> Result<(), Box<dyn std::error::Error>> {
        let transport = Arc::new(RecordingTransport {
            bodies: Mutex::new(Vec::new()),
        });
        let provider = OllamaProvider::with_transport("http://ollama.test", transport.clone());
        let mut historical = ChatMessage::new(ChatRole::Assistant, "previous answer");
        historical.thinking = Some("previous reasoning".to_owned());
        let response = provider
            .chat(
                ChatCompletionRequest {
                    model: "test-model".to_owned(),
                    messages: vec![historical],
                    tools: Vec::new(),
                    options: ChatCompletionOptions::default(),
                },
                &CancellationToken::new(),
            )
            .await?;
        assert_eq!(response.message.thinking.as_deref(), Some("reasoning"));
        let bodies = transport.bodies.lock().map_err(|error| error.to_string())?;
        assert_eq!(
            bodies[0]["messages"][0]["thinking"],
            Value::String("previous reasoning".to_owned())
        );
        Ok(())
    }
}
