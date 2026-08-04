use std::error::Error;
use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, Utc};
use flowmation_application::scheduling::{
    ScheduleRepository, ScheduleWorkerRepository, WorkerExecutionResult,
};
use flowmation_application::workflow::{DurableStepKind, WorkflowDurability, WorkflowRecord};
use flowmation_application::{ChatMessage, ChatRole, ConversationRepository, StoredConversation};
use flowmation_domain::agent::{AgentExecutionMode, AgentSessionRecord, PackageSource};
use flowmation_domain::ids::AgentSessionId;
use flowmation_domain::schedule::{
    CreateScheduleInput, ScheduleKind, ScheduleOccurrenceStatus, ScheduleStatus,
};
use flowmation_sqlite::{SqliteApplicationRepository, SqliteDatabase};
use flowmation_workflow_host::protocol::{
    AgentInvocationPolicy, WorkflowMetadata, WorkflowPresentation,
};
use serde_json::json;
use tempfile::tempdir;

const FIRST: &str = "2026-01-01T00:00:00.000Z";

#[test]
fn conversation_trait_preserves_isolation_and_filters_system_messages() -> Result<(), Box<dyn Error>>
{
    let directory = tempdir()?;
    let repository = SqliteApplicationRepository::open_global_dir(directory.path())?;
    let session = AgentSessionRecord {
        id: AgentSessionId::new("session-a")?,
        project_dir: PathBuf::from("/project-a"),
        agent_name: "finance".to_owned(),
        mode: AgentExecutionMode::Direct,
        provider: "local".to_owned(),
        model: "model".to_owned(),
        created_at: FIRST.to_owned(),
        updated_at: FIRST.to_owned(),
    };
    ConversationRepository::save(
        &repository,
        &StoredConversation {
            session,
            history: vec![
                ChatMessage::new(ChatRole::System, "reconstructed"),
                ChatMessage::new(ChatRole::User, "hello"),
            ],
        },
    )?;

    let stored = ConversationRepository::get(&repository, "/project-a", "finance")?
        .ok_or("conversation was not stored")?;
    assert_eq!(
        stored.history,
        vec![ChatMessage::new(ChatRole::User, "hello")]
    );
    assert!(ConversationRepository::get(&repository, "/project-b", "finance")?.is_none());
    ConversationRepository::clear(&repository, "/project-a", "finance")?;
    assert!(
        ConversationRepository::get(&repository, "/project-a", "finance")?
            .ok_or("cleared conversation disappeared")?
            .history
            .is_empty()
    );
    Ok(())
}

#[tokio::test]
async fn workflow_durability_trait_uses_legacy_run_and_step_storage() -> Result<(), Box<dyn Error>>
{
    let directory = tempdir()?;
    let repository = SqliteApplicationRepository::open_global_dir(directory.path())?;
    let workflow = WorkflowRecord {
        metadata: WorkflowMetadata {
            name: "report".to_owned(),
            description: "Creates a report".to_owned(),
            input_schema: None,
            agent_invocation: AgentInvocationPolicy::Disabled,
            presentation: WorkflowPresentation::Direct,
        },
        directory: PathBuf::from("/workflows/report"),
        entry_path: PathBuf::from("/workflows/report/WORKFLOW.js"),
        fingerprint: "fingerprint".to_owned(),
        source: PackageSource::Global,
        agent_name: Some("finance".to_owned()),
        resource_id: Some("finance/report".to_owned()),
    };
    WorkflowDurability::create_run(
        &repository,
        "run-1",
        &workflow,
        PathBuf::from("/project").as_path(),
        &json!({ "month": "2025-12" }),
    )
    .await?;
    WorkflowDurability::mark_running(&repository, "run-1").await?;
    WorkflowDurability::start_step(
        &repository,
        "run-1",
        "publish",
        DurableStepKind::Effect,
        Some(&json!({ "idempotencyKey": "branch-123" })),
    )
    .await?;
    WorkflowDurability::complete_step(
        &repository,
        "run-1",
        "publish",
        &json!({ "published": true }),
    )
    .await?;
    let step = WorkflowDurability::step(&repository, "run-1", "publish")
        .await?
        .ok_or("durable step disappeared")?;
    assert!(step.completed);
    assert_eq!(step.kind, DurableStepKind::Effect);
    WorkflowDurability::complete_run(
        &repository,
        "run-1",
        &json!("done"),
        WorkflowPresentation::Agent,
    )
    .await?;

    let mut database = SqliteDatabase::open_global_dir(directory.path())?;
    let run = database
        .workflow_runs()
        .get("run-1")?
        .ok_or("durable run disappeared")?;
    assert_eq!(run.output, Some(json!("done")));
    assert_eq!(run.summary.agent_name, "finance");
    Ok(())
}

#[test]
fn schedule_traits_share_occurrences_and_worker_leases() -> Result<(), Box<dyn Error>> {
    let directory = tempdir()?;
    let repository = SqliteApplicationRepository::open_global_dir(directory.path())?;
    let first = timestamp(FIRST)?;
    let next = timestamp("2026-01-01T00:01:00.000Z")?;
    let later = timestamp("2026-01-01T00:02:00.000Z")?;
    let schedule = ScheduleRepository::create(
        &repository,
        &CreateScheduleInput {
            project_dir: PathBuf::from("/project"),
            agent_name: "finance".to_owned(),
            workflow_name: "report".to_owned(),
            input: json!(""),
            kind: ScheduleKind::Cron,
            cron: "* * * * *".to_owned(),
            timezone: "UTC".to_owned(),
            package_fingerprint: "fingerprint".to_owned(),
            now: Some(FIRST.to_owned()),
        },
        next,
    )?;
    assert_eq!(schedule.status, ScheduleStatus::Active);
    assert_eq!(ScheduleWorkerRepository::due(&repository, later)?.len(), 1);
    assert!(ScheduleWorkerRepository::acquire_lease(
        &repository,
        "worker",
        "owner-a",
        first,
        Duration::from_secs(30),
    )?);
    assert!(!ScheduleWorkerRepository::acquire_lease(
        &repository,
        "worker",
        "owner-b",
        first,
        Duration::from_secs(30),
    )?);

    let occurrence = ScheduleWorkerRepository::claim(&repository, &schedule, next, Some(later))?
        .ok_or("occurrence was not claimed")?;
    ScheduleWorkerRepository::update_occurrence(
        &repository,
        &occurrence,
        &WorkerExecutionResult {
            run_id: Some("run-1".to_owned()),
            status: ScheduleOccurrenceStatus::Completed,
            result: Some(json!("done")),
            error: None,
        },
    )?;
    let occurrences = ScheduleRepository::occurrences(&repository, &schedule.id)?;
    assert_eq!(
        occurrences.first().map(|entry| entry.status),
        Some(ScheduleOccurrenceStatus::Completed)
    );
    assert_eq!(
        occurrences.first().and_then(|entry| entry.result.clone()),
        Some(json!("done"))
    );
    Ok(())
}

fn timestamp(value: &str) -> Result<DateTime<Utc>, chrono::ParseError> {
    value.parse()
}
