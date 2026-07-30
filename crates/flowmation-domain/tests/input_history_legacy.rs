use std::fs;
use std::thread;

use flowmation_domain::input_history::{InputHistory, InputHistoryStore};
use tempfile::tempdir;
use uuid::Uuid;

// Legacy: InputHistory.test.ts — navigates newest-to-oldest and restores draft.
#[test]
fn navigates_history_and_restores_current_draft() {
    let mut history = InputHistory::default();
    history.record("first");
    history.record("second");
    let mut navigator = history.start_navigation();

    assert_eq!(navigator.previous("unfinished draft"), "second");
    assert_eq!(navigator.previous("second"), "first");
    assert_eq!(navigator.previous("first"), "first");
    assert_eq!(navigator.next("first"), "second");
    assert_eq!(navigator.next("second"), "unfinished draft");
    assert_eq!(navigator.next("unfinished draft"), "unfinished draft");
}

// Legacy: InputHistory.test.ts — ignores empty input and retains only configured limit.
#[test]
fn ignores_empty_input_and_retains_configured_limit() -> Result<(), Box<dyn std::error::Error>> {
    let mut history = InputHistory::new(
        vec!["first".to_owned(), "second".to_owned(), "third".to_owned()],
        2,
    )?;
    history.record("");
    history.record("fourth");

    assert_eq!(history.snapshot(), vec!["third", "fourth"]);
    Ok(())
}

// Legacy: InputHistory.test.ts — rejects invalid limits.
#[test]
fn rejects_invalid_history_limits() {
    assert!(InputHistory::new(Vec::new(), 0).is_err());
}

// Legacy: InputHistoryStore.test.ts — returns empty when no persisted file exists.
#[test]
fn returns_empty_history_when_no_file_exists() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let store = InputHistoryStore::new(root.path());

    assert!(store.load()?.is_empty());
    Ok(())
}

// Legacy: InputHistoryStore.test.ts — persists and reloads input history.
#[test]
fn persists_and_reloads_input_history() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let store = InputHistoryStore::new(root.path());
    store.append("first", 500)?;
    store.append("/workflow deploy", 500)?;

    assert_eq!(store.load()?, vec!["first", "/workflow deploy"]);
    assert!(fs::read_to_string(store.file_path())?.contains("\"version\": 1"));
    Ok(())
}

// Legacy: InputHistoryStore.test.ts — preserves concurrent appends.
#[test]
fn preserves_entries_appended_by_concurrent_stores() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let first_store = InputHistoryStore::new(root.path());
    let second_store = InputHistoryStore::new(root.path());
    let first = thread::spawn(move || {
        first_store.append("first", 500)?;
        first_store.append("third", 500)
    });
    let second = thread::spawn(move || {
        second_store.append("second", 500)?;
        second_store.append("fourth", 500)
    });
    first
        .join()
        .map_err(|_| "first history writer panicked")??;
    second
        .join()
        .map_err(|_| "second history writer panicked")??;

    let mut entries = InputHistoryStore::new(root.path()).load()?;
    entries.sort();
    assert_eq!(entries, vec!["first", "fourth", "second", "third"]);
    Ok(())
}

// Legacy: InputHistoryStore.test.ts — reclaims abandoned bakery-lock entries.
#[test]
fn reclaims_abandoned_lock_entries_without_disturbing_concurrent_writers()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let store = InputHistoryStore::new(root.path());
    let lock_directory = {
        let mut path = store.file_path().as_os_str().to_owned();
        path.push(".locks");
        std::path::PathBuf::from(path)
    };
    fs::create_dir_all(&lock_directory)?;
    let abandoned_process_id = u32::MAX;
    fs::write(
        lock_directory.join(format!(
            "{abandoned_process_id}-{}.choosing",
            Uuid::new_v4()
        )),
        "",
    )?;
    fs::write(
        lock_directory.join(format!(
            "1-{abandoned_process_id}-{}.ticket",
            Uuid::new_v4()
        )),
        "",
    )?;

    let first_store = InputHistoryStore::new(root.path());
    let second_store = InputHistoryStore::new(root.path());
    let first = thread::spawn(move || first_store.append("first", 500));
    let second = thread::spawn(move || second_store.append("second", 500));
    first.join().map_err(|_| "first writer panicked")??;
    second.join().map_err(|_| "second writer panicked")??;

    let mut entries = store.load()?;
    entries.sort();
    assert_eq!(entries, vec!["first", "second"]);
    assert_eq!(fs::read_dir(lock_directory)?.count(), 0);
    Ok(())
}

// Legacy: InputHistoryStore.test.ts — retains only configured number of entries.
#[test]
fn persisted_history_retains_only_configured_limit() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let store = InputHistoryStore::new(root.path());
    store.append("first", 2)?;
    store.append("second", 2)?;
    store.append("third", 2)?;

    assert_eq!(store.load()?, vec!["second", "third"]);
    Ok(())
}

// Legacy: InputHistoryStore.test.ts — rejects malformed persisted history.
#[test]
fn rejects_malformed_persisted_history() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let store = InputHistoryStore::new(root.path());
    fs::write(store.file_path(), r#"{"version":1,"entries":[42]}"#)?;

    assert!(store.load().is_err());
    Ok(())
}

#[test]
fn malformed_history_is_recovered_on_append() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let store = InputHistoryStore::new(root.path());
    fs::write(store.file_path(), "not JSON")?;

    store.append("recovered", 500)?;

    assert_eq!(store.load()?, vec!["recovered"]);
    Ok(())
}
