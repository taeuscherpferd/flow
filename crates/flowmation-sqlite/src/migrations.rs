use chrono::{SecondsFormat, Utc};
use rusqlite::{Connection, Transaction, TransactionBehavior, params};

use crate::{PersistenceError, Result};

pub const LATEST_MIGRATION_VERSION: i64 = 6;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppliedMigration {
    pub version: i64,
    pub name: String,
    pub applied_at: String,
}

struct Migration {
    version: i64,
    name: &'static str,
    apply: fn(&Transaction<'_>) -> rusqlite::Result<()>,
}

const MIGRATIONS: [Migration; 6] = [
    Migration {
        version: 1,
        name: "workflow storage",
        apply: create_workflow_storage,
    },
    Migration {
        version: 2,
        name: "workflow agent and trigger metadata",
        apply: ensure_workflow_metadata,
    },
    Migration {
        version: 3,
        name: "schedule storage",
        apply: create_schedule_storage,
    },
    Migration {
        version: 4,
        name: "schedule run status trigger",
        apply: create_schedule_trigger,
    },
    Migration {
        version: 5,
        name: "agent conversation storage",
        apply: create_conversation_storage,
    },
    Migration {
        version: 6,
        name: "one-shot schedule timing",
        apply: ensure_schedule_kind,
    },
];

pub fn migrate(connection: &mut Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS flowmation_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at TEXT NOT NULL
        );",
    )?;

    let latest_applied = connection.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM flowmation_migrations",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    if latest_applied > LATEST_MIGRATION_VERSION {
        return Err(PersistenceError::SchemaTooNew {
            found: latest_applied,
            latest: LATEST_MIGRATION_VERSION,
        });
    }

    for migration in MIGRATIONS
        .iter()
        .filter(|item| item.version > latest_applied)
    {
        apply_migration(connection, migration)?;
    }
    Ok(())
}

pub fn applied_migrations(connection: &Connection) -> Result<Vec<AppliedMigration>> {
    let mut statement = connection.prepare(
        "SELECT version, name, applied_at
         FROM flowmation_migrations
         ORDER BY version",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(AppliedMigration {
            version: row.get(0)?,
            name: row.get(1)?,
            applied_at: row.get(2)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(PersistenceError::from)
}

fn apply_migration(connection: &mut Connection, migration: &Migration) -> Result<()> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|source| PersistenceError::Migration {
            version: migration.version,
            name: migration.name,
            source,
        })?;
    (migration.apply)(&transaction).map_err(|source| PersistenceError::Migration {
        version: migration.version,
        name: migration.name,
        source,
    })?;
    transaction
        .execute(
            "INSERT INTO flowmation_migrations (version, name, applied_at)
             VALUES (?, ?, ?)",
            params![
                migration.version,
                migration.name,
                Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            ],
        )
        .map_err(|source| PersistenceError::Migration {
            version: migration.version,
            name: migration.name,
            source,
        })?;
    transaction
        .commit()
        .map_err(|source| PersistenceError::Migration {
            version: migration.version,
            name: migration.name,
            source,
        })
}

fn create_workflow_storage(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    transaction.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS workflow_runs (
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
          updated_at TEXT NOT NULL,
          FOREIGN KEY(parent_run_id) REFERENCES workflow_runs(id)
        );

        CREATE INDEX IF NOT EXISTS workflow_runs_project_updated
        ON workflow_runs(project_dir, updated_at DESC);

        CREATE TABLE IF NOT EXISTS workflow_steps (
          run_id TEXT NOT NULL,
          key TEXT NOT NULL,
          kind TEXT NOT NULL,
          state TEXT NOT NULL,
          input_json TEXT,
          output_json TEXT,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          PRIMARY KEY(run_id, key),
          FOREIGN KEY(run_id) REFERENCES workflow_runs(id) ON DELETE CASCADE
        );
        "#,
    )
}

fn ensure_workflow_metadata(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    ensure_column(
        transaction,
        "workflow_runs",
        "agent_name",
        "TEXT NOT NULL DEFAULT 'main'",
    )?;
    ensure_column(
        transaction,
        "workflow_runs",
        "trigger_json",
        r#"TEXT NOT NULL DEFAULT '{"type":"manual"}'"#,
    )
}

fn ensure_column(
    transaction: &Transaction<'_>,
    table: &'static str,
    column: &str,
    definition: &'static str,
) -> rusqlite::Result<()> {
    let mut statement = transaction.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement.query_map([], |row| row.get::<_, String>(1))?;
    for existing in columns {
        if existing? == column {
            return Ok(());
        }
    }
    transaction.execute_batch(&format!(
        "ALTER TABLE {table} ADD COLUMN {column} {definition}"
    ))
}

fn create_schedule_storage(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    transaction.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS schedules (
          id TEXT PRIMARY KEY,
          project_dir TEXT NOT NULL,
          agent_name TEXT NOT NULL,
          workflow_name TEXT NOT NULL,
          input_json TEXT NOT NULL,
          schedule_kind TEXT NOT NULL DEFAULT 'cron',
          cron TEXT NOT NULL,
          timezone TEXT NOT NULL,
          package_fingerprint TEXT NOT NULL,
          status TEXT NOT NULL,
          next_run_at TEXT NOT NULL,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS schedules_due
        ON schedules(status, next_run_at);

        CREATE TABLE IF NOT EXISTS schedule_occurrences (
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

        CREATE TABLE IF NOT EXISTS schedule_worker_leases (
          lease_key TEXT PRIMARY KEY,
          owner_id TEXT NOT NULL,
          expires_at TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS schedule_notifications (
          id TEXT PRIMARY KEY,
          project_dir TEXT NOT NULL,
          agent_name TEXT NOT NULL,
          schedule_id TEXT,
          occurrence_id TEXT,
          kind TEXT NOT NULL,
          message TEXT NOT NULL,
          is_read INTEGER NOT NULL DEFAULT 0,
          created_at TEXT NOT NULL
        );
        ",
    )
}

fn ensure_schedule_kind(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    ensure_column(
        transaction,
        "schedules",
        "schedule_kind",
        "TEXT NOT NULL DEFAULT 'cron'",
    )
}

fn create_schedule_trigger(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    transaction.execute_batch(
        r"
        CREATE TRIGGER IF NOT EXISTS schedule_occurrence_run_status
        AFTER UPDATE OF status, output_json, error ON workflow_runs
        WHEN json_extract(NEW.trigger_json, '$.type') = 'schedule'
        BEGIN
          UPDATE schedule_occurrences
          SET status = CASE
                WHEN NEW.status = 'completed' THEN 'completed'
                WHEN NEW.status = 'waiting' THEN 'waiting'
                WHEN NEW.status IN ('failed', 'cancelled', 'version-mismatch')
                  THEN 'failed'
                ELSE 'running'
              END,
              result_json = NEW.output_json,
              error = NEW.error,
              updated_at = NEW.updated_at
          WHERE run_id = NEW.id;

          INSERT INTO schedule_notifications (
            id, project_dir, agent_name, schedule_id, occurrence_id,
            kind, message, is_read, created_at
          )
          SELECT
            lower(hex(randomblob(16))),
            NEW.project_dir,
            NEW.agent_name,
            json_extract(NEW.trigger_json, '$.scheduleId'),
            occurrence.id,
            CASE
              WHEN NEW.status = 'completed' THEN 'completed'
              WHEN NEW.status = 'waiting' THEN 'waiting'
              ELSE 'failed'
            END,
            'Scheduled workflow ' || NEW.agent_name || '/' ||
              NEW.workflow_name || ' ' || NEW.status || ' (' || NEW.id || ').',
            0,
            NEW.updated_at
          FROM schedule_occurrences AS occurrence
          WHERE occurrence.run_id = NEW.id
            AND NEW.status IN (
              'completed', 'waiting', 'failed', 'cancelled', 'version-mismatch'
            )
            AND OLD.status <> NEW.status;
        END;
        ",
    )
}

fn create_conversation_storage(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    transaction.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS agent_conversations (
          id TEXT PRIMARY KEY,
          project_dir TEXT NOT NULL,
          agent_name TEXT NOT NULL,
          mode TEXT NOT NULL,
          provider TEXT NOT NULL,
          model TEXT NOT NULL,
          history_json TEXT NOT NULL,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          UNIQUE(project_dir, agent_name)
        );
        ",
    )
}
