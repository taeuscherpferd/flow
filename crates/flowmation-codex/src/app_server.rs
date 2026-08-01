use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use flowmation_application::{
    ChatCompletionRequest, ChatCompletionResult, ProviderError, ThinkingMode,
};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio_util::sync::CancellationToken;

use crate::discovery::{parse_account_status, parse_device_login, parse_models_page};
use crate::{
    CodexAccountStatus, CodexDeviceLogin, CodexModel, build_prompt, output_schema,
    parse_model_output,
};

const APP_SERVER_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(windows)]
const CODEX_WRAPPER_ENV: &str = "FLOWMATION_CODEX_WRAPPER_PATH";

pub(crate) struct AppServerConnection {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl AppServerConnection {
    pub(crate) async fn spawn(executable: &Path) -> Result<Self, std::io::Error> {
        let mut child = app_server_command(executable)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| {
            std::io::Error::other("Codex app-server did not expose standard input")
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            std::io::Error::other("Codex app-server did not expose standard output")
        })?;
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        })
    }

    pub(crate) async fn initialize(
        &mut self,
        cancellation: &CancellationToken,
    ) -> Result<(), ProviderError> {
        self.request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "flowmation",
                    "title": "Flowmation",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
            cancellation,
        )
        .await?;
        self.notify("initialized", json!({})).await
    }

    pub(crate) async fn start_thread(
        &mut self,
        model: &str,
        cancellation: &CancellationToken,
    ) -> Result<String, ProviderError> {
        let result = self
            .request("thread/start", thread_start_params(model), cancellation)
            .await?;
        result
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                ProviderError::InvalidResponse(
                    "Codex app-server did not return a thread id".to_owned(),
                )
            })
    }

    pub(crate) async fn account_status(
        &mut self,
        cancellation: &CancellationToken,
    ) -> Result<CodexAccountStatus, ProviderError> {
        let result = self
            .request(
                "account/read",
                json!({ "refreshToken": false }),
                cancellation,
            )
            .await?;
        parse_account_status(&result)
    }

    pub(crate) async fn list_models(
        &mut self,
        cancellation: &CancellationToken,
    ) -> Result<Vec<CodexModel>, ProviderError> {
        let mut models = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut params = json!({ "limit": 100, "includeHidden": false });
            if let Some(value) = cursor.as_ref() {
                params["cursor"] = Value::String(value.clone());
            }
            let result = self.request("model/list", params, cancellation).await?;
            let (mut page, next_cursor) = parse_models_page(&result)?;
            models.append(&mut page);
            let Some(next_cursor) = next_cursor else {
                return Ok(models);
            };
            cursor = Some(next_cursor);
        }
    }

    pub(crate) async fn start_device_login(
        &mut self,
        cancellation: &CancellationToken,
    ) -> Result<CodexDeviceLogin, ProviderError> {
        let result = self
            .request(
                "account/login/start",
                json!({ "type": "chatgptDeviceCode" }),
                cancellation,
            )
            .await?;
        parse_device_login(&result)
    }

    pub(crate) async fn wait_for_login(
        &mut self,
        login_id: &str,
        cancellation: &CancellationToken,
    ) -> Result<(), ProviderError> {
        loop {
            let message = self.read(cancellation).await?;
            if message.get("method").and_then(Value::as_str) != Some("account/login/completed")
                || message.pointer("/params/loginId").and_then(Value::as_str) != Some(login_id)
            {
                continue;
            }
            if message.pointer("/params/success").and_then(Value::as_bool) == Some(true) {
                return Ok(());
            }
            let error = message
                .pointer("/params/error")
                .and_then(Value::as_str)
                .unwrap_or("Codex login did not complete");
            return Err(ProviderError::Unavailable(error.to_owned()));
        }
    }

    pub(crate) async fn run_turn(
        &mut self,
        thread_id: &str,
        request: &ChatCompletionRequest,
        cancellation: &CancellationToken,
    ) -> Result<ChatCompletionResult, ProviderError> {
        let turn_id = self.next_request_id();
        self.send(&json!({
            "method": "turn/start",
            "id": turn_id,
            "params": turn_start_params(thread_id, request)?
        }))
        .await?;

        let mut final_message = None;
        loop {
            let message = self.read(cancellation).await?;
            if message.get("id").and_then(Value::as_u64) == Some(turn_id)
                && let Some(error) = message.get("error")
            {
                return Err(rpc_error(error));
            }
            if is_final_agent_message(&message)
                && let Some(text) = message.pointer("/params/item/text").and_then(Value::as_str)
            {
                final_message = Some(text.to_owned());
            }
            if message.get("method").and_then(Value::as_str) == Some("turn/completed") {
                return completed_turn_result(&message, final_message);
            }
        }
    }

    pub(crate) async fn stop(&mut self) {
        let _result = self.child.kill().await;
    }

    async fn request(
        &mut self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, ProviderError> {
        tokio::time::timeout(
            APP_SERVER_REQUEST_TIMEOUT,
            self.request_without_timeout(method, params, cancellation),
        )
        .await
        .map_err(|_| {
            ProviderError::Unavailable(format!(
                "Codex app-server did not respond to {method} within {} seconds",
                APP_SERVER_REQUEST_TIMEOUT.as_secs()
            ))
        })?
    }

    async fn request_without_timeout(
        &mut self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, ProviderError> {
        let id = self.next_request_id();
        self.send(&json!({ "method": method, "id": id, "params": params }))
            .await?;
        loop {
            let message = self.read(cancellation).await?;
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                return Err(rpc_error(error));
            }
            return message.get("result").cloned().ok_or_else(|| {
                ProviderError::InvalidResponse(format!(
                    "Codex app-server response {id} had no result"
                ))
            });
        }
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), ProviderError> {
        self.send(&json!({ "method": method, "params": params }))
            .await
    }

    async fn send(&mut self, value: &Value) -> Result<(), ProviderError> {
        let mut line = serde_json::to_vec(value)
            .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
        line.push(b'\n');
        self.stdin.write_all(&line).await.map_err(io_error)?;
        self.stdin.flush().await.map_err(io_error)
    }

    async fn read(&mut self, cancellation: &CancellationToken) -> Result<Value, ProviderError> {
        let mut line = String::new();
        tokio::select! {
            () = cancellation.cancelled() => Err(ProviderError::Cancelled),
            result = self.stdout.read_line(&mut line) => {
                let bytes = result.map_err(io_error)?;
                if bytes == 0 {
                    return Err(ProviderError::Unavailable(
                        "Codex app-server closed its output unexpectedly".to_owned(),
                    ));
                }
                serde_json::from_str(&line).map_err(|error| {
                    ProviderError::InvalidResponse(format!(
                        "Codex app-server returned invalid JSON: {error}"
                    ))
                })
            }
        }
    }

    fn next_request_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

fn app_server_command(executable: &Path) -> Command {
    #[cfg(windows)]
    if executable
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ps1"))
    {
        let mut command = Command::new("powershell.exe");
        command.args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            "& $env:FLOWMATION_CODEX_WRAPPER_PATH app-server",
        ]);
        command.env(CODEX_WRAPPER_ENV, executable);
        return command;
    }

    let mut command = Command::new(executable);
    command.arg("app-server");
    command
}

fn thread_start_params(model: &str) -> Value {
    json!({
        "model": model,
        "approvalPolicy": "never",
        "sandbox": "read-only",
        "serviceName": "flowmation",
        "ephemeral": true
    })
}

fn turn_start_params(
    thread_id: &str,
    request: &ChatCompletionRequest,
) -> Result<Value, ProviderError> {
    let mut params = json!({
        "threadId": thread_id,
        "input": [{
            "type": "text",
            "text": build_prompt(request)?
        }],
        "approvalPolicy": "never",
        "sandboxPolicy": { "type": "readOnly" },
        "model": request.model,
        "outputSchema": output_schema(request)
    });
    if let Some(effort) = codex_effort(request.options.thinking) {
        params["effort"] = Value::String(effort.to_owned());
    }
    Ok(params)
}

const fn codex_effort(thinking: Option<ThinkingMode>) -> Option<&'static str> {
    match thinking {
        None | Some(ThinkingMode::Default) => None,
        Some(ThinkingMode::Off) => Some("none"),
        Some(ThinkingMode::On | ThinkingMode::Medium) => Some("medium"),
        Some(ThinkingMode::Low) => Some("low"),
        Some(ThinkingMode::High) => Some("high"),
    }
}

fn is_final_agent_message(message: &Value) -> bool {
    message.get("method").and_then(Value::as_str) == Some("item/completed")
        && message.pointer("/params/item/type").and_then(Value::as_str) == Some("agentMessage")
        && message
            .pointer("/params/item/phase")
            .and_then(Value::as_str)
            .is_none_or(|phase| phase == "final_answer")
}

fn completed_turn_result(
    message: &Value,
    final_message: Option<String>,
) -> Result<ChatCompletionResult, ProviderError> {
    let status = message
        .pointer("/params/turn/status")
        .and_then(Value::as_str)
        .unwrap_or("failed");
    if status != "completed" {
        let detail = message
            .pointer("/params/turn/error/message")
            .and_then(Value::as_str)
            .unwrap_or("Codex turn did not complete");
        return Err(ProviderError::Unavailable(detail.to_owned()));
    }
    let text = final_message.ok_or_else(|| {
        ProviderError::InvalidResponse(
            "Codex completed without a final assistant message".to_owned(),
        )
    })?;
    parse_model_output(&text)
}

fn rpc_error(error: &Value) -> ProviderError {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("Codex app-server request failed");
    ProviderError::Unavailable(message.to_owned())
}

fn io_error(error: std::io::Error) -> ProviderError {
    ProviderError::Unavailable(format!("Codex app-server I/O failed: {error}"))
}

#[cfg(test)]
mod launch_tests {
    use std::ffi::OsStr;
    #[cfg(windows)]
    use std::io::Write;
    use std::path::Path;
    #[cfg(windows)]
    use std::time::Duration;

    #[cfg(windows)]
    use tempfile::Builder;
    #[cfg(windows)]
    use tokio_util::sync::CancellationToken;

    #[cfg(windows)]
    use super::AppServerConnection;
    use super::app_server_command;

    #[test]
    fn native_codex_binary_is_launched_directly() {
        let command = app_server_command(Path::new("codex"));
        assert_eq!(command.as_std().get_program(), OsStr::new("codex"));
        assert_eq!(
            command.as_std().get_args().collect::<Vec<_>>(),
            vec![OsStr::new("app-server")]
        );
    }

    #[cfg(windows)]
    #[test]
    fn powershell_codex_wrapper_is_launched_through_powershell() {
        let executable = Path::new(r"C:\Users\example\AppData\Local\pnpm\codex.ps1");
        let command = app_server_command(executable);
        assert_eq!(command.as_std().get_program(), OsStr::new("powershell.exe"));
        assert_eq!(
            command.as_std().get_args().collect::<Vec<_>>(),
            vec![
                OsStr::new("-NoLogo"),
                OsStr::new("-NoProfile"),
                OsStr::new("-NonInteractive"),
                OsStr::new("-ExecutionPolicy"),
                OsStr::new("Bypass"),
                OsStr::new("-Command"),
                OsStr::new("& $env:FLOWMATION_CODEX_WRAPPER_PATH app-server"),
            ]
        );
        assert!(command.as_std().get_envs().any(|(name, value)| {
            name == OsStr::new("FLOWMATION_CODEX_WRAPPER_PATH")
                && value == Some(executable.as_os_str())
        }));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn powershell_wrapper_receives_input_without_waiting_for_end_of_file()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut wrapper = Builder::new().suffix(".ps1").tempfile()?;
        wrapper.write_all(
            br#"if ($MyInvocation.ExpectingInput) {
    $input | Out-Null
}
[Console]::In.ReadLine() | Out-Null
[Console]::Out.WriteLine('{"id":1,"result":{}}')
[Console]::Out.Flush()
[Console]::In.ReadLine() | Out-Null
"#,
        )?;
        wrapper.flush()?;
        let wrapper_path = wrapper.into_temp_path();

        let mut connection = AppServerConnection::spawn(wrapper_path.as_ref()).await?;
        tokio::time::timeout(
            Duration::from_secs(5),
            connection.initialize(&CancellationToken::new()),
        )
        .await??;
        connection.stop().await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use flowmation_application::{
        ChatCompletionOptions, ChatCompletionRequest, ChatMessage, ChatRole, ThinkingMode,
    };

    use super::{thread_start_params, turn_start_params};

    fn request(thinking: Option<ThinkingMode>) -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "gpt-5.6".to_owned(),
            messages: vec![ChatMessage::new(ChatRole::User, "hello")],
            tools: Vec::new(),
            options: ChatCompletionOptions {
                num_ctx: Some(1_050_000),
                thinking,
            },
        }
    }

    #[test]
    fn starts_disposable_threads() {
        let params = thread_start_params("gpt-5.6");
        assert_eq!(params["ephemeral"], true);
        assert_eq!(params["sandbox"], "read-only");
    }

    #[test]
    fn maps_provider_thinking_modes_to_codex_effort() -> Result<(), Box<dyn std::error::Error>> {
        let cases = [
            (None, None),
            (Some(ThinkingMode::Default), None),
            (Some(ThinkingMode::Off), Some("none")),
            (Some(ThinkingMode::On), Some("medium")),
            (Some(ThinkingMode::Low), Some("low")),
            (Some(ThinkingMode::Medium), Some("medium")),
            (Some(ThinkingMode::High), Some("high")),
        ];

        for (thinking, expected) in cases {
            let params = turn_start_params("thread-1", &request(thinking))?;
            assert_eq!(params["sandboxPolicy"]["type"], "readOnly");
            assert_eq!(
                params.get("effort").and_then(|value| value.as_str()),
                expected
            );
        }
        Ok(())
    }
}
