use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::Connection;

use crate::migrations;
use crate::{
    AgentConversationRepository, AppliedMigration, EffectRepository, HumanResponseRepository,
    NotificationRepository, OccurrenceRepository, PersistenceError, Result, ScheduleRepository,
    WorkerLeaseRepository, WorkflowRunRepository, WorkflowStepRepository,
};

const SQLITE_BUSY_TIMEOUT_MS: u64 = 5_000;

pub struct SqliteDatabase {
    path: PathBuf,
    connection: Connection,
}

#[allow(clippy::missing_errors_doc)]
impl SqliteDatabase {
    pub fn open_global_dir(global_dir: impl AsRef<Path>) -> Result<Self> {
        let global_dir = global_dir.as_ref();
        fs::create_dir_all(global_dir).map_err(|source| PersistenceError::CreateDirectory {
            path: global_dir.to_path_buf(),
            source,
        })?;
        Self::open(global_dir.join("runs.sqlite"))
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|source| PersistenceError::CreateDirectory {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let connection = Connection::open(path)?;
        Self::initialize(path.to_path_buf(), connection)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::initialize(PathBuf::from(":memory:"), Connection::open_in_memory()?)
    }

    fn initialize(path: PathBuf, mut connection: Connection) -> Result<Self> {
        connection.busy_timeout(Duration::from_millis(SQLITE_BUSY_TIMEOUT_MS))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", true)?;
        migrations::migrate(&mut connection)?;
        Ok(Self { path, connection })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn applied_migrations(&self) -> Result<Vec<AppliedMigration>> {
        migrations::applied_migrations(&self.connection)
    }

    pub const fn workflow_runs(&mut self) -> WorkflowRunRepository<'_> {
        WorkflowRunRepository::new(&mut self.connection)
    }

    pub const fn workflow_steps(&mut self) -> WorkflowStepRepository<'_> {
        WorkflowStepRepository::new(&mut self.connection)
    }

    pub const fn effects(&mut self) -> EffectRepository<'_> {
        EffectRepository::new(&mut self.connection)
    }

    pub const fn human_responses(&mut self) -> HumanResponseRepository<'_> {
        HumanResponseRepository::new(&mut self.connection)
    }

    pub const fn schedules(&mut self) -> ScheduleRepository<'_> {
        ScheduleRepository::new(&mut self.connection)
    }

    pub const fn occurrences(&mut self) -> OccurrenceRepository<'_> {
        OccurrenceRepository::new(&mut self.connection)
    }

    pub const fn worker_leases(&mut self) -> WorkerLeaseRepository<'_> {
        WorkerLeaseRepository::new(&mut self.connection)
    }

    pub const fn notifications(&mut self) -> NotificationRepository<'_> {
        NotificationRepository::new(&mut self.connection)
    }

    pub const fn agent_conversations(&mut self) -> AgentConversationRepository<'_> {
        AgentConversationRepository::new(&mut self.connection)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::SqliteDatabase;

    #[test]
    fn configures_legacy_sqlite_pragmas() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let database = SqliteDatabase::open_global_dir(directory.path())?;
        let busy_timeout = database
            .connection
            .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))?;
        let journal_mode = database
            .connection
            .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))?;
        let foreign_keys = database
            .connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))?;

        assert_eq!(busy_timeout, 5_000);
        assert_eq!(journal_mode, "wal");
        assert_eq!(foreign_keys, 1);
        Ok(())
    }
}
