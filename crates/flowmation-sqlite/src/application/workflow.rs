use std::path::Path;

use async_trait::async_trait;
use flowmation_application::workflow::{
    DurableRun, DurableRunStatus, DurableStep, DurableStepKind, WorkflowDurability, WorkflowRecord,
};
use flowmation_workflow_host::protocol::WorkflowPresentation as HostPresentation;

use super::SqliteApplicationRepository;
use crate::{
    CreateWorkflowRun, NewWorkflowStep, WorkflowPresentation, WorkflowRunStatus, WorkflowStepKind,
    WorkflowStepState,
};

#[async_trait]
impl WorkflowDurability for SqliteApplicationRepository {
    async fn create_run(
        &self,
        run_id: &str,
        record: &WorkflowRecord,
        project_dir: &Path,
        input: &serde_json::Value,
    ) -> Result<(), String> {
        let create = CreateWorkflowRun {
            id: run_id.to_owned(),
            workflow_name: record.metadata.name.clone(),
            project_dir: path_text(project_dir)?,
            agent_name: record.agent_name.clone(),
            trigger: None,
            source_entry_path: path_text(&record.entry_path)?,
            source_fingerprint: record.fingerprint.clone(),
            presentation: presentation(record.metadata.presentation),
            input: input.clone(),
        };
        self.database()?
            .workflow_runs()
            .create(&create)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    async fn mark_running(&self, run_id: &str) -> Result<(), String> {
        let changed = self
            .database()?
            .workflow_runs()
            .transition_to_running(run_id)
            .map_err(|error| error.to_string())?;
        require_changed(changed, "Workflow run could not transition to running.")
    }

    async fn load_run(&self, run_id: &str) -> Result<Option<DurableRun>, String> {
        self.database()?
            .workflow_runs()
            .get(run_id)
            .map_err(|error| error.to_string())
            .map(|run| {
                run.map(|run| DurableRun {
                    workflow_name: run.summary.workflow_name,
                    project_dir: run.summary.project_dir.into(),
                    source_entry_path: run.source_entry_path.into(),
                    source_fingerprint: run.source_fingerprint,
                    status: durable_status(run.summary.status),
                    input: run.input,
                    output: run.output,
                })
            })
    }

    async fn complete_run(
        &self,
        run_id: &str,
        output: &serde_json::Value,
        output_presentation: HostPresentation,
    ) -> Result<(), String> {
        let changed = self
            .database()?
            .workflow_runs()
            .complete(run_id, output, presentation(output_presentation))
            .map_err(|error| error.to_string())?;
        require_changed(changed, "Workflow run could not be completed.")
    }

    async fn mark_run(
        &self,
        run_id: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), String> {
        let status = run_status(status)?;
        let changed = self
            .database()?
            .workflow_runs()
            .transition_running_status(run_id, status, error)
            .map_err(|database_error| database_error.to_string())?;
        require_changed(changed, "Workflow run status could not be updated.")
    }

    async fn step(&self, run_id: &str, key: &str) -> Result<Option<DurableStep>, String> {
        self.database()?
            .workflow_steps()
            .get(run_id, key)
            .map_err(|error| error.to_string())
            .map(|step| {
                step.map(|step| DurableStep {
                    kind: match step.kind {
                        WorkflowStepKind::Checkpoint => DurableStepKind::Checkpoint,
                        WorkflowStepKind::Effect => DurableStepKind::Effect,
                        WorkflowStepKind::Human => DurableStepKind::Human,
                    },
                    input: step.input,
                    output: step.output,
                    completed: step.state == WorkflowStepState::Completed,
                })
            })
    }

    async fn start_step(
        &self,
        run_id: &str,
        key: &str,
        kind: DurableStepKind,
        input: Option<&serde_json::Value>,
    ) -> Result<(), String> {
        self.database()?
            .workflow_steps()
            .start(&NewWorkflowStep {
                run_id: run_id.to_owned(),
                key: key.to_owned(),
                kind: match kind {
                    DurableStepKind::Checkpoint => WorkflowStepKind::Checkpoint,
                    DurableStepKind::Effect => WorkflowStepKind::Effect,
                    DurableStepKind::Human => WorkflowStepKind::Human,
                },
                input: input.cloned(),
            })
            .map_err(|error| error.to_string())
    }

    async fn complete_step(
        &self,
        run_id: &str,
        key: &str,
        output: &serde_json::Value,
    ) -> Result<(), String> {
        self.database()?
            .workflow_steps()
            .complete(run_id, key, output)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

const fn presentation(value: HostPresentation) -> WorkflowPresentation {
    match value {
        HostPresentation::Direct => WorkflowPresentation::Direct,
        HostPresentation::Agent => WorkflowPresentation::Agent,
    }
}

const fn durable_status(value: WorkflowRunStatus) -> DurableRunStatus {
    match value {
        WorkflowRunStatus::Queued => DurableRunStatus::Queued,
        WorkflowRunStatus::Running => DurableRunStatus::Running,
        WorkflowRunStatus::Waiting => DurableRunStatus::Waiting,
        WorkflowRunStatus::Interrupted => DurableRunStatus::Interrupted,
        WorkflowRunStatus::Completed => DurableRunStatus::Completed,
        WorkflowRunStatus::Failed => DurableRunStatus::Failed,
        WorkflowRunStatus::Cancelled => DurableRunStatus::Cancelled,
        WorkflowRunStatus::VersionMismatch => DurableRunStatus::VersionMismatch,
    }
}

fn run_status(value: &str) -> Result<WorkflowRunStatus, String> {
    match value {
        "queued" => Ok(WorkflowRunStatus::Queued),
        "running" => Ok(WorkflowRunStatus::Running),
        "waiting" => Ok(WorkflowRunStatus::Waiting),
        "interrupted" => Ok(WorkflowRunStatus::Interrupted),
        "completed" => Ok(WorkflowRunStatus::Completed),
        "failed" => Ok(WorkflowRunStatus::Failed),
        "cancelled" => Ok(WorkflowRunStatus::Cancelled),
        "version-mismatch" => Ok(WorkflowRunStatus::VersionMismatch),
        _ => Err(format!("Unknown workflow run status \"{value}\".")),
    }
}

fn require_changed(changed: bool, message: &str) -> Result<(), String> {
    if changed {
        Ok(())
    } else {
        Err(message.to_owned())
    }
}

fn path_text(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("Path is not valid UTF-8: {}", path.display()))
}
