use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

macro_rules! string_enum {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value),+
                }
            }

            pub(super) fn parse(value: &str) -> crate::Result<Self> {
                match value {
                    $($value => Ok(Self::$variant)),+,
                    _ => Err(crate::PersistenceError::InvalidValue {
                        field: stringify!($name),
                        value: value.to_owned(),
                    }),
                }
            }
        }
    };
}

string_enum!(WorkflowRunStatus {
    Queued => "queued",
    Running => "running",
    Waiting => "waiting",
    Interrupted => "interrupted",
    Completed => "completed",
    Failed => "failed",
    Cancelled => "cancelled",
    VersionMismatch => "version-mismatch",
});

string_enum!(WorkflowPresentation {
    Direct => "direct",
    Agent => "agent",
});

string_enum!(WorkflowStepKind {
    Checkpoint => "checkpoint",
    Effect => "effect",
    Human => "human",
});

string_enum!(WorkflowStepState {
    Started => "started",
    Completed => "completed",
});

string_enum!(ScheduleStatus {
    Active => "active",
    Paused => "paused",
    NeedsReauthorization => "needs-reauthorization",
});

string_enum!(ScheduleOccurrenceStatus {
    Pending => "pending",
    Running => "running",
    Completed => "completed",
    Failed => "failed",
    Waiting => "waiting",
    Skipped => "skipped",
    Invalidated => "invalidated",
});

string_enum!(ScheduleNotificationKind {
    Completed => "completed",
    Failed => "failed",
    Waiting => "waiting",
    Invalidated => "invalidated",
});

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum WorkflowTrigger {
    Manual,
    Schedule {
        #[serde(rename = "scheduleId")]
        schedule_id: String,
        #[serde(rename = "scheduledFor")]
        scheduled_for: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateWorkflowRun {
    pub id: String,
    pub workflow_name: String,
    pub project_dir: String,
    pub agent_name: Option<String>,
    pub trigger: Option<WorkflowTrigger>,
    pub source_entry_path: String,
    pub source_fingerprint: String,
    pub presentation: WorkflowPresentation,
    pub input: Value,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowRunSummary {
    pub id: String,
    pub workflow_name: String,
    pub project_dir: String,
    pub agent_name: String,
    pub trigger: WorkflowTrigger,
    pub status: WorkflowRunStatus,
    pub presentation: WorkflowPresentation,
    pub created_at: String,
    pub updated_at: String,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowRunDetails {
    pub summary: WorkflowRunSummary,
    pub input: Value,
    pub output: Option<Value>,
    pub source_entry_path: String,
    pub source_fingerprint: String,
    pub parent_run_id: Option<String>,
    pub depth: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewWorkflowStep {
    pub run_id: String,
    pub key: String,
    pub kind: WorkflowStepKind,
    pub input: Option<Value>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowStep {
    pub run_id: String,
    pub key: String,
    pub kind: WorkflowStepKind,
    pub state: WorkflowStepState,
    pub input: Option<Value>,
    pub output: Option<Value>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectRecord {
    pub key: String,
    pub idempotency_key: String,
    pub state: WorkflowStepState,
    pub output: Option<Value>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HumanResponseRecord {
    pub key: String,
    pub prompt: Value,
    pub state: WorkflowStepState,
    pub response: Option<Value>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateSchedule {
    pub id: Option<String>,
    pub project_dir: String,
    pub agent_name: String,
    pub workflow_name: String,
    pub input: Value,
    pub cron: String,
    pub timezone: String,
    pub package_fingerprint: String,
    pub next_run_at: String,
    pub now: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleRecord {
    pub id: String,
    pub project_dir: String,
    pub agent_name: String,
    pub workflow_name: String,
    pub input: Value,
    pub cron: String,
    pub timezone: String,
    pub package_fingerprint: String,
    pub status: ScheduleStatus,
    pub next_run_at: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleOccurrence {
    pub id: String,
    pub schedule_id: String,
    pub scheduled_for: String,
    pub status: ScheduleOccurrenceStatus,
    pub run_id: Option<String>,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OccurrenceUpdate {
    pub run_id: Option<String>,
    pub result: Option<Value>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleNotification {
    pub id: String,
    pub project_dir: String,
    pub agent_name: String,
    pub schedule_id: Option<String>,
    pub occurrence_id: Option<String>,
    pub kind: ScheduleNotificationKind,
    pub message: String,
    pub read: bool,
    pub created_at: String,
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
pub struct StoredToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Map<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredChatMessage {
    pub role: ChatRole,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<StoredToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentSessionRecord {
    pub id: String,
    pub project_dir: String,
    pub agent_name: String,
    pub provider: String,
    pub model: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredAgentConversation {
    pub session: AgentSessionRecord,
    pub history: Vec<StoredChatMessage>,
}
