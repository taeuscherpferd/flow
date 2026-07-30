use std::error::Error;

use flowmation_sqlite::{
    LATEST_MIGRATION_VERSION, ScheduleOccurrenceStatus, SqliteDatabase, WorkflowRunStatus,
    WorkflowTrigger,
};
use rusqlite::Connection;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn migrates_typescript_workflow_runs_without_rewriting_data() -> Result<(), Box<dyn Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("runs.sqlite");
    let connection = Connection::open(&path)?;
    connection.execute_batch(
        "
        CREATE TABLE workflow_runs (
          id TEXT PRIMARY KEY,
          workflow_name TEXT NOT NULL,
          project_dir TEXT NOT NULL,
          source_entry_path TEXT NOT NULL,
          source_fingerprint TEXT NOT NULL,
          status TEXT NOT NULL,
          presentation TEXT NOT NULL,
          input_json TEXT NOT NULL,
          output_json TEXT,
          parent_run_id TEXT,
          depth INTEGER NOT NULL,
          error TEXT,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );
        INSERT INTO workflow_runs VALUES (
          'old-run', 'legacy', '/project', '/workflow.js', 'fingerprint',
          'completed', 'direct', '\"\"', '\"done\"', NULL, 0, NULL,
          '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'
        );
        ",
    )?;
    drop(connection);

    let mut database = SqliteDatabase::open(&path)?;
    let run = database
        .workflow_runs()
        .get("old-run")?
        .ok_or("legacy workflow run was not preserved")?;
    assert_eq!(run.summary.agent_name, "main");
    assert_eq!(run.summary.trigger, WorkflowTrigger::Manual);
    assert_eq!(run.summary.status, WorkflowRunStatus::Completed);
    assert_eq!(run.output, Some(json!("done")));
    assert_eq!(
        database.applied_migrations()?.len(),
        usize::try_from(LATEST_MIGRATION_VERSION)?
    );
    Ok(())
}

#[test]
fn completes_a_partially_present_schedule_schema_additively() -> Result<(), Box<dyn Error>> {
    let directory = tempdir()?;
    let path = directory.path().join("runs.sqlite");
    let connection = Connection::open(&path)?;
    connection.execute_batch(
        r#"
        CREATE TABLE workflow_runs (
          id TEXT PRIMARY KEY,
          workflow_name TEXT NOT NULL,
          project_dir TEXT NOT NULL,
          agent_name TEXT NOT NULL DEFAULT 'main',
          trigger_json TEXT NOT NULL DEFAULT '{"type":"manual"}',
          source_entry_path TEXT NOT NULL,
          source_fingerprint TEXT NOT NULL,
          status TEXT NOT NULL,
          presentation TEXT NOT NULL,
          input_json TEXT NOT NULL,
          output_json TEXT,
          parent_run_id TEXT,
          depth INTEGER NOT NULL,
          error TEXT,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );
        CREATE TABLE schedules (
          id TEXT PRIMARY KEY,
          project_dir TEXT NOT NULL,
          agent_name TEXT NOT NULL,
          workflow_name TEXT NOT NULL,
          input_json TEXT NOT NULL,
          cron TEXT NOT NULL,
          timezone TEXT NOT NULL,
          package_fingerprint TEXT NOT NULL,
          status TEXT NOT NULL,
          next_run_at TEXT NOT NULL,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );
        CREATE TABLE schedule_occurrences (
          id TEXT PRIMARY KEY,
          schedule_id TEXT NOT NULL,
          scheduled_for TEXT NOT NULL,
          status TEXT NOT NULL,
          run_id TEXT,
          result_json TEXT,
          error TEXT,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          UNIQUE(schedule_id, scheduled_for),
          FOREIGN KEY(schedule_id) REFERENCES schedules(id) ON DELETE CASCADE
        );
        INSERT INTO schedules VALUES (
          'old-schedule', '/project', 'main', 'report', '""', '* * * * *',
          'UTC', 'old-fingerprint', 'active', '2026-01-01T00:01:00.000Z',
          '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'
        );
        INSERT INTO schedule_occurrences VALUES (
          'old-occurrence', 'old-schedule', '2026-01-01T00:01:00.000Z',
          'waiting', NULL, NULL, NULL,
          '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'
        );
        "#,
    )?;
    drop(connection);

    let mut database = SqliteDatabase::open(&path)?;
    let schedule = database
        .schedules()
        .get("old-schedule")?
        .ok_or("legacy schedule was not preserved")?;
    assert_eq!(schedule.package_fingerprint, "old-fingerprint");
    let occurrence = database
        .occurrences()
        .get("old-occurrence")?
        .ok_or("legacy occurrence was not preserved")?;
    assert_eq!(occurrence.status, ScheduleOccurrenceStatus::Waiting);
    assert!(database.notifications().unread("/project")?.is_empty());

    let connection = Connection::open(&path)?;
    let trigger_count = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type = 'trigger' AND name = 'schedule_occurrence_run_status'",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    assert_eq!(trigger_count, 1);
    Ok(())
}
