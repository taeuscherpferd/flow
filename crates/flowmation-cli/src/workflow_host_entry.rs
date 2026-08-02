use std::ffi::OsString;
use std::path::{Path, PathBuf};

const HOST_ENTRY: &str = "workflow-host/dist/index.js";

#[cfg(any(test, not(debug_assertions)))]
struct EmbeddedWorkflowHostFile {
    path: &'static str,
    contents: &'static [u8],
    executable: bool,
}

#[cfg(not(debug_assertions))]
include!(concat!(env!("OUT_DIR"), "/embedded_workflow_host.rs"));

pub fn entry_path() -> Result<PathBuf, String> {
    let configured = std::env::var_os("FLOWMATION_WORKFLOW_HOST");
    let executable = std::env::current_exe().ok();
    resolve_entry(configured, executable.as_deref())
}

fn resolve_entry(
    configured: Option<OsString>,
    executable: Option<&Path>,
) -> Result<PathBuf, String> {
    if let Some(configured) = configured {
        return Ok(PathBuf::from(configured));
    }
    if let Some(directory) = executable.and_then(Path::parent) {
        let packaged = directory.join(HOST_ENTRY);
        if packaged.is_file() {
            return Ok(packaged);
        }
    }
    debug_or_embedded_entry()
}

#[cfg(debug_assertions)]
fn debug_or_embedded_entry() -> Result<PathBuf, String> {
    Ok(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../workflow-host/dist/index.js"))
}

#[cfg(not(debug_assertions))]
fn debug_or_embedded_entry() -> Result<PathBuf, String> {
    let runtime_directory = std::env::var_os("FLOWMATION_RUNTIME_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".work-agent/runtime"))
        })
        .unwrap_or_else(|| std::env::temp_dir().join("flowmation-runtime"));
    extract_embedded_host(
        &runtime_directory,
        EMBEDDED_WORKFLOW_HOST_ID,
        EMBEDDED_WORKFLOW_HOST_FILES,
    )
}

#[cfg(any(test, not(debug_assertions)))]
fn extract_embedded_host(
    runtime_directory: &Path,
    host_id: &str,
    files: &[EmbeddedWorkflowHostFile],
) -> Result<PathBuf, String> {
    let host_directory = runtime_directory.join("workflow-host").join(host_id);
    let host_entry = host_directory.join("dist/index.js");
    if host_entry.is_file() {
        return Ok(host_entry);
    }

    let parent = host_directory
        .parent()
        .ok_or_else(|| "The workflow host runtime path has no parent directory.".to_owned())?;
    std::fs::create_dir_all(parent).map_err(|error| {
        format!(
            "Could not create workflow host runtime directory {}: {error}",
            parent.display()
        )
    })?;
    let temporary_directory = create_temporary_directory(parent, host_id)?;

    let extraction_result = (|| {
        for file in files {
            let destination = temporary_directory.join(file.path);
            let destination_parent = destination.parent().ok_or_else(|| {
                format!("Embedded workflow host path has no parent: {}", file.path)
            })?;
            std::fs::create_dir_all(destination_parent).map_err(|error| {
                format!(
                    "Could not create embedded workflow host directory {}: {error}",
                    destination_parent.display()
                )
            })?;
            std::fs::write(&destination, file.contents).map_err(|error| {
                format!(
                    "Could not extract embedded workflow host file {}: {error}",
                    destination.display()
                )
            })?;
            set_executable(&destination, file.executable)?;
        }
        std::fs::rename(&temporary_directory, &host_directory).map_err(|error| {
            format!(
                "Could not activate embedded workflow host at {}: {error}",
                host_directory.display()
            )
        })
    })();

    if let Err(error) = extraction_result {
        if host_entry.is_file() {
            let _result = std::fs::remove_dir_all(&temporary_directory);
            return Ok(host_entry);
        }
        let _result = std::fs::remove_dir_all(&temporary_directory);
        return Err(error);
    }
    Ok(host_entry)
}

#[cfg(any(test, not(debug_assertions)))]
fn create_temporary_directory(parent: &Path, host_id: &str) -> Result<PathBuf, String> {
    for attempt in 0..100 {
        let candidate = parent.join(format!(".{host_id}-{}-{attempt}.tmp", std::process::id()));
        match std::fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(format!(
                    "Could not create temporary workflow host directory {}: {error}",
                    candidate.display()
                ));
            }
        }
    }
    Err("Could not allocate a temporary workflow host directory.".to_owned())
}

#[cfg(all(unix, any(test, not(debug_assertions))))]
fn set_executable(path: &Path, executable: bool) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    if !executable {
        return Ok(());
    }
    let mut permissions = std::fs::metadata(path)
        .map_err(|error| format!("Could not inspect {}: {error}", path.display()))?
        .permissions();
    permissions.set_mode(permissions.mode() | 0o755);
    std::fs::set_permissions(path, permissions)
        .map_err(|error| format!("Could not make {} executable: {error}", path.display()))
}

#[cfg(all(not(unix), any(test, not(debug_assertions))))]
fn set_executable(_path: &Path, _executable: bool) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    #[test]
    fn configured_path_takes_precedence() {
        let configured = OsString::from("/configured/workflow-host.js");
        let resolved = resolve_entry(Some(configured), Some(Path::new("/release/flowmation")))
            .expect("configured path should resolve");

        assert_eq!(resolved, PathBuf::from("/configured/workflow-host.js"));
    }

    #[test]
    fn debug_path_uses_the_checkout_when_a_package_is_unavailable() {
        let resolved = resolve_entry(None, Some(Path::new("/missing/flowmation")))
            .expect("debug path should resolve");

        assert_eq!(
            resolved,
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../workflow-host/dist/index.js")
        );
    }

    #[test]
    fn extracts_an_embedded_host() {
        let runtime = tempfile::tempdir().expect("runtime directory should be created");
        let files = [
            EmbeddedWorkflowHostFile {
                path: "dist/index.js",
                contents: b"host",
                executable: false,
            },
            EmbeddedWorkflowHostFile {
                path: "node_modules/runtime/package.json",
                contents: b"{}",
                executable: false,
            },
        ];

        let entry = extract_embedded_host(runtime.path(), "test-host", &files)
            .expect("embedded host should extract");

        assert_eq!(
            std::fs::read_to_string(entry).expect("entry should be readable"),
            "host"
        );
        assert!(
            runtime
                .path()
                .join("workflow-host/test-host/node_modules/runtime/package.json")
                .is_file()
        );
    }

    #[cfg(not(debug_assertions))]
    #[tokio::test]
    async fn embedded_release_host_loads_typescript() {
        let runtime = tempfile::tempdir().expect("runtime directory should be created");
        let entry = extract_embedded_host(
            runtime.path(),
            EMBEDDED_WORKFLOW_HOST_ID,
            EMBEDDED_WORKFLOW_HOST_FILES,
        )
        .expect("release host should extract");
        let workflow_directory = runtime.path().join("workflows/typed");
        std::fs::create_dir_all(&workflow_directory).expect("workflow directory should be created");
        let workflow_entry = workflow_directory.join("WORKFLOW.ts");
        std::fs::write(
            &workflow_entry,
            r#"
                import { defineWorkflow } from "flowmation/workflow";
                interface Input { value: string }
                export default defineWorkflow<Input>({
                    name: "typed",
                    description: "Embedded TypeScript workflow",
                    async run(_context, input) {
                        return input.value;
                    },
                });
            "#,
        )
        .expect("workflow should be written");
        let host = flowmation_workflow_host::WorkflowHost::spawn_without_callbacks(
            flowmation_workflow_host::WorkflowHostConfig::new(entry),
        )
        .await
        .expect("embedded host should start");

        let inspected = host
            .inspect(flowmation_workflow_host::protocol::InspectWorkflowParams {
                entry_path: workflow_entry.to_string_lossy().into_owned(),
            })
            .await
            .expect("embedded host should inspect TypeScript");

        assert_eq!(inspected.metadata.name, "typed");
        host.shutdown().await.expect("embedded host should stop");
    }
}
