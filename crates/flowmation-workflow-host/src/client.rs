use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, oneshot};
use tokio::task::JoinHandle;

use crate::protocol::{
    CancelWorkflowParams, CancelWorkflowResult, HandshakeParams, HandshakeResult, HostNotification,
    InspectWorkflowParams, InspectWorkflowResult, InvokeCallbackParams, JSON_RPC_VERSION,
    PROTOCOL_VERSION, RpcError, RpcId, RunWorkflowParams, RunWorkflowResult,
    WorkflowCallbackRequest,
};

#[derive(Debug, Error)]
pub enum WorkflowHostError {
    #[error("failed to launch workflow host: {0}")]
    Launch(#[source] std::io::Error),
    #[error("workflow host did not expose its {0} pipe")]
    MissingPipe(&'static str),
    #[error("workflow host transport failed: {0}")]
    Transport(String),
    #[error("workflow host rejected the request ({0}): {1}")]
    Remote(i32, String),
    #[error(
        "workflow host protocol version mismatch: Rust requires {expected}, host reported {actual}"
    )]
    ProtocolVersionMismatch { expected: u32, actual: u32 },
    #[error("workflow host sent an invalid message: {0}")]
    InvalidMessage(String),
    #[error("workflow host {operation} timed out after {timeout:?}")]
    Timeout {
        operation: &'static str,
        timeout: Duration,
    },
}

#[derive(Clone, Debug)]
enum PendingFailure {
    Remote(RpcError),
    Closed(String),
}

type PendingResult = Result<Value, PendingFailure>;

struct RpcPeer {
    writer: Mutex<ChildStdin>,
    pending: Mutex<HashMap<RpcId, oneshot::Sender<PendingResult>>>,
    next_id: AtomicU64,
    closed: AtomicBool,
}

impl RpcPeer {
    async fn request<P: Serialize + Sync>(
        &self,
        method: &str,
        params: &P,
    ) -> Result<Value, WorkflowHostError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(WorkflowHostError::Transport(
                "the workflow host connection is closed".into(),
            ));
        }

        let id = RpcId::Number(self.next_id.fetch_add(1, Ordering::Relaxed));
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), sender);
        let message = json!({
            "jsonrpc": JSON_RPC_VERSION,
            "id": id,
            "method": method,
            "params": params,
        });
        if let Err(error) = self.write(&message).await {
            self.pending.lock().await.remove(&id);
            return Err(error);
        }

        match receiver.await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(PendingFailure::Remote(error))) => {
                Err(WorkflowHostError::Remote(error.code, error.message))
            }
            Ok(Err(PendingFailure::Closed(message))) => Err(WorkflowHostError::Transport(message)),
            Err(_) => Err(WorkflowHostError::Transport(
                "the workflow host response channel closed".into(),
            )),
        }
    }

    async fn respond(&self, id: &RpcId, result: Result<Value, RpcError>) {
        let message = match result {
            Ok(value) => json!({
                "jsonrpc": JSON_RPC_VERSION,
                "id": id,
                "result": value,
            }),
            Err(error) => json!({
                "jsonrpc": JSON_RPC_VERSION,
                "id": id,
                "error": error,
            }),
        };
        let _result = self.write(&message).await;
    }

    async fn write(&self, message: &Value) -> Result<(), WorkflowHostError> {
        let mut encoded = serde_json::to_vec(message)
            .map_err(|error| WorkflowHostError::InvalidMessage(error.to_string()))?;
        encoded.push(b'\n');
        let mut writer = self.writer.lock().await;
        writer
            .write_all(&encoded)
            .await
            .map_err(|error| WorkflowHostError::Transport(error.to_string()))?;
        writer
            .flush()
            .await
            .map_err(|error| WorkflowHostError::Transport(error.to_string()))
    }

    async fn close(&self, reason: impl Into<String>) {
        self.closed.store(true, Ordering::Release);
        let reason = reason.into();
        let pending = std::mem::take(&mut *self.pending.lock().await);
        for sender in pending.into_values() {
            let _result = sender.send(Err(PendingFailure::Closed(reason.clone())));
        }
    }
}

#[derive(Clone)]
pub struct CallbackInvoker {
    peer: Arc<RpcPeer>,
}

impl CallbackInvoker {
    /// Invokes a JavaScript callback registered by the active workflow.
    ///
    /// # Errors
    ///
    /// Returns an error when the host exits, rejects the callback, or sends an invalid response.
    pub async fn invoke(
        &self,
        callback_id: impl Into<String>,
        arguments: Vec<Value>,
    ) -> Result<Value, WorkflowHostError> {
        self.peer
            .request(
                "callback.invoke",
                &InvokeCallbackParams {
                    callback_id: callback_id.into(),
                    arguments,
                },
            )
            .await
    }
}

#[async_trait]
pub trait WorkflowCallbackHandler: Send + Sync + 'static {
    async fn handle(
        &self,
        request: WorkflowCallbackRequest,
        invoker: CallbackInvoker,
    ) -> Result<Value, RpcError>;

    async fn notification(&self, _notification: HostNotification) {}
}

struct RejectCallbacks;

#[async_trait]
impl WorkflowCallbackHandler for RejectCallbacks {
    async fn handle(
        &self,
        request: WorkflowCallbackRequest,
        _invoker: CallbackInvoker,
    ) -> Result<Value, RpcError> {
        let method = match request {
            WorkflowCallbackRequest::Unknown { method, .. } => method,
            other => format!("{other:?}"),
        };
        Err(RpcError::new(
            RpcError::METHOD_NOT_FOUND,
            format!("no Rust callback handler is configured for {method}"),
        ))
    }
}

#[derive(Clone, Debug)]
pub struct WorkflowHostConfig {
    pub executable: PathBuf,
    pub host_entry: PathBuf,
    pub arguments: Vec<OsString>,
    pub working_directory: Option<PathBuf>,
    pub environment: Vec<(OsString, OsString)>,
    pub protocol_version: u32,
    pub client_name: String,
    pub client_version: String,
    pub handshake_timeout: Duration,
    pub shutdown_timeout: Duration,
}

impl WorkflowHostConfig {
    #[must_use]
    pub fn new(host_entry: impl Into<PathBuf>) -> Self {
        Self {
            executable: PathBuf::from("node"),
            host_entry: host_entry.into(),
            arguments: Vec::new(),
            working_directory: None,
            environment: Vec::new(),
            protocol_version: PROTOCOL_VERSION,
            client_name: "flowmation-rust".into(),
            client_version: env!("CARGO_PKG_VERSION").into(),
            handshake_timeout: Duration::from_secs(5),
            shutdown_timeout: Duration::from_secs(2),
        }
    }
}

pub struct WorkflowHost {
    peer: Arc<RpcPeer>,
    child: Mutex<Child>,
    reader_task: Mutex<Option<JoinHandle<()>>>,
    handshake: HandshakeResult,
    shutdown_timeout: Duration,
}

impl WorkflowHost {
    /// Launches the configured host and completes the protocol handshake.
    ///
    /// # Errors
    ///
    /// Returns an error when the process cannot launch, handshake, or negotiate the configured
    /// protocol version.
    pub async fn spawn(
        config: WorkflowHostConfig,
        callback_handler: Arc<dyn WorkflowCallbackHandler>,
    ) -> Result<Self, WorkflowHostError> {
        let mut command = Command::new(&config.executable);
        command
            .args(&config.arguments)
            .arg(&config.host_entry)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        if let Some(working_directory) = &config.working_directory {
            command.current_dir(working_directory);
        }
        command.envs(config.environment.iter().cloned());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.as_std_mut().process_group(0);
        }

        let mut child = command.spawn().map_err(WorkflowHostError::Launch)?;
        let stdin = child
            .stdin
            .take()
            .ok_or(WorkflowHostError::MissingPipe("stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or(WorkflowHostError::MissingPipe("stdout"))?;
        let peer = Arc::new(RpcPeer {
            writer: Mutex::new(stdin),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            closed: AtomicBool::new(false),
        });
        let reader_task = tokio::spawn(read_messages(
            BufReader::new(stdout),
            Arc::clone(&peer),
            callback_handler,
        ));
        let handshake_params = HandshakeParams {
            protocol_version: config.protocol_version,
            client_name: config.client_name,
            client_version: config.client_version,
        };
        let handshake_value = match tokio::time::timeout(
            config.handshake_timeout,
            peer.request("host.handshake", &handshake_params),
        )
        .await
        {
            Ok(Ok(value)) => value,
            Ok(Err(WorkflowHostError::Remote(RpcError::VERSION_MISMATCH, _))) => {
                let _result = child.start_kill();
                return Err(WorkflowHostError::ProtocolVersionMismatch {
                    expected: config.protocol_version,
                    actual: PROTOCOL_VERSION,
                });
            }
            Ok(Err(error)) => {
                let _result = child.start_kill();
                return Err(error);
            }
            Err(_) => {
                let _result = child.start_kill();
                return Err(WorkflowHostError::Timeout {
                    operation: "handshake",
                    timeout: config.handshake_timeout,
                });
            }
        };
        let handshake: HandshakeResult = serde_json::from_value(handshake_value)
            .map_err(|error| WorkflowHostError::InvalidMessage(error.to_string()))?;
        if handshake.protocol_version != config.protocol_version {
            let _result = child.start_kill();
            return Err(WorkflowHostError::ProtocolVersionMismatch {
                expected: config.protocol_version,
                actual: handshake.protocol_version,
            });
        }

        Ok(Self {
            peer,
            child: Mutex::new(child),
            reader_task: Mutex::new(Some(reader_task)),
            handshake,
            shutdown_timeout: config.shutdown_timeout,
        })
    }

    /// Launches a host that rejects every workflow SDK callback.
    ///
    /// # Errors
    ///
    /// Returns the same launch and handshake errors as [`Self::spawn`].
    pub async fn spawn_without_callbacks(
        config: WorkflowHostConfig,
    ) -> Result<Self, WorkflowHostError> {
        Self::spawn(config, Arc::new(RejectCallbacks)).await
    }

    #[must_use]
    pub const fn handshake(&self) -> &HandshakeResult {
        &self.handshake
    }

    /// Loads and validates workflow metadata in the JavaScript host.
    ///
    /// # Errors
    ///
    /// Returns an error when the host cannot load the module or the transport fails.
    pub async fn inspect(
        &self,
        params: InspectWorkflowParams,
    ) -> Result<InspectWorkflowResult, WorkflowHostError> {
        self.request_typed("workflow.inspect", &params).await
    }

    /// Executes a workflow while serving its reverse SDK requests.
    ///
    /// # Errors
    ///
    /// Returns an error when execution fails, is cancelled, or the transport closes.
    pub async fn run(
        &self,
        params: RunWorkflowParams,
    ) -> Result<RunWorkflowResult, WorkflowHostError> {
        self.request_typed("workflow.run", &params).await
    }

    /// Requests cancellation of an active JavaScript workflow run.
    ///
    /// # Errors
    ///
    /// Returns an error when the cancellation request cannot be delivered.
    pub async fn cancel(
        &self,
        run_id: impl Into<String>,
        reason: Option<String>,
    ) -> Result<CancelWorkflowResult, WorkflowHostError> {
        self.request_typed(
            "workflow.cancel",
            &CancelWorkflowParams {
                run_id: run_id.into(),
                reason,
            },
        )
        .await
    }

    /// Invokes a callback previously registered by JavaScript.
    ///
    /// # Errors
    ///
    /// Returns an error when the callback expired, rejects, or the transport closes.
    pub async fn invoke_callback(
        &self,
        callback_id: impl Into<String>,
        arguments: Vec<Value>,
    ) -> Result<Value, WorkflowHostError> {
        CallbackInvoker {
            peer: Arc::clone(&self.peer),
        }
        .invoke(callback_id, arguments)
        .await
    }

    /// Sends an extension request not covered by the typed protocol helpers.
    ///
    /// # Errors
    ///
    /// Returns an error when the host rejects the request or the transport closes.
    pub async fn request_value<P: Serialize + Sync>(
        &self,
        method: &str,
        params: &P,
    ) -> Result<Value, WorkflowHostError> {
        self.peer.request(method, params).await
    }

    /// Asks the host to shut down, then terminates its process tree if it does not exit in time.
    ///
    /// # Errors
    ///
    /// Returns an error when waiting for the process fails.
    pub async fn shutdown(&self) -> Result<(), WorkflowHostError> {
        if !self.peer.closed.load(Ordering::Acquire) {
            let request = tokio::time::timeout(
                self.shutdown_timeout,
                self.peer
                    .request("host.shutdown", &Map::<String, Value>::new()),
            )
            .await;
            if request.is_err() {
                self.terminate(false).await;
            }
        }

        let wait_result = {
            let mut child = self.child.lock().await;
            tokio::time::timeout(self.shutdown_timeout, child.wait()).await
        };
        match wait_result {
            Ok(Ok(_status)) => {}
            Ok(Err(error)) => return Err(WorkflowHostError::Transport(error.to_string())),
            Err(_) => {
                self.terminate(true).await;
                let mut child = self.child.lock().await;
                child
                    .wait()
                    .await
                    .map_err(|error| WorkflowHostError::Transport(error.to_string()))?;
                drop(child);
            }
        }
        self.peer.close("the workflow host shut down").await;
        let reader_task = self.reader_task.lock().await.take();
        if let Some(reader_task) = reader_task {
            let _result = reader_task.await;
        }
        Ok(())
    }

    async fn request_typed<P, R>(&self, method: &str, params: &P) -> Result<R, WorkflowHostError>
    where
        P: Serialize + Sync,
        R: DeserializeOwned,
    {
        let result = self.peer.request(method, params).await?;
        serde_json::from_value(result)
            .map_err(|error| WorkflowHostError::InvalidMessage(error.to_string()))
    }

    async fn terminate(&self, force: bool) {
        let process_id = self.child.lock().await.id();
        if let Some(process_id) = process_id {
            terminate_process_tree(process_id, force).await;
        }
        if force {
            let _result = self.child.lock().await.start_kill();
        }
    }
}

impl Drop for WorkflowHost {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.try_lock() {
            let _result = child.start_kill();
        }
    }
}

async fn read_messages(
    mut reader: BufReader<tokio::process::ChildStdout>,
    peer: Arc<RpcPeer>,
    callback_handler: Arc<dyn WorkflowCallbackHandler>,
) {
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => {
                peer.close("the workflow host closed stdout").await;
                return;
            }
            Ok(_) => {
                let parsed = serde_json::from_str::<Value>(line.trim_end());
                match parsed {
                    Ok(message) => {
                        process_message(message, Arc::clone(&peer), Arc::clone(&callback_handler))
                            .await;
                    }
                    Err(error) => {
                        peer.close(format!("invalid JSON from workflow host: {error}"))
                            .await;
                        return;
                    }
                }
            }
            Err(error) => {
                peer.close(format!("failed reading workflow host stdout: {error}"))
                    .await;
                return;
            }
        }
    }
}

async fn process_message(
    message: Value,
    peer: Arc<RpcPeer>,
    callback_handler: Arc<dyn WorkflowCallbackHandler>,
) {
    let Some(object) = message.as_object() else {
        return;
    };
    if object.get("jsonrpc").and_then(Value::as_str) != Some(JSON_RPC_VERSION) {
        return;
    }
    if let Some(method) = object.get("method").and_then(Value::as_str) {
        let params = object.get("params").cloned().unwrap_or(Value::Null);
        if let Some(id_value) = object.get("id") {
            let id = serde_json::from_value::<RpcId>(id_value.clone());
            if let Ok(id) = id {
                let method = method.to_owned();
                tokio::spawn(async move {
                    let result = match WorkflowCallbackRequest::from_method(method, params) {
                        Ok(request) => {
                            callback_handler
                                .handle(
                                    request,
                                    CallbackInvoker {
                                        peer: Arc::clone(&peer),
                                    },
                                )
                                .await
                        }
                        Err(error) => {
                            Err(RpcError::new(RpcError::INVALID_PARAMS, error.to_string()))
                        }
                    };
                    peer.respond(&id, result).await;
                });
            }
        } else if let Ok(notification) = HostNotification::from_method(method.to_owned(), params) {
            tokio::spawn(async move {
                callback_handler.notification(notification).await;
            });
        }
        return;
    }

    let Some(id_value) = object.get("id") else {
        return;
    };
    let Ok(id) = serde_json::from_value::<RpcId>(id_value.clone()) else {
        return;
    };
    let result = parse_response(object);
    let sender = peer.pending.lock().await.remove(&id);
    if let Some(sender) = sender {
        let _result = sender.send(result);
    }
}

fn parse_response(object: &Map<String, Value>) -> PendingResult {
    if let Some(result) = object.get("result") {
        return Ok(result.clone());
    }
    if let Some(error) = object.get("error") {
        return serde_json::from_value::<RpcError>(error.clone())
            .map(PendingFailure::Remote)
            .map_or_else(
                |decode_error| {
                    Err(PendingFailure::Closed(format!(
                        "invalid JSON-RPC error: {decode_error}"
                    )))
                },
                Err,
            );
    }
    Err(PendingFailure::Closed(
        "JSON-RPC response has neither result nor error".into(),
    ))
}

#[cfg(unix)]
async fn terminate_process_tree(process_id: u32, force: bool) {
    let signal = if force { "-KILL" } else { "-TERM" };
    let _result = Command::new("kill")
        .arg(signal)
        .arg("--")
        .arg(format!("-{process_id}"))
        .status()
        .await;
}

#[cfg(windows)]
async fn terminate_process_tree(process_id: u32, _force: bool) {
    let _result = Command::new("taskkill.exe")
        .args(["/pid", &process_id.to_string(), "/t", "/f"])
        .status()
        .await;
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::protocol::{LogCallback, WorkflowCallbackRequest};

    const MOCK_HOST: &str = r#"
import { createInterface } from "node:readline";

const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
const write = (message) => process.stdout.write(`${JSON.stringify(message)}\n`);
write({
  jsonrpc: "2.0",
  method: "host.ready",
  params: { protocolVersion: 1, runtime: `node/${process.versions.node}` },
});

let callbackRequest;
for await (const line of lines) {
  const message = JSON.parse(line);
  if (message.method === "host.handshake") {
    if (message.params.protocolVersion !== 1) {
      write({
        jsonrpc: "2.0",
        id: message.id,
        error: {
          code: -32001,
          message: "version mismatch",
          data: { supportedVersion: 1 },
        },
      });
    } else {
      write({
        jsonrpc: "2.0",
        id: message.id,
        result: {
          protocolVersion: 1,
          hostName: "mock-host",
          hostVersion: "1.0.0",
          runtime: `node/${process.versions.node}`,
          capabilities: ["bidirectional-callbacks"],
        },
      });
    }
  } else if (message.method === "test.callback") {
    callbackRequest = message.id;
    write({
      jsonrpc: "2.0",
      id: "callback-from-js",
      method: "sdk.log",
      params: { runId: "run-1", message: "nested" },
    });
  } else if (message.id === "callback-from-js") {
    write({
      jsonrpc: "2.0",
      id: callbackRequest,
      result: { callbackResult: message.result },
    });
  } else if (message.method === "host.shutdown") {
    write({ jsonrpc: "2.0", id: message.id, result: null });
    lines.close();
  }
}
"#;

    struct LogHandler;

    #[async_trait]
    impl WorkflowCallbackHandler for LogHandler {
        async fn handle(
            &self,
            request: WorkflowCallbackRequest,
            _invoker: CallbackInvoker,
        ) -> Result<Value, RpcError> {
            match request {
                WorkflowCallbackRequest::Log(LogCallback { message, .. }) => {
                    Ok(Value::String(format!("handled:{message}")))
                }
                _ => Err(RpcError::new(
                    RpcError::METHOD_NOT_FOUND,
                    "unexpected callback",
                )),
            }
        }
    }

    fn mock_host() -> Result<(TempDir, PathBuf), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let script = directory.path().join("host.mjs");
        fs::write(&script, MOCK_HOST)?;
        Ok((directory, script))
    }

    #[tokio::test]
    async fn launches_handshakes_and_routes_bidirectional_callbacks() -> Result<(), Box<dyn Error>>
    {
        let (_directory, script) = mock_host()?;
        let host =
            WorkflowHost::spawn(WorkflowHostConfig::new(script), Arc::new(LogHandler)).await?;

        assert_eq!(host.handshake().protocol_version, PROTOCOL_VERSION);
        assert_eq!(host.handshake().host_name, "mock-host");
        let result = host
            .request_value("test.callback", &Map::<String, Value>::new())
            .await?;
        assert_eq!(result, json!({ "callbackResult": "handled:nested" }));
        host.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn rejects_protocol_version_mismatch() -> Result<(), Box<dyn Error>> {
        let (_directory, script) = mock_host()?;
        let mut config = WorkflowHostConfig::new(script);
        config.protocol_version = 99;
        let result = WorkflowHost::spawn_without_callbacks(config).await;

        assert!(matches!(
            result,
            Err(WorkflowHostError::ProtocolVersionMismatch {
                expected: 99,
                actual: PROTOCOL_VERSION
            })
        ));
        Ok(())
    }
}
