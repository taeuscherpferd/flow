mod conversation;
mod schedule;
mod workflow;

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use crate::{Result, SqliteDatabase};

pub struct SqliteApplicationRepository {
    database: Mutex<SqliteDatabase>,
}

impl SqliteApplicationRepository {
    /// Opens and migrates the shared `runs.sqlite` database.
    ///
    /// # Errors
    ///
    /// Returns an error when the directory, `SQLite` configuration, or a migration fails.
    pub fn open_global_dir(global_dir: impl AsRef<Path>) -> Result<Self> {
        SqliteDatabase::open_global_dir(global_dir).map(Self::from_database)
    }

    #[must_use]
    pub const fn from_database(database: SqliteDatabase) -> Self {
        Self {
            database: Mutex::new(database),
        }
    }

    fn database(&self) -> std::result::Result<MutexGuard<'_, SqliteDatabase>, String> {
        self.database
            .lock()
            .map_err(|_| "SQLite database mutex was poisoned.".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::SqliteApplicationRepository;

    fn require_send_sync<T: Send + Sync>() {}

    #[test]
    fn adapter_is_thread_safe() {
        require_send_sync::<SqliteApplicationRepository>();
    }
}
