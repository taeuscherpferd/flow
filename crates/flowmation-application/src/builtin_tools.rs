use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Map, Value};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

use crate::tool::{
    Tool, ToolEffect, ToolExecutionContext, ToolResult, object_schema, string_schema_property,
};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_OUTPUT_BYTES: usize = 500_000;

#[derive(Debug)]
pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read the contents of a text file at the given path."
    }

    fn parameters(&self) -> flowmation_domain::chat::JsonSchema {
        object_schema(
            [(
                "path",
                string_schema_property(Some(
                    "File path, absolute or relative to the current working directory.".to_owned(),
                )),
            )],
            ["path"],
        )
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Read
    }

    async fn execute(
        &self,
        arguments: Map<String, Value>,
        context: &ToolExecutionContext,
    ) -> ToolResult {
        let Some(path) = nonempty_string(&arguments, "path") else {
            return ToolResult::failure("Error: 'path' must be a non-empty string.");
        };
        let resolved = context.cwd.join(path);
        tokio::select! {
            () = context.cancellation.cancelled() => ToolResult::failure("File read cancelled."),
            result = tokio::fs::read_to_string(&resolved) => match result {
                Ok(content) => ToolResult::success(content),
                Err(error) => ToolResult::failure(format!("Error reading \"{path}\": {error}")),
            }
        }
    }
}

#[derive(Debug)]
pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file at the given path, creating parent directories as needed. \
         Overwrites existing files."
    }

    fn parameters(&self) -> flowmation_domain::chat::JsonSchema {
        object_schema(
            [
                ("path", string_schema_property(None)),
                ("content", string_schema_property(None)),
            ],
            ["path", "content"],
        )
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Write
    }

    async fn execute(
        &self,
        arguments: Map<String, Value>,
        context: &ToolExecutionContext,
    ) -> ToolResult {
        let Some(path) = nonempty_string(&arguments, "path") else {
            return ToolResult::failure("Error: 'path' must be a non-empty string.");
        };
        let Some(content) = arguments.get("content").and_then(Value::as_str) else {
            return ToolResult::failure("Error: 'content' must be a string.");
        };
        let resolved = context.cwd.join(path);
        let Some(parent) = resolved.parent() else {
            return ToolResult::failure(format!("Error writing \"{path}\": invalid parent path"));
        };
        if context.cancellation.is_cancelled() {
            return ToolResult::failure("File write cancelled.");
        }
        if let Err(error) = tokio::fs::create_dir_all(parent).await {
            return ToolResult::failure(format!("Error writing \"{path}\": {error}"));
        }
        tokio::select! {
            () = context.cancellation.cancelled() => ToolResult::failure("File write cancelled."),
            result = tokio::fs::write(&resolved, content) => match result {
                Ok(()) => ToolResult::success(format!(
                    "Wrote {} characters to \"{path}\".",
                    content.chars().count()
                )),
                Err(error) => ToolResult::failure(format!("Error writing \"{path}\": {error}")),
            }
        }
    }
}

#[derive(Debug)]
pub struct RunCommandTool;

#[async_trait]
impl Tool for RunCommandTool {
    fn name(&self) -> &str {
        "run_command"
    }

    fn description(&self) -> &str {
        "Execute a shell command and return its stdout, stderr, and exit code. Times out after 30 \
         seconds; output is capped at 500KB."
    }

    fn parameters(&self) -> flowmation_domain::chat::JsonSchema {
        object_schema(
            [(
                "command",
                string_schema_property(Some("The shell command to execute.".to_owned())),
            )],
            ["command"],
        )
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Command
    }

    async fn execute(
        &self,
        arguments: Map<String, Value>,
        context: &ToolExecutionContext,
    ) -> ToolResult {
        let Some(command_text) = nonempty_string(&arguments, "command") else {
            return ToolResult::failure("Error: 'command' must be a non-empty string.");
        };
        match execute_command(command_text, context).await {
            Ok(output) => ToolResult {
                ok: output.exit_code == Some(0),
                content: format_command_output(&output),
            },
            Err(CommandFailure::Cancelled) => ToolResult::failure("Command cancelled."),
            Err(CommandFailure::TimedOut(output)) => ToolResult::failure(format!(
                "Command timed out after {}s.\n\n{}",
                COMMAND_TIMEOUT.as_secs(),
                format_command_output(&output)
            )),
            Err(CommandFailure::Io(error)) => {
                ToolResult::failure(format!("Error starting command: {error}"))
            }
        }
    }
}

struct CommandOutput {
    exit_code: Option<i32>,
    stdout: CapturedOutput,
    stderr: CapturedOutput,
}

struct CapturedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

enum CommandFailure {
    Cancelled,
    TimedOut(CommandOutput),
    Io(std::io::Error),
}

async fn execute_command(
    command_text: &str,
    context: &ToolExecutionContext,
) -> Result<CommandOutput, CommandFailure> {
    let mut command = shell_command(command_text);
    command
        .current_dir(&context.cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn().map_err(CommandFailure::Io)?;
    let pid = child.id();
    let stdout = child.stdout.take().ok_or_else(|| {
        CommandFailure::Io(std::io::Error::other("command stdout pipe was not created"))
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        CommandFailure::Io(std::io::Error::other("command stderr pipe was not created"))
    })?;
    let stdout_task = tokio::spawn(read_capped(stdout));
    let stderr_task = tokio::spawn(read_capped(stderr));
    let outcome = tokio::select! {
        () = context.cancellation.cancelled() => {
            terminate_process_tree(&mut child, pid).await;
            Err(CommandFailure::Cancelled)
        }
        () = tokio::time::sleep(COMMAND_TIMEOUT) => {
            terminate_process_tree(&mut child, pid).await;
            let output = collect_output(None, stdout_task, stderr_task).await?;
            Err(CommandFailure::TimedOut(output))
        }
        status = child.wait() => {
            let status = status.map_err(CommandFailure::Io)?;
            collect_output(status.code(), stdout_task, stderr_task).await
        }
    };
    outcome
}

async fn collect_output(
    exit_code: Option<i32>,
    stdout: tokio::task::JoinHandle<std::io::Result<CapturedOutput>>,
    stderr: tokio::task::JoinHandle<std::io::Result<CapturedOutput>>,
) -> Result<CommandOutput, CommandFailure> {
    let stdout = stdout
        .await
        .map_err(|error| CommandFailure::Io(std::io::Error::other(error.to_string())))?
        .map_err(CommandFailure::Io)?;
    let stderr = stderr
        .await
        .map_err(|error| CommandFailure::Io(std::io::Error::other(error.to_string())))?
        .map_err(CommandFailure::Io)?;
    Ok(CommandOutput {
        exit_code,
        stdout,
        stderr,
    })
}

async fn read_capped(mut reader: impl AsyncRead + Unpin) -> std::io::Result<CapturedOutput> {
    let mut retained = Vec::new();
    let mut chunk = [0_u8; 8_192];
    let mut truncated = false;
    loop {
        let count = reader.read(&mut chunk).await?;
        if count == 0 {
            break;
        }
        let remaining = MAX_OUTPUT_BYTES.saturating_sub(retained.len());
        retained.extend_from_slice(&chunk[..count.min(remaining)]);
        truncated |= count > remaining;
    }
    Ok(CapturedOutput {
        bytes: retained,
        truncated,
    })
}

async fn terminate_process_tree(child: &mut tokio::process::Child, pid: Option<u32>) {
    #[cfg(unix)]
    if let Some(pid) = pid.and_then(|pid| i32::try_from(pid).ok()) {
        let _result = nix::sys::signal::killpg(
            nix::unistd::Pid::from_raw(pid),
            nix::sys::signal::Signal::SIGTERM,
        );
    }
    let _result = child.kill().await;
    let _result = child.wait().await;
}

#[cfg(unix)]
fn shell_command(command_text: &str) -> Command {
    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg(command_text);
    command
}

#[cfg(windows)]
fn shell_command(command_text: &str) -> Command {
    let mut command = Command::new("cmd");
    command.arg("/C").arg(command_text);
    command
}

fn nonempty_string<'a>(arguments: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

fn format_command_output(output: &CommandOutput) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout.bytes);
    let stderr = String::from_utf8_lossy(&output.stderr.bytes);
    let mut parts = vec![
        format!(
            "exit code: {}",
            output
                .exit_code
                .map_or_else(|| "unknown".to_owned(), |code| code.to_string())
        ),
        format!(
            "stdout:\n{stdout}{}",
            if output.stdout.truncated {
                "\n[stdout truncated]"
            } else {
                ""
            }
        ),
    ];
    if !stderr.is_empty() {
        parts.push(format!(
            "stderr:\n{stderr}{}",
            if output.stderr.truncated {
                "\n[stderr truncated]"
            } else {
                ""
            }
        ));
    }
    parts.join("\n\n")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::{Map, Value};
    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;

    use crate::policy::{
        AuthorizationDecision, FixedPermissionBroker, StandardAuthorizationPolicy,
    };
    use crate::tool::{EmptySecretsProvider, ExecutionMode, Tool, ToolExecutionContext};

    use super::{ReadFileTool, RunCommandTool, WriteFileTool};

    fn context(cwd: std::path::PathBuf) -> ToolExecutionContext {
        ToolExecutionContext {
            cwd,
            authorization: Arc::new(StandardAuthorizationPolicy::new(Arc::new(
                FixedPermissionBroker::new(AuthorizationDecision::Allow),
            ))),
            secrets: Arc::new(EmptySecretsProvider),
            execution_mode: ExecutionMode::Direct,
            cancellation: CancellationToken::new(),
        }
    }

    #[tokio::test]
    async fn reads_and_writes_files() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let context = context(directory.path().to_path_buf());
        let mut write_arguments = Map::new();
        write_arguments.insert(
            "path".to_owned(),
            Value::String("nested/example.txt".to_owned()),
        );
        write_arguments.insert("content".to_owned(), Value::String("hello".to_owned()));
        assert!(WriteFileTool.execute(write_arguments, &context).await.ok);
        let mut read_arguments = Map::new();
        read_arguments.insert(
            "path".to_owned(),
            Value::String("nested/example.txt".to_owned()),
        );
        assert_eq!(
            ReadFileTool.execute(read_arguments, &context).await.content,
            "hello"
        );
        Ok(())
    }

    #[tokio::test]
    async fn command_captures_output_and_exit_status() {
        let context = context(std::env::temp_dir());
        let mut arguments = Map::new();
        arguments.insert(
            "command".to_owned(),
            Value::String("printf output; printf problem >&2; exit 7".to_owned()),
        );
        let result = RunCommandTool.execute(arguments, &context).await;
        assert!(!result.ok);
        assert!(result.content.contains("exit code: 7"));
        assert!(result.content.contains("stdout:\noutput"));
        assert!(result.content.contains("stderr:\nproblem"));
    }
}
