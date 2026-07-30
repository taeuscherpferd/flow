use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FingerprintError {
    #[error("Symbolic links are not allowed for fingerprinted directories: {0}")]
    SymbolicLinkRoot(String),
    #[error("Fingerprint target is not a directory: {0}")]
    NotDirectory(String),
    #[error("Symbolic links are not allowed in fingerprinted directories: {0}")]
    SymbolicLinkEntry(String),
    #[error("Unsupported filesystem entry in fingerprinted directory: {0}")]
    UnsupportedEntry(String),
    #[error("Could not inspect fingerprint path {path}: {source}")]
    Inspect {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("Could not read fingerprint file {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// Computes the legacy SHA-256 package fingerprint.
///
/// # Errors
///
/// Returns [`FingerprintError`] when the target is not a directory, contains a
/// symbolic link or unsupported entry, or cannot be read.
pub fn fingerprint_directory(directory: &Path) -> Result<String, FingerprintError> {
    let files = list_regular_files(directory)?;
    let mut hash = Sha256::new();
    for file in files {
        let portable_path = file
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        hash.update(portable_path.as_bytes());
        hash.update([0]);
        let absolute_path = directory.join(&file);
        let contents = fs::read(&absolute_path).map_err(|source| FingerprintError::Read {
            path: absolute_path.display().to_string(),
            source,
        })?;
        hash.update(contents);
        hash.update([0]);
    }
    Ok(hex::encode(hash.finalize()))
}

/// Lists fingerprinted files in the legacy deterministic path order.
///
/// # Errors
///
/// Returns [`FingerprintError`] when the target is not a directory, contains a
/// symbolic link or unsupported entry, or cannot be inspected.
pub fn list_regular_files(directory: &Path) -> Result<Vec<PathBuf>, FingerprintError> {
    let metadata = fs::symlink_metadata(directory).map_err(|source| FingerprintError::Inspect {
        path: directory.display().to_string(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(FingerprintError::SymbolicLinkRoot(
            directory.display().to_string(),
        ));
    }
    if !metadata.is_dir() {
        return Err(FingerprintError::NotDirectory(
            directory.display().to_string(),
        ));
    }

    let mut files = Vec::new();
    collect_regular_files(directory, Path::new(""), &mut files)?;
    files.sort_by(|left, right| {
        legacy_path_compare(&portable_sort_key(left), &portable_sort_key(right))
    });
    Ok(files)
}

fn collect_regular_files(
    directory: &Path,
    relative: &Path,
    files: &mut Vec<PathBuf>,
) -> Result<(), FingerprintError> {
    let current = directory.join(relative);
    let entries = fs::read_dir(&current).map_err(|source| FingerprintError::Inspect {
        path: current.display().to_string(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| FingerprintError::Inspect {
            path: current.display().to_string(),
            source,
        })?;
        let child = relative.join(entry.file_name());
        let child_path = directory.join(&child);
        let file_type = entry
            .file_type()
            .map_err(|source| FingerprintError::Inspect {
                path: child_path.display().to_string(),
                source,
            })?;
        if file_type.is_symlink() {
            return Err(FingerprintError::SymbolicLinkEntry(
                child_path.display().to_string(),
            ));
        }
        if file_type.is_dir() {
            collect_regular_files(directory, &child, files)?;
        } else if file_type.is_file() {
            files.push(child);
        } else {
            return Err(FingerprintError::UnsupportedEntry(
                child_path.display().to_string(),
            ));
        }
    }
    Ok(())
}

fn portable_sort_key(path: &Path) -> String {
    path.to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
}

fn legacy_path_compare(left: &str, right: &str) -> Ordering {
    let primary = left
        .chars()
        .map(primary_collation_weight)
        .cmp(right.chars().map(primary_collation_weight));
    if primary != Ordering::Equal {
        return primary;
    }
    let case = left
        .chars()
        .map(case_collation_weight)
        .cmp(right.chars().map(case_collation_weight));
    if case != Ordering::Equal {
        return case;
    }
    left.cmp(right)
}

fn primary_collation_weight(character: char) -> u32 {
    let character = character.to_ascii_lowercase();
    match character {
        ' ' => 0,
        '_' => 1,
        '-' => 2,
        ',' => 3,
        ';' => 4,
        ':' => 5,
        '!' => 6,
        '?' => 7,
        '.' => 8,
        '\'' => 9,
        '"' => 10,
        '(' => 11,
        ')' => 12,
        '[' => 13,
        ']' => 14,
        '{' => 15,
        '}' => 16,
        '@' => 17,
        '*' => 18,
        '/' => 19,
        '\\' => 20,
        '&' => 21,
        '#' => 22,
        '%' => 23,
        '`' => 24,
        '^' => 25,
        '+' => 26,
        '<' => 27,
        '=' => 28,
        '>' => 29,
        '|' => 30,
        '~' => 31,
        '$' => 32,
        '0'..='9' => 33 + u32::from(character) - u32::from('0'),
        'a'..='z' => 43 + u32::from(character) - u32::from('a'),
        _ => 69 + u32::from(character),
    }
}

const fn case_collation_weight(character: char) -> u8 {
    if character.is_ascii_uppercase() { 1 } else { 0 }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{FingerprintError, fingerprint_directory, list_regular_files};

    #[test]
    fn hashes_relative_paths_and_contents_in_deterministic_order()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        fs::create_dir(root.path().join("nested"))?;
        fs::write(root.path().join("z.txt"), b"last")?;
        fs::write(root.path().join("nested/a.txt"), b"first")?;

        let files = list_regular_files(root.path())?;
        let fingerprint = fingerprint_directory(root.path())?;

        assert_eq!(
            files,
            vec![
                std::path::PathBuf::from("nested/a.txt"),
                std::path::PathBuf::from("z.txt")
            ]
        );
        assert_eq!(
            fingerprint,
            "e8f3530ccb6e9dd00b036f5f18146a3a93bd640a31682754f86454b4a5394fe8"
        );
        Ok(())
    }

    // Legacy: AgentPackageRegistry.test.ts — rejects symbolic links anywhere in a package.
    #[cfg(unix)]
    #[test]
    fn rejects_symbolic_links_anywhere_in_directory() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::symlink;

        let root = tempdir()?;
        let external = root.path().join("external.txt");
        let package = root.path().join("package");
        fs::create_dir(&package)?;
        fs::write(&external, "external")?;
        symlink(&external, package.join("linked.txt"))?;

        let error = fingerprint_directory(&package);

        assert!(matches!(error, Err(FingerprintError::SymbolicLinkEntry(_))));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn rejects_a_symbolic_link_root() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::symlink;

        let root = tempdir()?;
        let package = root.path().join("package");
        let linked_package = root.path().join("linked-package");
        fs::create_dir(&package)?;
        symlink(&package, &linked_package)?;

        let error = list_regular_files(&linked_package);

        assert!(matches!(error, Err(FingerprintError::SymbolicLinkRoot(_))));
        Ok(())
    }

    #[test]
    fn uses_legacy_node_locale_order_for_ascii_package_paths()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        fs::create_dir_all(root.path().join("context"))?;
        fs::create_dir_all(root.path().join("skills/reconcile"))?;
        for path in [
            "SOUL.md",
            "context/policy.md",
            "AGENTS.md",
            "skills/reconcile/SKILL.md",
            "AGENT.yaml",
            "CONTEXT.md",
        ] {
            fs::write(root.path().join(path), "")?;
        }

        assert_eq!(
            list_regular_files(root.path())?,
            vec![
                std::path::PathBuf::from("AGENT.yaml"),
                std::path::PathBuf::from("AGENTS.md"),
                std::path::PathBuf::from("CONTEXT.md"),
                std::path::PathBuf::from("context/policy.md"),
                std::path::PathBuf::from("skills/reconcile/SKILL.md"),
                std::path::PathBuf::from("SOUL.md"),
            ]
        );
        assert_eq!(
            fingerprint_directory(root.path())?,
            "9a37696b508ed25d0e9715b902cfce45d961509dfc6e2d9ea24305a968bb680e"
        );
        Ok(())
    }
}
