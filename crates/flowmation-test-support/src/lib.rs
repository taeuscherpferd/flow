use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use flowmation_application::{
    ChatCompletionRequest, ChatCompletionResult, ModelProvider, ProviderError,
};
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
pub struct RecordingProvider {
    id: String,
    requests: Mutex<Vec<ChatCompletionRequest>>,
    responses: Mutex<VecDeque<Result<ChatCompletionResult, ProviderError>>>,
}

impl RecordingProvider {
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        responses: impl IntoIterator<Item = Result<ChatCompletionResult, ProviderError>>,
    ) -> Self {
        Self {
            id: id.into(),
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(responses.into_iter().collect()),
        }
    }

    pub fn requests(&self) -> Result<Vec<ChatCompletionRequest>, String> {
        self.requests
            .lock()
            .map(|requests| requests.clone())
            .map_err(|error| error.to_string())
    }
}

#[async_trait]
impl ModelProvider for RecordingProvider {
    fn id(&self) -> &str {
        &self.id
    }

    async fn chat(
        &self,
        request: ChatCompletionRequest,
        cancellation: &CancellationToken,
    ) -> Result<ChatCompletionResult, ProviderError> {
        if cancellation.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        self.requests
            .lock()
            .map_err(|error| ProviderError::Unavailable(error.to_string()))?
            .push(request);
        self.responses
            .lock()
            .map_err(|error| ProviderError::Unavailable(error.to_string()))?
            .pop_front()
            .unwrap_or_else(|| {
                Err(ProviderError::InvalidResponse(
                    "recording provider has no queued response".to_owned(),
                ))
            })
    }
}

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Debug)]
pub struct FixedClock {
    now: Mutex<DateTime<Utc>>,
}

impl FixedClock {
    #[must_use]
    pub fn new(now: DateTime<Utc>) -> Self {
        Self {
            now: Mutex::new(now),
        }
    }

    pub fn set(&self, now: DateTime<Utc>) -> Result<(), String> {
        self.now
            .lock()
            .map(|mut current| *current = now)
            .map_err(|error| error.to_string())
    }
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.now
            .lock()
            .map(|now| *now)
            .unwrap_or_else(|poisoned| *poisoned.into_inner())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessInvocation {
    pub program: PathBuf,
    pub arguments: Vec<String>,
    pub cwd: PathBuf,
}

#[derive(Debug, Default)]
pub struct RecordingProcessLauncher {
    invocations: Mutex<Vec<ProcessInvocation>>,
}

impl RecordingProcessLauncher {
    pub fn launch(
        &self,
        program: impl AsRef<Path>,
        arguments: Vec<String>,
        cwd: impl AsRef<Path>,
    ) -> Result<(), String> {
        self.invocations
            .lock()
            .map_err(|error| error.to_string())?
            .push(ProcessInvocation {
                program: program.as_ref().to_path_buf(),
                arguments,
                cwd: cwd.as_ref().to_path_buf(),
            });
        Ok(())
    }

    pub fn invocations(&self) -> Result<Vec<ProcessInvocation>, String> {
        self.invocations
            .lock()
            .map(|invocations| invocations.clone())
            .map_err(|error| error.to_string())
    }
}
