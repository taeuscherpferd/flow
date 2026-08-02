use std::fmt::Debug;

use async_trait::async_trait;
use http::StatusCode;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpResponse {
    pub status: StatusCode,
    pub body: String,
}

#[derive(Debug, thiserror::Error)]
pub enum HttpTransportError {
    #[error("{0}")]
    Request(String),
    #[error("request was cancelled")]
    Cancelled,
}

#[async_trait]
pub trait HttpTransport: Debug + Send + Sync {
    async fn post_json(
        &self,
        url: &str,
        bearer_token: Option<&str>,
        body: Value,
        cancellation: &CancellationToken,
    ) -> Result<HttpResponse, HttpTransportError>;
}

#[derive(Debug)]
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl Default for ReqwestTransport {
    fn default() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl HttpTransport for ReqwestTransport {
    async fn post_json(
        &self,
        url: &str,
        bearer_token: Option<&str>,
        body: Value,
        cancellation: &CancellationToken,
    ) -> Result<HttpResponse, HttpTransportError> {
        let mut request = self.client.post(url).json(&body);
        if let Some(token) = bearer_token {
            request = request.bearer_auth(token);
        }
        let response = tokio::select! {
            () = cancellation.cancelled() => return Err(HttpTransportError::Cancelled),
            response = request.send() => response
                .map_err(|error| HttpTransportError::Request(error.to_string()))?,
        };
        let status = response.status();
        let body = tokio::select! {
            () = cancellation.cancelled() => return Err(HttpTransportError::Cancelled),
            body = response.text() => body
                .map_err(|error| HttpTransportError::Request(error.to_string()))?,
        };
        Ok(HttpResponse { status, body })
    }
}
