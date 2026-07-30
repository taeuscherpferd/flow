use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ApplicationCommand {
    SendMessage { text: String },
    RunWorkflow { name: String, input: Value },
    ResumeRun { run_id: Uuid },
    CancelRun { run_id: Uuid },
    SwitchAgent { name: String },
    SwitchModel { model: String },
    ClearConversation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ApplicationQuery {
    Models,
    Agents,
    Workflows,
    Runs { project_dir: PathBuf },
    Schedules { project_dir: PathBuf },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryResult {
    pub items: Vec<Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ApplicationEvent {
    Status {
        run_id: Option<Uuid>,
        message: String,
    },
    Log {
        run_id: Uuid,
        message: String,
    },
    Output {
        run_id: Option<Uuid>,
        value: Value,
    },
    PermissionRequested {
        request_id: Uuid,
        tool_name: String,
        arguments: Value,
    },
    HumanRequested {
        request_id: Uuid,
        run_id: Uuid,
        prompt: String,
        details: Option<String>,
    },
    RunChanged {
        run_id: Uuid,
        status: String,
    },
    Error {
        message: String,
    },
}

#[derive(Debug)]
pub struct ApplicationFacade {
    events: broadcast::Sender<ApplicationEvent>,
    cancellations: Arc<Mutex<HashMap<Uuid, CancellationToken>>>,
}

impl Default for ApplicationFacade {
    fn default() -> Self {
        Self::new()
    }
}

impl ApplicationFacade {
    #[must_use]
    pub fn new() -> Self {
        let (events, _) = broadcast::channel(256);
        Self {
            events,
            cancellations: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<ApplicationEvent> {
        self.events.subscribe()
    }

    pub fn emit(&self, event: ApplicationEvent) {
        let _subscriber_count = self.events.send(event);
    }

    pub fn begin_operation(&self, run_id: Uuid) -> CancellationToken {
        let token = CancellationToken::new();
        if let Ok(mut cancellations) = self.cancellations.lock() {
            cancellations.insert(run_id, token.clone());
        }
        token
    }

    pub fn finish_operation(&self, run_id: Uuid) {
        if let Ok(mut cancellations) = self.cancellations.lock() {
            cancellations.remove(&run_id);
        }
    }

    #[must_use]
    pub fn cancel(&self, run_id: Uuid) -> bool {
        let Ok(cancellations) = self.cancellations.lock() else {
            return false;
        };
        let Some(token) = cancellations.get(&run_id) else {
            return false;
        };
        token.cancel();
        true
    }
}
