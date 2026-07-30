use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use flowmation_application::workflow::DurableStepKind;
use flowmation_application::{DurableRun, DurableStep, WorkflowDurability, WorkflowRecord};
use flowmation_sqlite::{
    CreateWorkflowRun, OccurrenceUpdate, ScheduleOccurrenceStatus, SqliteApplicationRepository,
    SqliteDatabase, WorkflowPresentation, WorkflowTrigger,
};
use flowmation_workflow_host::protocol::WorkflowPresentation as HostPresentation;
use serde_json::Value;

pub(super) struct ScheduledDurability {
    repository: Arc<SqliteApplicationRepository>,
    schedule_id: String,
    occurrence_id: String,
    scheduled_for: String,
    run_id: Mutex<Option<String>>,
    global_dir: PathBuf,
}

impl ScheduledDurability {
    pub(super) fn new(
        repository: Arc<SqliteApplicationRepository>,
        schedule_id: &str,
        occurrence_id: &str,
        scheduled_for: &str,
        run_id: Option<String>,
        global_dir: &Path,
    ) -> Self {
        Self {
            repository,
            schedule_id: schedule_id.to_owned(),
            occurrence_id: occurrence_id.to_owned(),
            scheduled_for: scheduled_for.to_owned(),
            run_id: Mutex::new(run_id),
            global_dir: global_dir.to_path_buf(),
        }
    }

    pub(super) fn run_id(&self) -> Result<String, String> {
        self.run_id
            .lock()
            .map_err(|error| error.to_string())?
            .clone()
            .ok_or_else(|| "Scheduled workflow did not create a run ID.".to_owned())
    }
}

#[async_trait]
impl WorkflowDurability for ScheduledDurability {
    async fn create_run(
        &self,
        run_id: &str,
        record: &WorkflowRecord,
        project_dir: &Path,
        input: &Value,
    ) -> Result<(), String> {
        let mut database =
            SqliteDatabase::open_global_dir(&self.global_dir).map_err(|error| error.to_string())?;
        database
            .occurrences()
            .update(
                &self.occurrence_id,
                ScheduleOccurrenceStatus::Running,
                &OccurrenceUpdate {
                    run_id: Some(run_id.to_owned()),
                    result: None,
                    error: None,
                },
            )
            .map_err(|error| error.to_string())?;
        *self.run_id.lock().map_err(|error| error.to_string())? = Some(run_id.to_owned());
        database
            .workflow_runs()
            .create(&CreateWorkflowRun {
                id: run_id.to_owned(),
                workflow_name: record.metadata.name.clone(),
                project_dir: path_text(project_dir)?,
                agent_name: record.agent_name.clone(),
                trigger: Some(WorkflowTrigger::Schedule {
                    schedule_id: self.schedule_id.clone(),
                    scheduled_for: self.scheduled_for.clone(),
                }),
                source_entry_path: path_text(&record.entry_path)?,
                source_fingerprint: record.fingerprint.clone(),
                presentation: persistence_presentation(record.metadata.presentation),
                input: input.clone(),
            })
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    async fn load_run(&self, run_id: &str) -> Result<Option<DurableRun>, String> {
        WorkflowDurability::load_run(self.repository.as_ref(), run_id).await
    }

    async fn mark_running(&self, run_id: &str) -> Result<(), String> {
        WorkflowDurability::mark_running(self.repository.as_ref(), run_id).await
    }

    async fn complete_run(
        &self,
        run_id: &str,
        output: &Value,
        presentation: HostPresentation,
    ) -> Result<(), String> {
        WorkflowDurability::complete_run(self.repository.as_ref(), run_id, output, presentation)
            .await
    }

    async fn mark_run(
        &self,
        run_id: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), String> {
        WorkflowDurability::mark_run(self.repository.as_ref(), run_id, status, error).await
    }

    async fn step(&self, run_id: &str, key: &str) -> Result<Option<DurableStep>, String> {
        WorkflowDurability::step(self.repository.as_ref(), run_id, key).await
    }

    async fn start_step(
        &self,
        run_id: &str,
        key: &str,
        kind: DurableStepKind,
        input: Option<&Value>,
    ) -> Result<(), String> {
        WorkflowDurability::start_step(self.repository.as_ref(), run_id, key, kind, input).await
    }

    async fn complete_step(&self, run_id: &str, key: &str, output: &Value) -> Result<(), String> {
        WorkflowDurability::complete_step(self.repository.as_ref(), run_id, key, output).await
    }
}

const fn persistence_presentation(value: HostPresentation) -> WorkflowPresentation {
    match value {
        HostPresentation::Direct => WorkflowPresentation::Direct,
        HostPresentation::Agent => WorkflowPresentation::Agent,
    }
}

fn path_text(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("Path is not valid UTF-8: {}", path.display()))
}
