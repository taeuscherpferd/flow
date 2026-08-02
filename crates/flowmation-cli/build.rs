use std::collections::hash_map::DefaultHasher;
use std::env;
use std::error::Error;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = required_path("CARGO_MANIFEST_DIR")?;
    let workspace_dir = manifest_dir.join("../..");
    let out_dir = required_path("OUT_DIR")?;
    let profile_dir = profile_directory(&out_dir)?;

    println!("cargo:rerun-if-changed=../../pnpm-lock.yaml");
    println!("cargo:rerun-if-changed=../../workflow-host/package.json");
    println!("cargo:rerun-if-changed=../../workflow-host/tsconfig.json");
    println!("cargo:rerun-if-changed=../../workflow-host/src");
    println!("cargo:rerun-if-changed=../../scripts/stage-workflow-host.mjs");

    run(
        Command::new(pnpm_executable())
            .current_dir(&workspace_dir)
            .args(["--dir", "workflow-host", "run", "build"]),
        "build the JavaScript workflow host",
    )?;
    run(
        Command::new("node")
            .current_dir(&workspace_dir)
            .arg("scripts/stage-workflow-host.mjs")
            .arg(profile_dir),
        "stage the JavaScript workflow host",
    )?;
    write_embedded_manifest(
        &profile_dir.join("workflow-host"),
        &out_dir.join("embedded_workflow_host.rs"),
    )
}

fn required_path(name: &str) -> Result<PathBuf, Box<dyn Error>> {
    env::var_os(name).map(PathBuf::from).ok_or_else(|| {
        Box::new(io::Error::other(format!(
            "Cargo did not provide the {name} environment variable"
        ))) as Box<dyn Error>
    })
}

fn profile_directory(out_dir: &Path) -> Result<&Path, Box<dyn Error>> {
    out_dir.ancestors().nth(3).ok_or_else(|| {
        Box::new(io::Error::other(format!(
            "could not derive the Cargo profile directory from {}",
            out_dir.display()
        ))) as Box<dyn Error>
    })
}

fn run(command: &mut Command, operation: &str) -> Result<(), Box<dyn Error>> {
    let status = command.status()?;
    if status.success() {
        return Ok(());
    }
    Err(Box::new(io::Error::other(format!(
        "failed to {operation}: process exited with {status}"
    ))))
}

fn write_embedded_manifest(
    host_directory: &Path,
    manifest_path: &Path,
) -> Result<(), Box<dyn Error>> {
    let mut files = Vec::new();
    collect_files(host_directory, &mut files)?;
    files.sort();

    let mut hasher = DefaultHasher::new();
    let mut manifest = fs::File::create(manifest_path)?;
    writeln!(
        manifest,
        "const EMBEDDED_WORKFLOW_HOST_FILES: &[EmbeddedWorkflowHostFile] = &["
    )?;

    for file_path in files {
        let relative_path = file_path
            .strip_prefix(host_directory)?
            .to_string_lossy()
            .replace('\\', "/");
        let contents = fs::read(&file_path)?;
        relative_path.hash(&mut hasher);
        contents.hash(&mut hasher);
        writeln!(
            manifest,
            "    EmbeddedWorkflowHostFile {{ path: {relative_path:?}, contents: include_bytes!({:?}), executable: {} }},",
            file_path.to_string_lossy(),
            is_executable(&file_path)?
        )?;
    }

    writeln!(manifest, "];")?;
    writeln!(
        manifest,
        "const EMBEDDED_WORKFLOW_HOST_ID: &str = \"{:016x}\";",
        hasher.finish()
    )?;
    Ok(())
}

fn collect_files(directory: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_files(&path, files)?;
        } else if file_type.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

#[cfg(unix)]
fn is_executable(path: &Path) -> io::Result<bool> {
    use std::os::unix::fs::PermissionsExt;

    Ok(fs::metadata(path)?.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(_path: &Path) -> io::Result<bool> {
    Ok(false)
}

#[cfg(windows)]
fn pnpm_executable() -> &'static str {
    "pnpm.cmd"
}

#[cfg(not(windows))]
fn pnpm_executable() -> &'static str {
    "pnpm"
}
