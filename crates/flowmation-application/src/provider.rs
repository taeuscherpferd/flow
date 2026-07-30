use std::fmt::Debug;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

pub use flowmation_domain::chat::{ChatMessage, ChatRole, ThinkingMode, ToolCall, ToolDefinition};

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatCompletionOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_ctx: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingMode>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinition>,
    #[serde(default)]
    pub options: ChatCompletionOptions,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ChatCompletionResult {
    pub message: ChatMessage,
}

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("provider request was cancelled")]
    Cancelled,
    #[error("{0}")]
    Unavailable(String),
    #[error("{0}")]
    InvalidResponse(String),
}

#[async_trait]
pub trait ModelProvider: Debug + Send + Sync {
    fn id(&self) -> &str;

    async fn chat(
        &self,
        request: ChatCompletionRequest,
        cancellation: &CancellationToken,
    ) -> Result<ChatCompletionResult, ProviderError>;
}
