use std::error::Error;

use chrono::{DateTime, Utc};
use flowmation_sqlite::{
    AgentSessionRecord, ChatRole, CreateSchedule, CreateWorkflowRun, LATEST_MIGRATION_VERSION,
    NewWorkflowStep, OccurrenceUpdate, ScheduleKind, ScheduleOccurrenceStatus, ScheduleStatus,
    SqliteDatabase, StoredChatMessage, WorkflowPresentation, WorkflowRunStatus, WorkflowStepKind,
    WorkflowStepState, WorkflowTrigger,
};
use rusqlite::Connection;
use serde_json::json;
use tempfile::tempdir;

const FIRST: &str = "2026-01-01T00:00:00.000Z";
const SECOND: &str = "2026-01-01T00:01:00.000Z";

fn run(id: &str, trigger: Option<WorkflowTrigger>) -> CreateWorkflowRun {
    CreateWorkflowRun {
        id: id.to_owned(),
        workflow_name: "monthly-close".to_owned(),
        project_dir: "/project".to_owned(),
        agent_name: Some("finance".to_owned()),
        trigger,
        source_entry_path: "/workflow.js".to_owned(),
        source_fingerprint: "fingerprint".to_owned(),
        presentation: WorkflowPresentation::Direct,
        input: json!({ "month": "2025-12" }),
    }
}

fn schedule() -> CreateSchedule {
    CreateSchedule {
        id: Some("schedule-1".to_owned()),
        project_dir: "/project".to_owned(),
        agent_name: "finance".to_owned(),
        workflow_name: "monthly-close".to_owned(),
        input: json!({ "month": "2025-12" }),
        kind: ScheduleKind::Cron,
        cron: "0 9 1 * *".to_owned(),
        timezone: "America/Denver".to_owned(),
        package_fingerprint: "abc123".to_owned(),
        next_run_at: "2026-02-01T16:00:00.000Z".to_owned(),
        now: Some(FIRST.to_owned()),
    }
}

#[test]
fn creates_the_legacy_compatible_schema_and_configures_sqlite() -> Result<(), Box<dyn Error>> {
    let directory = tempdir()?;
    let database = SqliteDatabase::open_global_dir(directory.path())?;
    let migrations = database.applied_migrations()?;
    assert_eq!(migrations.len(), usize::try_from(LATEST_MIGRATION_VERSION)?);
    assert_eq!(
        migrations.last().map(|migration| migration.version),
        Some(LATEST_MIGRATION_VERSION)
    );
    assert_eq!(
        migrations
            .iter()
            .map(|migration| migration.version)
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5, 6]
    );

    let connection = Connection::open(database.path())?;
    let tables = schema_objects(&connection, "table")?;
    for expected in [
        "agent_conversations",
        "flowmation_migrations",
        "schedule_notifications",
        "schedule_occurrences",
        "schedule_worker_leases",
        "schedules",
        "workflow_runs",
        "workflow_steps",
    ] {
        assert!(tables.iter().any(|table| table == expected));
    }
    assert_eq!(
        schema_objects(&connection, "trigger")?,
        vec!["schedule_occurrence_run_status"]
    );

    let columns = table_columns(&connection, "workflow_runs")?;
    assert!(columns.iter().any(|column| {
        column.0 == "agent_name"
            && column.1 == "TEXT"
            && column.2 == 1
            && column.3.as_deref() == Some("'main'")
    }));
    assert!(columns.iter().any(|column| {
        column.0 == "trigger_json"
            && column.1 == "TEXT"
            && column.2 == 1
            && column.3.as_deref() == Some("'{\"type\":\"manual\"}'")
    }));
    let schedule_columns = table_columns(&connection, "schedules")?;
    assert!(schedule_columns.iter().any(|column| {
        column.0 == "schedule_kind"
            && column.1 == "TEXT"
            && column.2 == 1
            && column.3.as_deref() == Some("'cron'")
    }));
    Ok(())
}

#[test]
fn claiming_a_one_shot_schedule_exhausts_it_atomically() -> Result<(), Box<dyn Error>> {
    let mut database = SqliteDatabase::open_in_memory()?;
    let mut input = schedule();
    input.id = Some("one-shot".to_owned());
    input.kind = ScheduleKind::Once;
    input.cron.clear();
    input.timezone = "UTC".to_owned();
    input.next_run_at = SECOND.to_owned();
    let created = database.schedules().create(&input)?;

    let occurrence = database
        .occurrences()
        .claim_due_at(&created.id, SECOND, None, SECOND)?;

    assert!(occurrence.is_some());
    assert_eq!(
        database
            .schedules()
            .get(&created.id)?
            .map(|stored| stored.status),
        Some(ScheduleStatus::Completed)
    );
    assert!(
        database
            .schedules()
            .list_due("2027-01-01T00:00:00.000Z")?
            .is_empty()
    );
    Ok(())
}

#[test]
fn stores_runs_steps_effects_and_human_responses() -> Result<(), Box<dyn Error>> {
    let mut database = SqliteDatabase::open_in_memory()?;
    let created = database
        .workflow_runs()
        .create_at(&run("run-1", None), FIRST)?;
    assert_eq!(created.summary.status, WorkflowRunStatus::Queued);
    assert_eq!(created.summary.trigger, WorkflowTrigger::Manual);
    assert_eq!(created.input, json!({ "month": "2025-12" }));

    assert!(
        database
            .workflow_runs()
            .transition_to_running_at("run-1", SECOND)?
    );
    assert!(
        !database
            .workflow_runs()
            .transition_to_running_at("run-1", SECOND)?
    );

    database.workflow_steps().start_at(
        &NewWorkflowStep {
            run_id: "run-1".to_owned(),
            key: "draft".to_owned(),
            kind: WorkflowStepKind::Checkpoint,
            input: None,
        },
        FIRST,
    )?;
    assert!(database.workflow_steps().complete_at(
        "run-1",
        "draft",
        &json!({ "saved": true }),
        SECOND,
    )?);
    let checkpoint = database.workflow_steps().get("run-1", "draft")?;
    assert_eq!(
        checkpoint.as_ref().map(|step| step.state),
        Some(WorkflowStepState::Completed)
    );

    database.effects().start("run-1", "publish", "branch-123")?;
    database
        .effects()
        .complete("run-1", "publish", &json!({ "published": true }))?;
    let effect = database.effects().get("run-1", "publish")?;
    assert_eq!(
        effect
            .as_ref()
            .map(|record| record.idempotency_key.as_str()),
        Some("branch-123")
    );

    database.human_responses().request(
        "run-1",
        "human.prompt.0",
        &json!({ "kind": "text", "prompt": "Continue?" }),
    )?;
    database
        .human_responses()
        .respond("run-1", "human.prompt.0", &json!("yes"))?;
    let response = database.human_responses().get("run-1", "human.prompt.0")?;
    assert_eq!(
        response.and_then(|record| record.response),
        Some(json!("yes"))
    );
    Ok(())
}

#[test]
fn stores_unique_occurrences_and_renewable_worker_leases() -> Result<(), Box<dyn Error>> {
    let mut database = SqliteDatabase::open_in_memory()?;
    let created = database.schedules().create(&schedule())?;
    let occurrence = database.occurrences().claim_due_at(
        &created.id,
        "2026-02-01T16:00:00.000Z",
        Some("2026-03-01T16:00:00.000Z"),
        SECOND,
    )?;
    assert!(occurrence.is_some());
    assert!(
        database
            .occurrences()
            .create_at(
                &created.id,
                "2026-02-01T16:00:00.000Z",
                ScheduleOccurrenceStatus::Pending,
                None,
                SECOND,
            )?
            .is_none()
    );
    assert!(
        database
            .occurrences()
            .claim_due_at(
                &created.id,
                "2026-03-01T16:00:00.000Z",
                Some("2026-04-01T15:00:00.000Z"),
                SECOND,
            )?
            .is_none()
    );
    let statuses = database
        .occurrences()
        .list(&created.id)?
        .into_iter()
        .map(|entry| entry.status)
        .collect::<Vec<_>>();
    assert!(statuses.contains(&ScheduleOccurrenceStatus::Pending));
    assert!(statuses.contains(&ScheduleOccurrenceStatus::Skipped));
    assert_eq!(
        database
            .schedules()
            .get(&created.id)?
            .map(|stored| stored.next_run_at),
        Some("2026-04-01T15:00:00.000Z".to_owned())
    );

    let first = DateTime::parse_from_rfc3339(FIRST)?.with_timezone(&Utc);
    assert!(
        database
            .worker_leases()
            .acquire("worker", "one", first, 30_000)?
    );
    assert!(
        !database
            .worker_leases()
            .acquire("worker", "two", first, 30_000)?
    );
    assert!(database.worker_leases().acquire(
        "worker",
        "two",
        first + chrono::Duration::seconds(31),
        30_000,
    )?);
    Ok(())
}

#[test]
fn mirrors_scheduled_run_outcomes_and_creates_notifications() -> Result<(), Box<dyn Error>> {
    let mut database = SqliteDatabase::open_in_memory()?;
    database.schedules().create(&schedule())?;
    let occurrence = database
        .occurrences()
        .create_at(
            "schedule-1",
            "2026-02-01T16:00:00.000Z",
            ScheduleOccurrenceStatus::Pending,
            None,
            FIRST,
        )?
        .ok_or("occurrence was not created")?;
    database.workflow_runs().create_at(
        &run(
            "run-scheduled",
            Some(WorkflowTrigger::Schedule {
                schedule_id: "schedule-1".to_owned(),
                scheduled_for: "2026-02-01T16:00:00.000Z".to_owned(),
            }),
        ),
        FIRST,
    )?;
    database.occurrences().update_at(
        &occurrence.id,
        ScheduleOccurrenceStatus::Running,
        &OccurrenceUpdate {
            run_id: Some("run-scheduled".to_owned()),
            ..OccurrenceUpdate::default()
        },
        FIRST,
    )?;
    database
        .workflow_runs()
        .transition_to_running_at("run-scheduled", SECOND)?;
    database.workflow_runs().complete_at(
        "run-scheduled",
        &json!({ "closed": true }),
        WorkflowPresentation::Agent,
        SECOND,
    )?;

    let stored = database
        .occurrences()
        .get(&occurrence.id)?
        .ok_or("occurrence disappeared")?;
    assert_eq!(stored.status, ScheduleOccurrenceStatus::Completed);
    assert_eq!(stored.result, Some(json!({ "closed": true })));
    let notifications = database.notifications().unread("/project")?;
    assert_eq!(notifications.len(), 1);
    assert_eq!(
        notifications.first().map(|entry| entry.message.as_str()),
        Some("Scheduled workflow finance/monthly-close completed (run-scheduled).")
    );
    Ok(())
}

#[test]
fn isolates_conversations_and_never_persists_system_messages() -> Result<(), Box<dyn Error>> {
    let mut database = SqliteDatabase::open_in_memory()?;
    let project_a = session("finance-a", "/project-a");
    let project_b = session("finance-b", "/project-b");
    database.agent_conversations().save_at(
        &project_a,
        &[
            message(ChatRole::System, "stale system prompt"),
            message(ChatRole::User, "project A question"),
            message(ChatRole::Assistant, "project A answer"),
        ],
        SECOND,
    )?;
    database.agent_conversations().save_at(
        &project_b,
        &[message(ChatRole::User, "project B question")],
        SECOND,
    )?;

    let history_a = database
        .agent_conversations()
        .get("/project-a", "finance")?
        .ok_or("project A conversation disappeared")?
        .history;
    assert_eq!(
        history_a,
        vec![
            message(ChatRole::User, "project A question"),
            message(ChatRole::Assistant, "project A answer"),
        ]
    );
    assert_eq!(
        database
            .agent_conversations()
            .get("/project-b", "finance")?
            .map(|conversation| conversation.history),
        Some(vec![message(ChatRole::User, "project B question")])
    );
    assert!(
        database
            .agent_conversations()
            .get("/project-a", "main")?
            .is_none()
    );
    assert!(
        database
            .agent_conversations()
            .clear_at("/project-a", "finance", SECOND)?
    );
    assert!(
        database
            .agent_conversations()
            .get("/project-a", "finance")?
            .ok_or("cleared conversation disappeared")?
            .history
            .is_empty()
    );
    assert_eq!(
        database
            .agent_conversations()
            .get("/project-b", "finance")?
            .map(|conversation| conversation.history.len()),
        Some(1)
    );
    Ok(())
}

fn session(id: &str, project_dir: &str) -> AgentSessionRecord {
    AgentSessionRecord {
        id: id.to_owned(),
        project_dir: project_dir.to_owned(),
        agent_name: "finance".to_owned(),
        provider: "local".to_owned(),
        model: "finance-model".to_owned(),
        created_at: FIRST.to_owned(),
        updated_at: FIRST.to_owned(),
    }
}

fn message(role: ChatRole, content: &str) -> StoredChatMessage {
    StoredChatMessage {
        role,
        content: content.to_owned(),
        thinking: None,
        tool_calls: None,
        tool_call_id: None,
        tool_name: None,
    }
}

fn schema_objects(connection: &Connection, kind: &str) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection.prepare(
        "SELECT name FROM sqlite_master
         WHERE type = ? AND name NOT LIKE 'sqlite_%'
         ORDER BY name",
    )?;
    statement
        .query_map([kind], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()
}

type Column = (String, String, i64, Option<String>);

fn table_columns(connection: &Connection, table: &str) -> rusqlite::Result<Vec<Column>> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    statement
        .query_map([], |row| {
            Ok((row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
}
