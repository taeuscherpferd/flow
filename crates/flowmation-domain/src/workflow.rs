use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::JsonValue;
use crate::agent::{PackageSource, is_kebab_case_name};
use crate::chat::ThinkingMode;
use crate::ids::{ScheduleId, WorkflowRunId};
use crate::schema::WorkflowSchema;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowPresentation {
    #[default]
    Direct,
    Agent,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentInvocationPolicy {
    #[default]
    Disabled,
    Confirm,
    Automatic,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct WorkflowInputDefinition {
    pub schema: WorkflowSchema,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDefinitionMetadata {
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<WorkflowInputDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_invocation: Option<AgentInvocationPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation: Option<WorkflowPresentation>,
}

impl WorkflowDefinitionMetadata {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        is_kebab_case_name(&self.name)
            && !self.description.trim().is_empty()
            && self
                .input
                .as_ref()
                .is_none_or(|input| input.schema.is_valid_root())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRecord {
    pub definition: WorkflowDefinitionMetadata,
    pub directory: PathBuf,
    pub entry_path: PathBuf,
    pub fingerprint: String,
    pub source: PackageSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowOutputValue {
    pub kind: WorkflowOutputKind,
    pub presentation: WorkflowPresentation,
    pub value: JsonValue,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkflowOutputKind {
    WorkflowOutput,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkflowRunStatus {
    Queued,
    Running,
    Waiting,
    Interrupted,
    Completed,
    Failed,
    Cancelled,
    VersionMismatch,
}

impl WorkflowRunStatus {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::VersionMismatch
        )
    }

    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (
                Self::Queued | Self::Waiting | Self::Interrupted,
                Self::Running | Self::Cancelled | Self::VersionMismatch
            ) | (
                Self::Running,
                Self::Waiting
                    | Self::Interrupted
                    | Self::Completed
                    | Self::Failed
                    | Self::Cancelled
                    | Self::VersionMismatch
            )
        )
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum WorkflowTrigger {
    #[default]
    Manual,
    Schedule {
        #[serde(rename = "scheduleId")]
        schedule_id: ScheduleId,
        #[serde(rename = "scheduledFor")]
        scheduled_for: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowStepKind {
    Checkpoint,
    Effect,
    Human,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowStepState {
    Started,
    Completed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStep {
    pub key: String,
    pub kind: WorkflowStepKind,
    pub state: WorkflowStepState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<JsonValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<JsonValue>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunSummary {
    pub id: WorkflowRunId,
    pub workflow_name: String,
    pub project_dir: PathBuf,
    pub agent_name: String,
    pub trigger: WorkflowTrigger,
    pub status: WorkflowRunStatus,
    pub presentation: WorkflowPresentation,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunDetails {
    #[serde(flatten)]
    pub summary: WorkflowRunSummary,
    pub input: JsonValue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<JsonValue>,
    pub source_fingerprint: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkflowRun {
    pub id: WorkflowRunId,
    pub workflow_name: String,
    pub project_dir: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<WorkflowTrigger>,
    pub source_entry_path: PathBuf,
    pub source_fingerprint: String,
    pub presentation: WorkflowPresentation,
    pub input: JsonValue,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowInvocationResult {
    pub run: WorkflowRunDetails,
    pub presentation: WorkflowPresentation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<JsonValue>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRef {
    pub provider: String,
    pub model: String,
    pub active: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowAgentTools {
    Default,
    None,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowAgentRunOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<WorkflowAgentTools>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingMode>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HumanChoice {
    pub value: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowHumanPromptKind {
    Approval,
    Choice,
    Text,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowHumanPrompt {
    pub kind: WorkflowHumanPromptKind,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<HumanChoice>>,
}

#[cfg(test)]
mod tests {
    use super::{WorkflowRunStatus, WorkflowTrigger};

    #[test]
    fn workflow_trigger_matches_legacy_tagged_json() -> Result<(), Box<dyn std::error::Error>> {
        let manual = serde_json::to_string(&WorkflowTrigger::Manual)?;
        let schedule: WorkflowTrigger = serde_json::from_str(
            r#"{"type":"schedule","scheduleId":"schedule-1","scheduledFor":"2026-07-29T12:00:00.000Z"}"#,
        )?;

        assert_eq!(manual, r#"{"type":"manual"}"#);
        assert!(matches!(schedule, WorkflowTrigger::Schedule { .. }));
        Ok(())
    }

    #[test]
    fn workflow_status_state_machine_keeps_terminal_states_terminal() {
        assert!(WorkflowRunStatus::Queued.can_transition_to(WorkflowRunStatus::Running));
        assert!(WorkflowRunStatus::Running.can_transition_to(WorkflowRunStatus::Waiting));
        assert!(WorkflowRunStatus::Waiting.can_transition_to(WorkflowRunStatus::Running));
        assert!(!WorkflowRunStatus::Completed.can_transition_to(WorkflowRunStatus::Running));
        assert!(WorkflowRunStatus::VersionMismatch.is_terminal());
    }
}
