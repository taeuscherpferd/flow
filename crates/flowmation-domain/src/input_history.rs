use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub const DEFAULT_INPUT_HISTORY_LIMIT: usize = 500;
const HISTORY_FILE_NAME: &str = "input-history.json";
const HISTORY_FILE_VERSION: u8 = 1;
const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(10);
const LOCK_WAIT_TIMEOUT: Duration = Duration::from_secs(5);
const LOCK_CHOOSING_SUFFIX: &str = ".choosing";
const LOCK_TICKET_SUFFIX: &str = ".ticket";

#[derive(Clone, Debug)]
pub struct InputHistory {
    entries: Vec<String>,
    limit: usize,
}

#[derive(Clone, Debug)]
pub struct InputHistoryNavigator {
    entries: Vec<String>,
    index: usize,
    draft: String,
}

#[derive(Clone, Debug)]
pub struct InputHistoryStore {
    file_path: PathBuf,
    lock_directory_path: PathBuf,
}

#[derive(Debug, Error)]
pub enum InputHistoryError {
    #[error("Input history limit must be a positive integer.")]
    InvalidLimit,
    #[error("Failed to parse {path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("Timed out waiting to update {0}.")]
    LockTimeout(PathBuf),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Serialize(#[from] serde_json::Error),
}

#[derive(Debug, Deserialize, Serialize)]
struct SerializedInputHistory {
    version: u8,
    entries: Vec<String>,
}

#[derive(Debug)]
struct LockEntry {
    file_name: String,
    process_id: u32,
    ticket: Option<u64>,
}

#[derive(Debug)]
struct HistoryLock {
    ticket_path: PathBuf,
}

impl Drop for HistoryLock {
    fn drop(&mut self) {
        drop(fs::remove_file(&self.ticket_path));
    }
}

impl Default for InputHistory {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            limit: DEFAULT_INPUT_HISTORY_LIMIT,
        }
    }
}

impl InputHistory {
    /// Builds bounded input history from existing entries.
    ///
    /// # Errors
    ///
    /// Returns [`InputHistoryError::InvalidLimit`] when `limit` is zero.
    pub fn new(
        initial_entries: impl IntoIterator<Item = String>,
        limit: usize,
    ) -> Result<Self, InputHistoryError> {
        if limit == 0 {
            return Err(InputHistoryError::InvalidLimit);
        }
        let mut history = Self {
            entries: Vec::new(),
            limit,
        };
        for entry in initial_entries {
            history.record(entry);
        }
        Ok(history)
    }

    pub fn record(&mut self, input: impl Into<String>) {
        let input = input.into();
        if input.is_empty() {
            return;
        }
        self.entries.push(input);
        if self.entries.len() > self.limit {
            let excess = self.entries.len() - self.limit;
            self.entries.drain(..excess);
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> Vec<String> {
        self.entries.clone()
    }

    #[must_use]
    pub const fn limit(&self) -> usize {
        self.limit
    }

    #[must_use]
    pub fn start_navigation(&self) -> InputHistoryNavigator {
        InputHistoryNavigator {
            entries: self.entries.clone(),
            index: self.entries.len(),
            draft: String::new(),
        }
    }
}

impl InputHistoryNavigator {
    pub fn previous(&mut self, current_input: &str) -> String {
        if self.index == 0 {
            return current_input.to_owned();
        }
        if self.index == self.entries.len() {
            current_input.clone_into(&mut self.draft);
        }
        self.index -= 1;
        self.entries
            .get(self.index)
            .cloned()
            .unwrap_or_else(|| current_input.to_owned())
    }

    pub fn next(&mut self, current_input: &str) -> String {
        if self.index == self.entries.len() {
            return current_input.to_owned();
        }
        self.index += 1;
        if self.index == self.entries.len() {
            self.draft.clone()
        } else {
            self.entries
                .get(self.index)
                .cloned()
                .unwrap_or_else(|| current_input.to_owned())
        }
    }
}

impl InputHistoryStore {
    #[must_use]
    pub fn new(global_dir: impl AsRef<Path>) -> Self {
        let file_path = global_dir.as_ref().join(HISTORY_FILE_NAME);
        let lock_directory_path = append_to_path(&file_path, ".locks");
        Self {
            file_path,
            lock_directory_path,
        }
    }

    #[must_use]
    pub fn file_path(&self) -> &Path {
        &self.file_path
    }

    /// Loads the versioned legacy history JSON file.
    ///
    /// # Errors
    ///
    /// Returns [`InputHistoryError`] when the file cannot be read or has an
    /// unsupported or malformed representation.
    pub fn load(&self) -> Result<Vec<String>, InputHistoryError> {
        let raw = match fs::read_to_string(&self.file_path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let parsed: SerializedInputHistory =
            serde_json::from_str(&raw).map_err(|error| InputHistoryError::Parse {
                path: self.file_path.clone(),
                message: error.to_string(),
            })?;
        if parsed.version != HISTORY_FILE_VERSION {
            return Err(InputHistoryError::Parse {
                path: self.file_path.clone(),
                message: "Unsupported input history format.".to_owned(),
            });
        }
        Ok(parsed.entries)
    }

    /// Atomically appends an entry while holding the cross-instance file lock.
    ///
    /// # Errors
    ///
    /// Returns [`InputHistoryError`] for an invalid limit, lock timeout, or
    /// persistence failure.
    pub fn append(&self, input: &str, limit: usize) -> Result<(), InputHistoryError> {
        if input.is_empty() {
            return Ok(());
        }
        if limit == 0 {
            return Err(InputHistoryError::InvalidLimit);
        }
        if let Some(parent) = self.file_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let _lock = self.acquire_lock()?;
        self.append_while_locked(input, limit)
    }

    fn append_while_locked(&self, input: &str, limit: usize) -> Result<(), InputHistoryError> {
        let mut entries = match self.load() {
            Ok(entries) => entries,
            Err(InputHistoryError::Parse { .. }) => Vec::new(),
            Err(error) => return Err(error),
        };
        entries.push(input.to_owned());
        let first_retained = entries.len().saturating_sub(limit);
        self.write_atomically(&entries[first_retained..])
    }

    fn acquire_lock(&self) -> Result<HistoryLock, InputHistoryError> {
        fs::create_dir_all(&self.lock_directory_path)?;
        let participant_id = format!("{}-{}", std::process::id(), Uuid::new_v4());
        let choosing_path = self
            .lock_directory_path
            .join(format!("{participant_id}{LOCK_CHOOSING_SUFFIX}"));
        create_private_file(&choosing_path)?;
        let mut created_ticket_path = None;
        let result = (|| {
            self.remove_abandoned_lock_entries()?;
            let highest_ticket = self
                .list_lock_entries()?
                .into_iter()
                .filter_map(|entry| entry.ticket)
                .max()
                .unwrap_or(0);
            let ticket = highest_ticket.checked_add(1).ok_or_else(|| {
                InputHistoryError::Io(std::io::Error::other(format!(
                    "Could not allocate a lock ticket for {}.",
                    self.file_path.display()
                )))
            })?;
            let ticket_path = self
                .lock_directory_path
                .join(format!("{ticket}-{participant_id}{LOCK_TICKET_SUFFIX}"));
            create_private_file(&ticket_path)?;
            created_ticket_path = Some(ticket_path.clone());
            remove_file_if_exists(&choosing_path)?;

            let started = Instant::now();
            loop {
                self.remove_abandoned_lock_entries()?;
                let entries = self.list_lock_entries()?;
                let another_process_is_choosing =
                    entries.iter().any(|entry| entry.ticket.is_none());
                let first_ticket = entries
                    .iter()
                    .filter_map(|entry| entry.ticket.map(|ticket| (ticket, &entry.file_name)))
                    .min();
                if !another_process_is_choosing
                    && first_ticket.is_some_and(|(_, file_name)| {
                        ticket_path.file_name().and_then(|value| value.to_str()) == Some(file_name)
                    })
                {
                    return Ok(ticket_path);
                }
                if started.elapsed() >= LOCK_WAIT_TIMEOUT {
                    return Err(InputHistoryError::LockTimeout(self.file_path.clone()));
                }
                thread::sleep(LOCK_RETRY_INTERVAL);
            }
        })();
        drop(remove_file_if_exists(&choosing_path));
        if result.is_err()
            && let Some(ticket_path) = created_ticket_path
        {
            drop(remove_file_if_exists(&ticket_path));
        }
        match result {
            Ok(ticket_path) => Ok(HistoryLock { ticket_path }),
            Err(error) => Err(error),
        }
    }

    fn remove_abandoned_lock_entries(&self) -> Result<(), InputHistoryError> {
        for entry in fs::read_dir(&self.lock_directory_path)? {
            let entry = entry?;
            let file_name = entry.file_name().to_string_lossy().into_owned();
            if !file_name.ends_with(LOCK_CHOOSING_SUFFIX)
                && !file_name.ends_with(LOCK_TICKET_SUFFIX)
            {
                continue;
            }
            let active = parse_lock_entry(&file_name)
                .is_some_and(|entry| is_process_running(entry.process_id));
            if !active {
                remove_file_if_exists(&entry.path())?;
            }
        }
        Ok(())
    }

    fn list_lock_entries(&self) -> Result<Vec<LockEntry>, InputHistoryError> {
        let mut entries = Vec::new();
        for entry in fs::read_dir(&self.lock_directory_path)? {
            let entry = entry?;
            if let Some(lock_entry) = parse_lock_entry(&entry.file_name().to_string_lossy()) {
                entries.push(lock_entry);
            }
        }
        Ok(entries)
    }

    fn write_atomically(&self, entries: &[String]) -> Result<(), InputHistoryError> {
        let history = SerializedInputHistory {
            version: HISTORY_FILE_VERSION,
            entries: entries.to_vec(),
        };
        let mut contents = serde_json::to_string_pretty(&history)?;
        contents.push('\n');
        let temporary_path = append_to_path(&self.file_path, &format!(".{}.tmp", Uuid::new_v4()));
        let write_result = write_private_file(&temporary_path, contents.as_bytes())
            .and_then(|()| fs::rename(&temporary_path, &self.file_path));
        if write_result.is_err() {
            let cleanup_result = fs::remove_file(&temporary_path);
            drop(cleanup_result);
        }
        write_result.map_err(InputHistoryError::Io)
    }
}

fn append_to_path(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_owned();
    value.push(suffix);
    PathBuf::from(value)
}

fn parse_lock_entry(file_name: &str) -> Option<LockEntry> {
    if let Some(value) = file_name.strip_suffix(LOCK_CHOOSING_SUFFIX) {
        let (process_id, participant_id) = value.split_once('-')?;
        let process_id = process_id.parse::<u32>().ok()?;
        Uuid::parse_str(participant_id).ok()?;
        return (process_id > 0).then(|| LockEntry {
            file_name: file_name.to_owned(),
            process_id,
            ticket: None,
        });
    }
    let value = file_name.strip_suffix(LOCK_TICKET_SUFFIX)?;
    let mut parts = value.splitn(3, '-');
    let ticket = parts.next()?.parse::<u64>().ok()?;
    let process_id = parts.next()?.parse::<u32>().ok()?;
    Uuid::parse_str(parts.next()?).ok()?;
    (ticket > 0 && process_id > 0).then(|| LockEntry {
        file_name: file_name.to_owned(),
        process_id,
        ticket: Some(ticket),
    })
}

#[cfg(unix)]
fn is_process_running(process_id: u32) -> bool {
    use nix::errno::Errno;
    use nix::sys::signal::kill;
    use nix::unistd::Pid;

    let Ok(process_id) = i32::try_from(process_id) else {
        return false;
    };
    match kill(Pid::from_raw(process_id), None) {
        Ok(()) | Err(Errno::EPERM) => true,
        Err(Errno::ESRCH) => false,
        Err(_) => true,
    }
}

#[cfg(not(unix))]
fn is_process_running(process_id: u32) -> bool {
    std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {process_id}"), "/FO", "CSV", "/NH"])
        .output()
        .is_ok_and(|output| {
            output.status.success()
                && String::from_utf8_lossy(&output.stdout)
                    .split(',')
                    .nth(1)
                    .is_some_and(|value| value.trim_matches('"') == process_id.to_string())
        })
}

#[cfg(unix)]
fn create_private_file(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map(drop)
}

#[cfg(not(unix))]
fn create_private_file(path: &Path) -> std::io::Result<()> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map(drop)
}

#[cfg(unix)]
fn write_private_file(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(contents)
}

#[cfg(not(unix))]
fn write_private_file(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    file.write_all(contents)
}

fn remove_file_if_exists(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}
