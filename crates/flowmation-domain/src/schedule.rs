use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::JsonValue;
use crate::ids::{ScheduleId, ScheduleNotificationId, ScheduleOccurrenceId, WorkflowRunId};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScheduleStatus {
    Active,
    Paused,
    NeedsReauthorization,
}

impl ScheduleStatus {
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Active, Self::Paused | Self::NeedsReauthorization)
                | (Self::Paused, Self::Active | Self::NeedsReauthorization)
                | (Self::NeedsReauthorization, Self::Active)
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScheduleOccurrenceStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Waiting,
    Skipped,
    Invalidated,
}

impl ScheduleOccurrenceStatus {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Skipped | Self::Invalidated
        )
    }

    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (
                Self::Pending,
                Self::Running | Self::Skipped | Self::Invalidated
            ) | (
                Self::Running,
                Self::Completed | Self::Failed | Self::Waiting | Self::Invalidated
            ) | (
                Self::Waiting,
                Self::Completed | Self::Failed | Self::Invalidated
            )
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScheduleNotificationKind {
    Completed,
    Failed,
    Waiting,
    Invalidated,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleRecord {
    pub id: ScheduleId,
    pub project_dir: PathBuf,
    pub agent_name: String,
    pub workflow_name: String,
    pub input: JsonValue,
    pub cron: String,
    pub timezone: String,
    pub package_fingerprint: String,
    pub status: ScheduleStatus,
    pub next_run_at: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleOccurrence {
    pub id: ScheduleOccurrenceId,
    pub schedule_id: ScheduleId,
    pub scheduled_for: String,
    pub status: ScheduleOccurrenceStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<WorkflowRunId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<JsonValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleNotification {
    pub id: ScheduleNotificationId,
    pub project_dir: PathBuf,
    pub agent_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_id: Option<ScheduleId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence_id: Option<ScheduleOccurrenceId>,
    pub kind: ScheduleNotificationKind,
    pub message: String,
    pub read: bool,
    pub created_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateScheduleInput {
    pub project_dir: PathBuf,
    pub agent_name: String,
    pub workflow_name: String,
    pub input: JsonValue,
    pub cron: String,
    pub timezone: String,
    pub package_fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub now: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{ScheduleOccurrenceStatus, ScheduleStatus};

    #[test]
    fn schedule_statuses_use_legacy_kebab_case() -> Result<(), Box<dyn std::error::Error>> {
        let serialized = serde_json::to_string(&ScheduleStatus::NeedsReauthorization)?;

        assert_eq!(serialized, "\"needs-reauthorization\"");
        Ok(())
    }

    #[test]
    fn occurrence_state_machine_allows_waiting_completion() {
        assert!(
            ScheduleOccurrenceStatus::Pending.can_transition_to(ScheduleOccurrenceStatus::Running)
        );
        assert!(
            ScheduleOccurrenceStatus::Running.can_transition_to(ScheduleOccurrenceStatus::Waiting)
        );
        assert!(
            ScheduleOccurrenceStatus::Waiting
                .can_transition_to(ScheduleOccurrenceStatus::Completed)
        );
        assert!(
            !ScheduleOccurrenceStatus::Completed
                .can_transition_to(ScheduleOccurrenceStatus::Running)
        );
    }
}
