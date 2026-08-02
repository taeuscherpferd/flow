use std::fmt::{Debug, Formatter};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use flowmation_application::{
    ChatCompletionOptions, ChatCompletionRequest, ChatMessage, ChatRole, ModelProvider,
    ProviderError, ThinkingMode, ToolCall,
};
use flowmation_domain::config::CredentialSource;
use flowmation_http::{HttpResponse, HttpTransport, HttpTransportError};
use flowmation_openai_compatible::{OpenAiCompatibleProvider, SecretResolveError, SecretResolver};
use http::StatusCode;
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

struct FixedSecretResolver {
    token: String,
}

impl Debug for FixedSecretResolver {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FixedSecretResolver")
            .finish_non_exhaustive()
    }
}

impl SecretResolver for FixedSecretResolver {
    fn resolve(&self, _source: &CredentialSource) -> Result<String, SecretResolveError> {
        Ok(self.token.clone())
    }
}

#[derive(Debug)]
struct MissingSecretResolver;

impl SecretResolver for MissingSecretResolver {
    fn resolve(&self, source: &CredentialSource) -> Result<String, SecretResolveError> {
        let CredentialSource::Environment { name } = source;
        Err(SecretResolveError::MissingEnvironment { name: name.clone() })
    }
}

#[derive(Clone, Debug, PartialEq)]
struct RecordedRequest {
    url: String,
    bearer_token: Option<String>,
    body: Value,
}

#[derive(Debug)]
struct RecordingTransport {
    requests: Mutex<Vec<RecordedRequest>>,
    response: HttpResponse,
}

#[async_trait]
impl HttpTransport for RecordingTransport {
    async fn post_json(
        &self,
        url: &str,
        bearer_token: Option<&str>,
        body: Value,
        _cancellation: &CancellationToken,
    ) -> Result<HttpResponse, HttpTransportError> {
        self.requests
            .lock()
            .map_err(|error| HttpTransportError::Request(error.to_string()))?
            .push(RecordedRequest {
                url: url.to_owned(),
                bearer_token: bearer_token.map(str::to_owned),
                body,
            });
        Ok(self.response.clone())
    }
}

#[derive(Debug)]
struct CancelledTransport;

#[async_trait]
impl HttpTransport for CancelledTransport {
    async fn post_json(
        &self,
        _url: &str,
        _bearer_token: Option<&str>,
        _body: Value,
        _cancellation: &CancellationToken,
    ) -> Result<HttpResponse, HttpTransportError> {
        Err(HttpTransportError::Cancelled)
    }
}

fn token_source() -> Option<CredentialSource> {
    Some(CredentialSource::Environment {
        name: "TEST_API_KEY".to_owned(),
    })
}

fn request() -> ChatCompletionRequest {
    let mut assistant = ChatMessage::new(ChatRole::Assistant, "");
    assistant.tool_calls.push(ToolCall {
        id: "call_old".to_owned(),
        name: "read_file".to_owned(),
        arguments: Map::from_iter([("path".to_owned(), json!("README.md"))]),
    });
    let mut tool = ChatMessage::new(ChatRole::Tool, "contents");
    tool.tool_call_id = Some("call_old".to_owned());
    ChatCompletionRequest {
        model: "example-model".to_owned(),
        messages: vec![assistant, tool],
        tools: Vec::new(),
        options: ChatCompletionOptions {
            num_ctx: Some(128_000),
            thinking: Some(ThinkingMode::High),
        },
    }
}

fn provider_with_response(body: impl Into<String>) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::with_dependencies(
        "local-compatible",
        "http://localhost:8000/v1",
        None,
        Arc::new(MissingSecretResolver),
        Arc::new(RecordingTransport {
            requests: Mutex::new(Vec::new()),
            response: HttpResponse {
                status: StatusCode::OK,
                body: body.into(),
            },
        }),
    )
}

#[tokio::test]
async fn sends_bearer_auth_and_maps_tool_calls() -> Result<(), Box<dyn std::error::Error>> {
    let transport = Arc::new(RecordingTransport {
        requests: Mutex::new(Vec::new()),
        response: HttpResponse {
            status: StatusCode::OK,
            body: r#"{
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": "call_new",
                            "type": "function",
                            "function": {
                                "name": "write_file",
                                "arguments": "{\"path\":\"result.txt\"}"
                            }
                        }]
                    }
                }]
            }"#
            .to_owned(),
        },
    });
    let provider = OpenAiCompatibleProvider::with_dependencies(
        "openrouter",
        "https://example.test/v1/",
        token_source(),
        Arc::new(FixedSecretResolver {
            token: "test-secret".to_owned(),
        }),
        transport.clone(),
    );

    let response = provider.chat(request(), &CancellationToken::new()).await?;

    assert_eq!(provider.id(), "openrouter");
    assert_eq!(response.message.tool_calls[0].id, "call_new");
    assert_eq!(
        response.message.tool_calls[0].arguments["path"],
        "result.txt"
    );
    let requests = transport
        .requests
        .lock()
        .map_err(|error| error.to_string())?;
    assert_eq!(requests[0].url, "https://example.test/v1/chat/completions");
    assert_eq!(requests[0].bearer_token.as_deref(), Some("test-secret"));
    assert_eq!(requests[0].body["reasoning_effort"], "high");
    assert_eq!(
        requests[0].body["messages"][0]["tool_calls"][0]["id"],
        "call_old"
    );
    assert_eq!(requests[0].body["messages"][1]["tool_call_id"], "call_old");
    assert!(requests[0].body.get("num_ctx").is_none());
    Ok(())
}

#[tokio::test]
async fn redacts_tokens_from_endpoint_errors() -> Result<(), Box<dyn std::error::Error>> {
    let provider = OpenAiCompatibleProvider::with_dependencies(
        "openai-api",
        "https://example.test/v1",
        token_source(),
        Arc::new(FixedSecretResolver {
            token: "test-secret".to_owned(),
        }),
        Arc::new(RecordingTransport {
            requests: Mutex::new(Vec::new()),
            response: HttpResponse {
                status: StatusCode::UNAUTHORIZED,
                body: "rejected test-secret".to_owned(),
            },
        }),
    );

    let Err(error) = provider.chat(request(), &CancellationToken::new()).await else {
        return Err("request unexpectedly succeeded".into());
    };
    let message = error.to_string();
    assert!(message.contains("[redacted]"));
    assert!(!message.contains("test-secret"));
    Ok(())
}

#[tokio::test]
async fn reports_missing_environment_credentials_before_requesting()
-> Result<(), Box<dyn std::error::Error>> {
    let provider = OpenAiCompatibleProvider::with_dependencies(
        "openai-api",
        "https://example.test/v1",
        token_source(),
        Arc::new(MissingSecretResolver),
        Arc::new(CancelledTransport),
    );

    let Err(error) = provider.chat(request(), &CancellationToken::new()).await else {
        return Err("request unexpectedly succeeded".into());
    };
    assert!(error.to_string().contains("TEST_API_KEY"));
    Ok(())
}

#[tokio::test]
async fn maps_transport_cancellation() -> Result<(), Box<dyn std::error::Error>> {
    let provider = OpenAiCompatibleProvider::with_dependencies(
        "local-compatible",
        "http://localhost:8000/v1",
        None,
        Arc::new(MissingSecretResolver),
        Arc::new(CancelledTransport),
    );

    let result = provider.chat(request(), &CancellationToken::new()).await;
    assert!(matches!(result, Err(ProviderError::Cancelled)));
    Ok(())
}

#[tokio::test]
async fn accepts_null_tool_calls_on_text_responses() -> Result<(), Box<dyn std::error::Error>> {
    let provider = provider_with_response(
        r#"{
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "Ready.",
                    "tool_calls": null
                }
            }]
        }"#,
    );

    let response = provider.chat(request(), &CancellationToken::new()).await?;

    assert_eq!(response.message.content, "Ready.");
    assert!(response.message.tool_calls.is_empty());
    Ok(())
}

#[tokio::test]
async fn surfaces_refusals_when_content_is_null() -> Result<(), Box<dyn std::error::Error>> {
    let provider = provider_with_response(
        r#"{
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "refusal": "I cannot help with that request."
                }
            }]
        }"#,
    );

    let response = provider.chat(request(), &CancellationToken::new()).await?;

    assert_eq!(response.message.content, "I cannot help with that request.");
    Ok(())
}
