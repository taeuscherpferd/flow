use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use flowmation_application::scheduling::{
    ScheduleExecution, ScheduleRepository, ScheduleWorker, WorkerExecutionResult,
};
use flowmation_domain::ids::WorkflowRunId;
use flowmation_domain::schedule::{
    CreateScheduleInput, ScheduleOccurrence, ScheduleOccurrenceStatus, ScheduleRecord,
    ScheduleStatus,
};
use flowmation_sqlite::{
    CreateWorkflowRun, OccurrenceUpdate, ScheduleOccurrenceStatus as PersistenceOccurrenceStatus,
    SqliteApplicationRepository, SqliteDatabase, WorkflowPresentation, WorkflowTrigger,
};
use serde_json::json;
use tempfile::tempdir;

const CREATED_AT: &str = "2026-07-25T12:00:00.000Z";

struct RecordingExecution {
    global_dir: PathBuf,
}

#[async_trait]
impl ScheduleExecution for RecordingExecution {
    async fn source_matches(&self, _schedule: &ScheduleRecord) -> Result<bool, String> {
        Ok(true)
    }

    async fn execute(
        &self,
        schedule: &ScheduleRecord,
        occurrence: &ScheduleOccurrence,
    ) -> WorkerExecutionResult {
        self.record_run(schedule, occurrence)
            .unwrap_or_else(|error| WorkerExecutionResult {
                run_id: occurrence.run_id.as_ref().map(ToString::to_string),
                status: ScheduleOccurrenceStatus::Failed,
                result: None,
                error: Some(error),
            })
    }
}

#[derive(Default)]
struct ChangedSourceExecution {
    evaluations: AtomicUsize,
}

#[async_trait]
impl ScheduleExecution for ChangedSourceExecution {
    async fn source_matches(&self, _schedule: &ScheduleRecord) -> Result<bool, String> {
        Ok(false)
    }

    async fn execute(
        &self,
        _schedule: &ScheduleRecord,
        _occurrence: &ScheduleOccurrence,
    ) -> WorkerExecutionResult {
        self.evaluations.fetch_add(1, Ordering::SeqCst);
        WorkerExecutionResult {
            run_id: None,
            status: ScheduleOccurrenceStatus::Completed,
            result: None,
            error: None,
        }
    }
}

impl RecordingExecution {
    fn record_run(
        &self,
        schedule: &ScheduleRecord,
        occurrence: &ScheduleOccurrence,
    ) -> Result<WorkerExecutionResult, String> {
        let mut database =
            SqliteDatabase::open_global_dir(&self.global_dir).map_err(|error| error.to_string())?;
        let run_id = occurrence
            .run_id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("run-{}", occurrence.id));
        database
            .occurrences()
            .update(
                occurrence.id.as_str(),
                PersistenceOccurrenceStatus::Running,
                &OccurrenceUpdate {
                    run_id: Some(run_id.clone()),
                    result: None,
                    error: None,
                },
            )
            .map_err(|error| error.to_string())?;
        if database
            .workflow_runs()
            .get(&run_id)
            .map_err(|error| error.to_string())?
            .is_none()
        {
            database
                .workflow_runs()
                .create_at(
                    &CreateWorkflowRun {
                        id: run_id.clone(),
                        workflow_name: schedule.workflow_name.clone(),
                        project_dir: path_text(&schedule.project_dir)?,
                        agent_name: Some(schedule.agent_name.clone()),
                        trigger: Some(WorkflowTrigger::Schedule {
                            schedule_id: schedule.id.to_string(),
                            scheduled_for: occurrence.scheduled_for.clone(),
                        }),
                        source_entry_path: schedule
                            .project_dir
                            .join(".work-agent/workflows")
                            .join(&schedule.workflow_name)
                            .join("WORKFLOW.js")
                            .display()
                            .to_string(),
                        source_fingerprint: schedule.package_fingerprint.clone(),
                        presentation: WorkflowPresentation::Direct,
                        input: schedule.input.clone(),
                    },
                    CREATED_AT,
                )
                .map_err(|error| error.to_string())?;
        }
        database
            .workflow_runs()
            .transition_to_running_at(&run_id, CREATED_AT)
            .map_err(|error| error.to_string())?;
        database
            .workflow_runs()
            .complete_at(
                &run_id,
                &json!("done"),
                WorkflowPresentation::Direct,
                CREATED_AT,
            )
            .map_err(|error| error.to_string())?;
        Ok(WorkerExecutionResult {
            run_id: Some(run_id),
            status: ScheduleOccurrenceStatus::Completed,
            result: Some(json!("done")),
            error: None,
        })
    }
}

#[tokio::test]
async fn runs_one_catch_up_occurrence_records_trigger_and_recovers() -> Result<(), Box<dyn Error>> {
    let directory = tempdir()?;
    let repository = Arc::new(SqliteApplicationRepository::open_global_dir(
        directory.path(),
    )?);
    let schedule = ScheduleRepository::create(
        repository.as_ref(),
        &CreateScheduleInput {
            project_dir: PathBuf::from("/project"),
            agent_name: "main".to_owned(),
            workflow_name: "scheduled-report".to_owned(),
            input: json!(""),
            cron: "* * * * *".to_owned(),
            timezone: "UTC".to_owned(),
            package_fingerprint: "fingerprint".to_owned(),
            now: Some(CREATED_AT.to_owned()),
        },
        timestamp("2026-07-25T12:01:00.000Z")?,
    )?;
    let worker = ScheduleWorker::new(
        repository.clone(),
        Arc::new(RecordingExecution {
            global_dir: directory.path().to_path_buf(),
        }),
    );

    assert!(worker.tick(timestamp("2026-07-25T12:10:00.000Z")?).await?);
    let occurrences = ScheduleRepository::occurrences(repository.as_ref(), &schedule.id)?;
    assert_eq!(occurrences.len(), 1);
    assert_eq!(occurrences[0].scheduled_for, "2026-07-25T12:01:00.000Z");
    assert_eq!(occurrences[0].status, ScheduleOccurrenceStatus::Completed);
    let run_id = occurrences[0]
        .run_id
        .as_ref()
        .ok_or("catch-up occurrence has no run ID")?;
    let mut database = SqliteDatabase::open_global_dir(directory.path())?;
    let run = database
        .workflow_runs()
        .get(run_id.as_str())?
        .ok_or("catch-up workflow run disappeared")?;
    assert_eq!(
        run.summary.trigger,
        WorkflowTrigger::Schedule {
            schedule_id: schedule.id.to_string(),
            scheduled_for: "2026-07-25T12:01:00.000Z".to_owned(),
        }
    );
    let notifications = database.notifications().unread("/project")?;
    assert_eq!(notifications.len(), 1);
    assert_eq!(
        notifications[0].kind,
        flowmation_sqlite::ScheduleNotificationKind::Completed
    );

    let recovering = database
        .occurrences()
        .create_at(
            schedule.id.as_str(),
            "2026-07-25T12:10:30.000Z",
            PersistenceOccurrenceStatus::Pending,
            None,
            "2026-07-25T12:10:30.000Z",
        )?
        .ok_or("recoverable occurrence was not created")?;
    database.occurrences().update_at(
        &recovering.id,
        PersistenceOccurrenceStatus::Running,
        &OccurrenceUpdate {
            run_id: Some("crash-window-run".to_owned()),
            result: None,
            error: None,
        },
        "2026-07-25T12:10:30.000Z",
    )?;
    drop(database);

    assert!(worker.tick(timestamp("2026-07-25T12:10:30.000Z")?).await?);
    let recovered = ScheduleRepository::occurrences(repository.as_ref(), &schedule.id)?
        .into_iter()
        .find(|occurrence| {
            occurrence.run_id.as_ref() == WorkflowRunId::new("crash-window-run").ok().as_ref()
        })
        .ok_or("recoverable occurrence disappeared")?;
    assert_eq!(recovered.status, ScheduleOccurrenceStatus::Completed);
    Ok(())
}

#[tokio::test]
async fn rejects_changed_source_before_evaluating_and_persists_invalidation()
-> Result<(), Box<dyn Error>> {
    let directory = tempdir()?;
    let repository = Arc::new(SqliteApplicationRepository::open_global_dir(
        directory.path(),
    )?);
    let schedule = ScheduleRepository::create(
        repository.as_ref(),
        &CreateScheduleInput {
            project_dir: PathBuf::from("/project"),
            agent_name: "main".to_owned(),
            workflow_name: "scheduled-report".to_owned(),
            input: json!(""),
            cron: "* * * * *".to_owned(),
            timezone: "UTC".to_owned(),
            package_fingerprint: "approved".to_owned(),
            now: Some(CREATED_AT.to_owned()),
        },
        timestamp("2026-07-25T12:01:00.000Z")?,
    )?;
    let execution = Arc::new(ChangedSourceExecution::default());
    let worker = ScheduleWorker::new(repository.clone(), execution.clone());

    assert!(worker.tick(timestamp("2026-07-25T12:02:00.000Z")?).await?);
    assert_eq!(execution.evaluations.load(Ordering::SeqCst), 0);
    let stored = ScheduleRepository::get(repository.as_ref(), &schedule.id)?
        .ok_or("invalidated schedule disappeared")?;
    assert_eq!(stored.status, ScheduleStatus::NeedsReauthorization);
    let occurrences = ScheduleRepository::occurrences(repository.as_ref(), &schedule.id)?;
    assert_eq!(occurrences.len(), 1);
    assert_eq!(occurrences[0].status, ScheduleOccurrenceStatus::Invalidated);
    assert!(
        occurrences[0]
            .error
            .as_deref()
            .is_some_and(|message| message.contains("needs reauthorization"))
    );
    let mut database = SqliteDatabase::open_global_dir(directory.path())?;
    let notifications = database.notifications().unread("/project")?;
    assert_eq!(notifications.len(), 1);
    assert_eq!(
        notifications[0].kind,
        flowmation_sqlite::ScheduleNotificationKind::Invalidated
    );
    assert_eq!(
        notifications[0].occurrence_id.as_deref(),
        Some(occurrences[0].id.as_str())
    );
    Ok(())
}

fn timestamp(value: &str) -> Result<DateTime<Utc>, chrono::ParseError> {
    value.parse()
}

fn path_text(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("Path is not valid UTF-8: {}", path.display()))
}
