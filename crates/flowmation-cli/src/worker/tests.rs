use std::error::Error;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use flowmation_application::ScheduleExecution;
use flowmation_application::WorkflowDurability;
use flowmation_application::workflow::WorkflowRecord;
use flowmation_domain::agent::PackageSource;
use flowmation_domain::fingerprint::fingerprint_directory;
use flowmation_domain::ids::ScheduleId;
use flowmation_domain::schedule::{ScheduleKind, ScheduleRecord, ScheduleStatus};
use flowmation_sqlite::{
    CreateSchedule, ScheduleOccurrenceStatus, SqliteApplicationRepository, SqliteDatabase,
    WorkflowTrigger,
};
use flowmation_workflow_host::protocol::{
    AgentInvocationPolicy, WorkflowMetadata, WorkflowPresentation,
};
use serde_json::json;
use tempfile::tempdir;

use super::ScheduledWorkflowExecution;
use super::durability::ScheduledDurability;

#[tokio::test]
async fn changed_source_is_rejected_without_evaluating_the_module() -> Result<(), Box<dyn Error>> {
    let root = tempdir()?;
    let global_dir = root.path().join("global");
    let project_dir = root.path().join("project");
    let workflow_dir = global_dir.join("workflows/scheduled-report");
    let workflow_path = workflow_dir.join("WORKFLOW.js");
    let sentinel = root.path().join("unauthorized-module-ran");
    fs::create_dir_all(&workflow_dir)?;
    fs::write(
        &workflow_path,
        "export default { name: 'scheduled-report', description: 'safe', run() {} };",
    )?;
    let schedule = ScheduleRecord {
        id: ScheduleId::new("schedule-1")?,
        project_dir,
        agent_name: "main".to_owned(),
        workflow_name: "scheduled-report".to_owned(),
        input: json!(""),
        kind: ScheduleKind::Cron,
        cron: "* * * * *".to_owned(),
        timezone: "UTC".to_owned(),
        package_fingerprint: fingerprint_directory(&workflow_dir)?,
        status: ScheduleStatus::Active,
        next_run_at: "2026-07-25T12:01:00.000Z".to_owned(),
        created_at: "2026-07-25T12:00:00.000Z".to_owned(),
        updated_at: "2026-07-25T12:00:00.000Z".to_owned(),
    };
    let execution = ScheduledWorkflowExecution { global_dir };
    assert!(execution.source_matches(&schedule).await?);

    fs::write(
        workflow_path,
        format!(
            "import {{ writeFileSync }} from 'node:fs'; writeFileSync({}, 'executed');",
            serde_json::to_string(&sentinel.display().to_string())?
        ),
    )?;

    assert!(!execution.source_matches(&schedule).await?);
    assert!(!sentinel.exists());
    Ok(())
}

#[test]
fn project_workflow_source_takes_precedence_over_global() -> Result<(), Box<dyn Error>> {
    let root = tempdir()?;
    let global_dir = root.path().join("global");
    let project_dir = root.path().join("project");
    fs::create_dir_all(global_dir.join("workflows/scheduled-report"))?;
    fs::create_dir_all(
        project_dir
            .join(".work-agent")
            .join("workflows/scheduled-report"),
    )?;
    let schedule = ScheduleRecord {
        id: ScheduleId::new("schedule-1")?,
        project_dir: project_dir.clone(),
        agent_name: "main".to_owned(),
        workflow_name: "scheduled-report".to_owned(),
        input: json!(""),
        kind: ScheduleKind::Cron,
        cron: "* * * * *".to_owned(),
        timezone: "UTC".to_owned(),
        package_fingerprint: "fingerprint".to_owned(),
        status: ScheduleStatus::Active,
        next_run_at: String::new(),
        created_at: String::new(),
        updated_at: String::new(),
    };

    let source = super::resolve_source(&global_dir, &schedule).ok_or("source was not resolved")?;
    assert_eq!(
        source.authorization_directory,
        project_dir.join(".work-agent/workflows/scheduled-report")
    );
    assert_eq!(
        source.workflow_root,
        project_dir.join(".work-agent/workflows")
    );
    assert_eq!(
        source.package_source,
        flowmation_domain::agent::PackageSource::Project
    );
    Ok(())
}

#[tokio::test]
async fn scheduled_durability_links_occurrence_before_recording_trigger()
-> Result<(), Box<dyn Error>> {
    let root = tempdir()?;
    let mut database = SqliteDatabase::open_global_dir(root.path())?;
    let schedule = database.schedules().create(&CreateSchedule {
        id: Some("schedule-1".to_owned()),
        project_dir: "/project".to_owned(),
        agent_name: "main".to_owned(),
        workflow_name: "scheduled-report".to_owned(),
        input: json!(""),
        kind: flowmation_sqlite::ScheduleKind::Cron,
        cron: "* * * * *".to_owned(),
        timezone: "UTC".to_owned(),
        package_fingerprint: "fingerprint".to_owned(),
        next_run_at: "2026-07-25T12:01:00.000Z".to_owned(),
        now: Some("2026-07-25T12:00:00.000Z".to_owned()),
    })?;
    let occurrence = database
        .occurrences()
        .create_at(
            &schedule.id,
            "2026-07-25T12:01:00.000Z",
            ScheduleOccurrenceStatus::Pending,
            None,
            "2026-07-25T12:01:00.000Z",
        )?
        .ok_or("occurrence was not created")?;
    drop(database);
    let repository = Arc::new(SqliteApplicationRepository::open_global_dir(root.path())?);
    let durability = ScheduledDurability::new(
        repository,
        &schedule.id,
        &occurrence.id,
        &occurrence.scheduled_for,
        None,
        root.path(),
    );
    let record = WorkflowRecord {
        metadata: WorkflowMetadata {
            name: "scheduled-report".to_owned(),
            description: "Produces a report".to_owned(),
            input_schema: None,
            agent_invocation: AgentInvocationPolicy::Disabled,
            presentation: WorkflowPresentation::Direct,
        },
        directory: root.path().join("workflows/scheduled-report"),
        entry_path: root.path().join("workflows/scheduled-report/WORKFLOW.js"),
        fingerprint: "fingerprint".to_owned(),
        source: PackageSource::Global,
        agent_name: Some("main".to_owned()),
        resource_id: Some("main/scheduled-report".to_owned()),
    };

    durability
        .create_run("run-1", &record, Path::new("/project"), &json!(""))
        .await?;

    let mut verification = SqliteDatabase::open_global_dir(root.path())?;
    let stored_occurrence = verification
        .occurrences()
        .get(&occurrence.id)?
        .ok_or("occurrence disappeared")?;
    assert_eq!(stored_occurrence.status, ScheduleOccurrenceStatus::Running);
    assert_eq!(stored_occurrence.run_id.as_deref(), Some("run-1"));
    let run = verification
        .workflow_runs()
        .get("run-1")?
        .ok_or("workflow run disappeared")?;
    assert_eq!(
        run.summary.trigger,
        WorkflowTrigger::Schedule {
            schedule_id: schedule.id,
            scheduled_for: occurrence.scheduled_for,
        }
    );
    Ok(())
}
