use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum PersistenceError {
    #[error("SQLite operation failed: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("stored JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("could not prepare SQLite directory {path}: {source}")]
    CreateDirectory {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid stored value for {field}: {value}")]
    InvalidValue { field: &'static str, value: String },
    #[error("migration {version} ({name}) failed: {source}")]
    Migration {
        version: i64,
        name: &'static str,
        source: rusqlite::Error,
    },
    #[error("SQLite schema is too new: found migration {found}, latest supported is {latest}")]
    SchemaTooNew { found: i64, latest: i64 },
}

pub type Result<T> = std::result::Result<T, PersistenceError>;
