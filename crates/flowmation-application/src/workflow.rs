use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use flowmation_domain::agent::{PackageSource, is_kebab_case_name};
use flowmation_domain::fingerprint::fingerprint_directory;
use flowmation_domain::schema::{WorkflowSchema, validate_schema};
use flowmation_workflow_host::protocol::{
    AgentRunCallback, AgentRunResult, AgentSession, ElevationCallback, ExecCallback, HumanCallback,
    InspectWorkflowParams, MapCallback, RpcError, RunWorkflowParams, WorkflowCallbackRequest,
    WorkflowMetadata, WorkflowPresentation as HostPresentation,
};
use flowmation_workflow_host::{
    CallbackInvoker, WorkflowCallbackHandler, WorkflowHost, WorkflowHostError,
};
use futures::stream::{self, StreamExt, TryStreamExt};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const HUMAN_SUSPENDED_ERROR: i32 = -32_010;
const DEFAULT_MAX_OUTPUT_BYTES: usize = 5 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct WorkflowRecord {
    pub metadata: WorkflowMetadata,
    pub directory: PathBuf,
    pub entry_path: PathBuf,
    pub fingerprint: String,
    pub source: PackageSource,
    pub agent_name: Option<String>,
    pub resource_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct WorkflowRegistryRoot {
    pub directory: PathBuf,
    pub source: PackageSource,
}

#[async_trait]
pub trait WorkflowInspector: Send + Sync {
    async fn inspect(&self, entry_path: &Path) -> Result<WorkflowMetadata, String>;
}

#[async_trait]
impl WorkflowInspector for WorkflowHost {
    async fn inspect(&self, entry_path: &Path) -> Result<WorkflowMetadata, String> {
        WorkflowHost::inspect(
            self,
            InspectWorkflowParams {
                entry_path: entry_path.display().to_string(),
            },
        )
        .await
        .map(|result| result.metadata)
        .map_err(|error| error.to_string())
    }
}

pub struct WorkflowRegistry {
    roots: Vec<WorkflowRegistryRoot>,
    inspector: Arc<dyn WorkflowInspector>,
    agent_name: Option<String>,
    names: Option<Vec<String>>,
    workflows: BTreeMap<String, WorkflowRecord>,
    warnings: Vec<String>,
}

impl WorkflowRegistry {
    #[must_use]
    pub fn new(
        roots: Vec<WorkflowRegistryRoot>,
        inspector: Arc<dyn WorkflowInspector>,
        agent_name: Option<String>,
        names: Option<Vec<String>>,
    ) -> Self {
        Self {
            roots,
            inspector,
            agent_name,
            names,
            workflows: BTreeMap::new(),
            warnings: Vec::new(),
        }
    }

    pub async fn load(&mut self) -> Result<(), std::io::Error> {
        self.workflows.clear();
        self.warnings.clear();
        for root in self.roots.clone() {
            self.scan(&root).await?;
        }
        Ok(())
    }

    #[must_use]
    pub fn list(&self) -> Vec<&WorkflowRecord> {
        self.workflows.values().collect()
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&WorkflowRecord> {
        self.workflows.get(name)
    }

    #[must_use]
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn parse_input(&self, record: &WorkflowRecord, raw: &str) -> Result<Value, String> {
        let Some(schema_value) = &record.metadata.input_schema else {
            return Ok(Value::String(raw.to_owned()));
        };
        let schema: WorkflowSchema =
            serde_json::from_value(schema_value.clone()).map_err(|error| error.to_string())?;
        let input = if matches!(schema, WorkflowSchema::String { .. }) {
            Value::String(raw.to_owned())
        } else {
            serde_json::from_str(if raw.is_empty() { "{}" } else { raw }).map_err(|error| {
                format!(
                    "Workflow \"{}\" expects JSON object input: {error}",
                    record.metadata.name
                )
            })?
        };
        validate_input(&schema, &input)?;
        Ok(input)
    }

    pub fn validate_input(&self, record: &WorkflowRecord, input: &Value) -> Result<(), String> {
        let Some(schema_value) = &record.metadata.input_schema else {
            return if input.is_string() {
                Ok(())
            } else {
                Err(format!(
                    "Workflow \"{}\" expects string input.",
                    record.metadata.name
                ))
            };
        };
        let schema: WorkflowSchema =
            serde_json::from_value(schema_value.clone()).map_err(|error| error.to_string())?;
        validate_input(&schema, input)
    }

    async fn scan(&mut self, root: &WorkflowRegistryRoot) -> Result<(), std::io::Error> {
        let mut entries = match tokio::fs::read_dir(&root.directory).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        while let Some(entry) = entries.next_entry().await? {
            if !entry.file_type().await?.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if self
                .names
                .as_ref()
                .is_some_and(|names| !names.iter().any(|candidate| candidate == &name))
            {
                continue;
            }
            self.load_directory(&entry.path(), &name, root.source).await;
        }
        Ok(())
    }

    async fn load_directory(&mut self, directory: &Path, name: &str, source: PackageSource) {
        if !is_kebab_case_name(name) {
            self.warnings.push(format!(
                "Skipping workflow directory \"{name}\" — names must use lowercase kebab-case."
            ));
            return;
        }
        let js = directory.join("WORKFLOW.js");
        let ts = directory.join("WORKFLOW.ts");
        let js_exists = tokio::fs::try_exists(&js).await.unwrap_or(false);
        let ts_exists = tokio::fs::try_exists(&ts).await.unwrap_or(false);
        let entry_path = match (js_exists, ts_exists) {
            (false, false) => return,
            (true, true) => {
                self.warnings.push(format!(
                    "Skipping workflow \"{name}\" — both WORKFLOW.ts and WORKFLOW.js exist."
                ));
                return;
            }
            (true, false) => js,
            (false, true) => ts,
        };
        let fingerprint = match fingerprint_directory(directory) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                self.warnings.push(format!(
                    "Skipping workflow \"{name}\" — failed to fingerprint {}: {error}.",
                    directory.display()
                ));
                return;
            }
        };
        let metadata = match self.inspector.inspect(&entry_path).await {
            Ok(metadata) => metadata,
            Err(error) => {
                self.warnings.push(format!(
                    "Skipping workflow \"{name}\" — failed to load {}: {error}.",
                    entry_path.display()
                ));
                return;
            }
        };
        if let Err(error) = validate_metadata(&metadata, name) {
            self.warnings
                .push(format!("Skipping workflow \"{name}\" — {error}."));
            return;
        }
        let agent_name = self.agent_name.clone();
        let resource_id = agent_name
            .as_ref()
            .map(|agent_name| format!("{agent_name}/{name}"));
        self.workflows.insert(
            name.to_owned(),
            WorkflowRecord {
                metadata,
                directory: directory.to_path_buf(),
                entry_path,
                fingerprint,
                source,
                agent_name,
                resource_id,
            },
        );
    }
}

fn validate_metadata(metadata: &WorkflowMetadata, expected_name: &str) -> Result<(), String> {
    if metadata.name != expected_name {
        return Err(format!(
            "the exported name \"{}\" does not match directory \"{expected_name}\"",
            metadata.name
        ));
    }
    if !is_kebab_case_name(&metadata.name) {
        return Err("workflow names must use lowercase kebab-case".to_owned());
    }
    if metadata.description.trim().is_empty() {
        return Err("the definition is missing a non-empty \"description\"".to_owned());
    }
    if let Some(schema) = &metadata.input_schema {
        let schema: WorkflowSchema =
            serde_json::from_value(schema.clone()).map_err(|error| error.to_string())?;
        if !schema.is_valid_root() {
            return Err("\"input.schema\" must be a string or object schema".to_owned());
        }
    }
    Ok(())
}

fn validate_input(schema: &WorkflowSchema, input: &Value) -> Result<(), String> {
    let validation = validate_schema(schema, input);
    if validation.valid {
        Ok(())
    } else {
        Err(validation.errors.join("\n"))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurableStepKind {
    Checkpoint,
    Effect,
    Human,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DurableStep {
    pub kind: DurableStepKind,
    pub input: Option<Value>,
    pub output: Option<Value>,
    pub completed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurableRunStatus {
    Queued,
    Running,
    Waiting,
    Interrupted,
    Completed,
    Failed,
    Cancelled,
    VersionMismatch,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DurableRun {
    pub workflow_name: String,
    pub project_dir: PathBuf,
    pub source_entry_path: PathBuf,
    pub source_fingerprint: String,
    pub status: DurableRunStatus,
    pub input: Value,
    pub output: Option<Value>,
}

#[async_trait]
pub trait WorkflowDurability: Send + Sync {
    async fn create_run(
        &self,
        run_id: &str,
        record: &WorkflowRecord,
        project_dir: &Path,
        input: &Value,
    ) -> Result<(), String>;
    async fn load_run(&self, run_id: &str) -> Result<Option<DurableRun>, String>;
    async fn mark_running(&self, run_id: &str) -> Result<(), String>;
    async fn complete_run(
        &self,
        run_id: &str,
        output: &Value,
        presentation: HostPresentation,
    ) -> Result<(), String>;
    async fn mark_run(&self, run_id: &str, status: &str, error: Option<&str>)
    -> Result<(), String>;
    async fn step(&self, run_id: &str, key: &str) -> Result<Option<DurableStep>, String>;
    async fn start_step(
        &self,
        run_id: &str,
        key: &str,
        kind: DurableStepKind,
        input: Option<&Value>,
    ) -> Result<(), String>;
    async fn complete_step(&self, run_id: &str, key: &str, output: &Value) -> Result<(), String>;
}

#[async_trait]
pub trait HumanRequestBroker: Send + Sync {
    async fn request(&self, run_id: &str, prompt: &HumanCallback) -> Result<Option<Value>, String>;
}

#[async_trait]
pub trait WorkflowAgentRuntime: Send + Sync {
    async fn create(&self, run_id: &str, model: Option<&str>) -> Result<AgentSession, String>;
    async fn fork(
        &self,
        run_id: &str,
        session_id: &str,
        model: Option<&str>,
    ) -> Result<AgentSession, String>;
    async fn retarget(
        &self,
        run_id: &str,
        session_id: &str,
        model: &str,
    ) -> Result<AgentSession, String>;
    async fn run(&self, request: &AgentRunCallback) -> Result<AgentRunResult, String>;
}

pub trait WorkflowLogSink: Send + Sync {
    fn log(&self, run_id: &str, message: &str, data: Option<&Value>);
}

#[derive(Clone)]
pub struct WorkflowCallbackServices {
    durability: Arc<dyn WorkflowDurability>,
    human: Arc<dyn HumanRequestBroker>,
    agents: Arc<dyn WorkflowAgentRuntime>,
    logs: Arc<dyn WorkflowLogSink>,
    active_runs: Arc<Mutex<HashMap<String, ActiveWorkflowRun>>>,
    human_occurrences: Arc<Mutex<HashMap<(String, String), usize>>>,
    human_gate: Arc<AsyncMutex<()>>,
}

#[derive(Clone)]
struct ActiveWorkflowRun {
    cancellation: CancellationToken,
    project_dir: PathBuf,
}

impl WorkflowCallbackServices {
    #[must_use]
    pub fn new(
        durability: Arc<dyn WorkflowDurability>,
        human: Arc<dyn HumanRequestBroker>,
        agents: Arc<dyn WorkflowAgentRuntime>,
        logs: Arc<dyn WorkflowLogSink>,
    ) -> Self {
        Self {
            durability,
            human,
            agents,
            logs,
            active_runs: Arc::new(Mutex::new(HashMap::new())),
            human_occurrences: Arc::new(Mutex::new(HashMap::new())),
            human_gate: Arc::new(AsyncMutex::new(())),
        }
    }

    pub fn register_run(&self, run_id: &str, project_dir: &Path, cancellation: CancellationToken) {
        if let Ok(mut active_runs) = self.active_runs.lock() {
            active_runs.insert(
                run_id.to_owned(),
                ActiveWorkflowRun {
                    cancellation,
                    project_dir: project_dir.to_path_buf(),
                },
            );
        }
    }

    pub fn unregister_run(&self, run_id: &str) {
        if let Ok(mut active_runs) = self.active_runs.lock() {
            active_runs.remove(run_id);
        }
        if let Ok(mut occurrences) = self.human_occurrences.lock() {
            occurrences.retain(|(occurrence_run_id, _), _| occurrence_run_id != run_id);
        }
    }

    fn active_run(&self, run_id: &str) -> ActiveWorkflowRun {
        self.active_runs
            .lock()
            .ok()
            .and_then(|active_runs| active_runs.get(run_id).cloned())
            .unwrap_or_else(|| ActiveWorkflowRun {
                cancellation: CancellationToken::new(),
                project_dir: PathBuf::new(),
            })
    }

    async fn durable_callback(
        &self,
        run_id: &str,
        key: &str,
        kind: DurableStepKind,
        input: Option<Value>,
        callback_id: &str,
        invoker: &CallbackInvoker,
    ) -> Result<Value, RpcError> {
        let callback_id = callback_id.to_owned();
        let invoker = invoker.clone();
        self.durable_value(run_id, key, kind, input, move || {
            let callback_id = callback_id.clone();
            let invoker = invoker.clone();
            async move {
                invoker
                    .invoke(callback_id, Vec::new())
                    .await
                    .map_err(|error| rpc_internal(error.to_string()))
            }
        })
        .await
    }

    async fn durable_value<F, Future>(
        &self,
        run_id: &str,
        key: &str,
        kind: DurableStepKind,
        input: Option<Value>,
        operation: F,
    ) -> Result<Value, RpcError>
    where
        F: FnOnce() -> Future,
        Future: std::future::Future<Output = Result<Value, RpcError>>,
    {
        if key.is_empty()
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        {
            return Err(rpc_invalid(format!(
                "Workflow step key \"{key}\" must contain only letters, numbers, \".\", \"_\" or \
                 \"-\"."
            )));
        }
        if let Some(existing) = self
            .durability
            .step(run_id, key)
            .await
            .map_err(rpc_internal)?
        {
            if existing.kind != kind {
                return Err(rpc_invalid(format!(
                    "Workflow step \"{key}\" was previously used as {:?}.",
                    existing.kind
                )));
            }
            if existing.input != input {
                return Err(rpc_invalid(format!(
                    "Workflow step \"{key}\" changed its input."
                )));
            }
            if existing.completed {
                return existing.output.ok_or_else(|| {
                    rpc_internal(format!("Workflow step \"{key}\" has no stored output."))
                });
            }
        } else {
            self.durability
                .start_step(run_id, key, kind, input.as_ref())
                .await
                .map_err(rpc_internal)?;
        }
        let output = operation().await?;
        self.durability
            .complete_step(run_id, key, &output)
            .await
            .map_err(rpc_internal)?;
        Ok(output)
    }

    async fn human_callback(&self, prompt: &HumanCallback) -> Result<Value, RpcError> {
        let serialized = serde_json::to_vec(prompt).map_err(rpc_internal)?;
        let prompt_hash = hex_prefix(Sha256::digest(serialized).as_slice(), 16);
        let occurrence = {
            let mut occurrences = self
                .human_occurrences
                .lock()
                .map_err(|error| rpc_internal(error.to_string()))?;
            let occurrence = occurrences
                .entry((prompt.run_id.clone(), prompt_hash.clone()))
                .or_default();
            let current = *occurrence;
            *occurrence += 1;
            current
        };
        let key = format!("human.{prompt_hash}.{occurrence}");
        let input = serde_json::to_value(prompt).map_err(rpc_internal)?;
        if let Some(existing) = self
            .durability
            .step(&prompt.run_id, &key)
            .await
            .map_err(rpc_internal)?
        {
            if existing.kind != DurableStepKind::Human || existing.input.as_ref() != Some(&input) {
                return Err(rpc_invalid(format!(
                    "Workflow human step \"{key}\" is invalid."
                )));
            }
            if existing.completed {
                return existing
                    .output
                    .ok_or_else(|| rpc_internal("Stored human response is missing."));
            }
        } else {
            self.durability
                .start_step(&prompt.run_id, &key, DurableStepKind::Human, Some(&input))
                .await
                .map_err(rpc_internal)?;
        }
        let response = {
            let _human_guard = self.human_gate.lock().await;
            self.human
                .request(&prompt.run_id, prompt)
                .await
                .map_err(rpc_internal)?
        };
        let Some(response) = response else {
            return Err(RpcError::new(
                HUMAN_SUSPENDED_ERROR,
                "workflow is waiting for human input",
            ));
        };
        self.durability
            .complete_step(&prompt.run_id, &key, &response)
            .await
            .map_err(rpc_internal)?;
        Ok(response)
    }

    async fn exec_callback(&self, request: &ExecCallback) -> Result<Value, RpcError> {
        if request.command.trim().is_empty() {
            return Err(rpc_invalid("Command cannot be empty."));
        }
        let options = ExecOptions::parse(&request.options)?;
        let active_run = self.active_run(&request.run_id);
        let mut command = Command::new(&request.command);
        command
            .args(&request.args)
            .kill_on_drop(true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        command.process_group(0);
        if let Some(cwd) = options.cwd.as_ref() {
            command.current_dir(cwd);
        } else if !active_run.project_dir.as_os_str().is_empty() {
            command.current_dir(&active_run.project_dir);
        }
        for (key, value) in &options.environment {
            command.env(key, value);
        }
        let mut child = command.spawn().map_err(rpc_internal)?;
        let pid = child.id();
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| rpc_internal("workflow command stdin pipe was not created"))?;
        if !options.input.is_empty() {
            stdin
                .write_all(options.input.as_bytes())
                .await
                .map_err(rpc_internal)?;
        }
        drop(stdin);
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| rpc_internal("workflow command stdout pipe was not created"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| rpc_internal("workflow command stderr pipe was not created"))?;
        let output_limit = CancellationToken::new();
        let output_bytes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let stdout_task = tokio::spawn(read_command_output(
            stdout,
            options.max_output_bytes,
            Arc::clone(&output_bytes),
            output_limit.clone(),
        ));
        let stderr_task = tokio::spawn(read_command_output(
            stderr,
            options.max_output_bytes,
            output_bytes,
            output_limit.clone(),
        ));
        let timeout = options
            .timeout
            .map_or_else(CancellationToken::new, |duration| {
                let timeout = CancellationToken::new();
                let trigger = timeout.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(duration).await;
                    trigger.cancel();
                });
                timeout
            });
        let status = tokio::select! {
            () = active_run.cancellation.cancelled() => {
                terminate_process_tree(&mut child, pid).await;
                return Err(RpcError::new(RpcError::CANCELLED, format!(
                    "Command \"{}\" was cancelled.",
                    request.command
                )));
            }
            () = timeout.cancelled(), if options.timeout.is_some() => {
                terminate_process_tree(&mut child, pid).await;
                return Err(rpc_internal(format!(
                    "Command \"{}\" timed out after {}ms.",
                    request.command,
                    options.timeout.map_or(0, |duration| duration.as_millis())
                )));
            }
            () = output_limit.cancelled() => {
                terminate_process_tree(&mut child, pid).await;
                return Err(rpc_internal(format!(
                    "Command \"{}\" exceeded the {}-byte output limit.",
                    request.command,
                    options.max_output_bytes
                )));
            }
            result = child.wait() => result.map_err(rpc_internal)?,
        };
        let stdout = join_command_output(stdout_task).await?;
        let stderr = join_command_output(stderr_task).await?;
        let exit_code = status.code().unwrap_or(-1);
        let result = json!({
            "command": request.command,
            "args": request.args,
            "stdout": String::from_utf8_lossy(&stdout),
            "stderr": String::from_utf8_lossy(&stderr),
            "exitCode": exit_code
        });
        if status.success() || options.allow_failure {
            Ok(result)
        } else {
            let message = String::from_utf8_lossy(&stderr).trim().to_owned();
            Err(rpc_internal(if message.is_empty() {
                format!("\"{}\" exited with code {exit_code}.", request.command)
            } else {
                message
            }))
        }
    }

    async fn map_callback(
        &self,
        request: &MapCallback,
        invoker: &CallbackInvoker,
    ) -> Result<Value, RpcError> {
        if request.concurrency == 0 {
            return Err(rpc_invalid("Map concurrency must be a positive integer."));
        }
        let concurrency = usize::try_from(request.concurrency).unwrap_or(usize::MAX);
        let callback_id = request.callback_id.clone();
        let invoker = invoker.clone();
        map_concurrently(request.items.clone(), concurrency, move |index, item| {
            let invoker = invoker.clone();
            let callback_id = callback_id.clone();
            async move {
                invoker
                    .invoke(callback_id, vec![item, json!(index)])
                    .await
                    .map_err(|error| rpc_internal(error.to_string()))
            }
        })
        .await
    }

    async fn elevation_callback(
        &self,
        request: &ElevationCallback,
        invoker: &CallbackInvoker,
    ) -> Result<Value, RpcError> {
        if request.attempts == 0 {
            return Err(rpc_invalid(
                "Elevation attempts must be a positive integer.",
            ));
        }
        let context = request
            .context
            .as_object()
            .ok_or_else(|| rpc_invalid("Elevation context must be an object."))?;
        let mode = context
            .get("mode")
            .and_then(Value::as_str)
            .ok_or_else(|| rpc_invalid("Elevation context mode must be a string."))?;
        let session = match mode {
            "fresh" => {
                self.agents
                    .create(&request.run_id, Some(&request.model))
                    .await
            }
            "fork" | "reuse" => {
                let session_id = context
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        rpc_invalid(format!("Elevation {mode} context requires a sessionId."))
                    })?;
                if mode == "fork" {
                    self.agents
                        .fork(&request.run_id, session_id, Some(&request.model))
                        .await
                } else {
                    self.agents
                        .retarget(&request.run_id, session_id, &request.model)
                        .await
                }
            }
            _ => {
                return Err(rpc_invalid(format!(
                    "Unsupported elevation context mode \"{mode}\"."
                )));
            }
        }
        .map_err(rpc_internal)?;
        let mut results = Vec::new();
        let mut checks = Vec::new();
        for attempt in 1..=request.attempts {
            let value = invoker
                .invoke(
                    &request.operation_callback_id,
                    vec![json!({
                        "attempt": attempt,
                        "previousResults": results,
                        "session": session
                    })],
                )
                .await
                .map_err(|error| rpc_internal(error.to_string()))?;
            let check = invoker
                .invoke(&request.check_callback_id, vec![value.clone()])
                .await
                .map_err(|error| rpc_internal(error.to_string()))?;
            let passed = check
                .as_bool()
                .or_else(|| check.get("passed").and_then(Value::as_bool))
                .unwrap_or(false);
            results.push(value.clone());
            checks.push(check);
            if passed {
                return Ok(value);
            }
        }
        if let Some(fallback_callback_id) = &request.fallback_callback_id {
            return invoker
                .invoke(
                    fallback_callback_id,
                    vec![json!({
                        "results": results,
                        "checks": checks,
                        "session": session
                    })],
                )
                .await
                .map_err(|error| rpc_internal(error.to_string()));
        }
        Err(rpc_internal("Workflow elevation attempts were exhausted."))
    }

    async fn agent_run_callback(&self, request: &AgentRunCallback) -> Result<Value, RpcError> {
        self.agents
            .run(request)
            .await
            .and_then(|result| serde_json::to_value(result).map_err(|error| error.to_string()))
            .map_err(rpc_internal)
    }
}

async fn map_concurrently<F, Fut>(
    items: Vec<Value>,
    concurrency: usize,
    operation: F,
) -> Result<Value, RpcError>
where
    F: Fn(usize, Value) -> Fut + Clone,
    Fut: std::future::Future<Output = Result<Value, RpcError>>,
{
    let mut results = stream::iter(items.into_iter().enumerate())
        .map(move |(index, item)| {
            let operation = operation.clone();
            async move { operation(index, item).await.map(|value| (index, value)) }
        })
        .buffer_unordered(concurrency)
        .try_collect::<Vec<_>>()
        .await?;
    results.sort_by_key(|(index, _)| *index);
    Ok(Value::Array(
        results.into_iter().map(|(_, value)| value).collect(),
    ))
}

struct ExecOptions {
    cwd: Option<PathBuf>,
    environment: BTreeMap<String, String>,
    input: String,
    timeout: Option<Duration>,
    max_output_bytes: usize,
    allow_failure: bool,
}

impl ExecOptions {
    fn parse(value: &Value) -> Result<Self, RpcError> {
        let object = value
            .as_object()
            .ok_or_else(|| rpc_invalid("Command options must be an object."))?;
        let cwd = optional_string(object.get("cwd"), "Command cwd")?.map(PathBuf::from);
        let input = optional_string(object.get("input"), "Command input")?
            .unwrap_or_default()
            .to_owned();
        let timeout = optional_positive_integer(object.get("timeoutMs"), "Command timeoutMs")?
            .map(Duration::from_millis);
        let max_output_bytes =
            optional_positive_integer(object.get("maxOutputBytes"), "Command maxOutputBytes")?
                .map_or(Ok(DEFAULT_MAX_OUTPUT_BYTES), |bytes| {
                    usize::try_from(bytes)
                        .map_err(|_| rpc_invalid("Command maxOutputBytes is too large."))
                })?;
        let allow_failure = object.get("allowFailure").map_or(Ok(false), |value| {
            value
                .as_bool()
                .ok_or_else(|| rpc_invalid("Command allowFailure must be a boolean."))
        })?;
        let environment = object.get("env").map_or_else(
            || Ok(BTreeMap::new()),
            |value| {
                value
                    .as_object()
                    .ok_or_else(|| rpc_invalid("Command env must be an object."))?
                    .iter()
                    .map(|(key, value)| {
                        value
                            .as_str()
                            .map(|value| (key.clone(), value.to_owned()))
                            .ok_or_else(|| {
                                rpc_invalid(format!(
                                    "Command environment variable \"{key}\" must be a string."
                                ))
                            })
                    })
                    .collect()
            },
        )?;
        Ok(Self {
            cwd,
            environment,
            input,
            timeout,
            max_output_bytes,
            allow_failure,
        })
    }
}

fn optional_string<'value>(
    value: Option<&'value Value>,
    label: &str,
) -> Result<Option<&'value str>, RpcError> {
    value
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| rpc_invalid(format!("{label} must be a string.")))
        })
        .transpose()
}

fn optional_positive_integer(value: Option<&Value>, label: &str) -> Result<Option<u64>, RpcError> {
    value
        .map(|value| {
            value
                .as_u64()
                .filter(|value| *value > 0)
                .ok_or_else(|| rpc_invalid(format!("{label} must be a positive integer.")))
        })
        .transpose()
}

async fn read_command_output(
    mut reader: impl AsyncRead + Unpin,
    max_output_bytes: usize,
    output_bytes: Arc<std::sync::atomic::AtomicUsize>,
    output_limit: CancellationToken,
) -> std::io::Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8_192];
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            return Ok(output);
        }
        let previous = output_bytes.fetch_add(read, std::sync::atomic::Ordering::Relaxed);
        if previous.saturating_add(read) > max_output_bytes {
            output_limit.cancel();
            return Ok(output);
        }
        output.extend_from_slice(&buffer[..read]);
    }
}

async fn join_command_output(
    task: JoinHandle<std::io::Result<Vec<u8>>>,
) -> Result<Vec<u8>, RpcError> {
    task.await.map_err(rpc_internal)?.map_err(rpc_internal)
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

#[async_trait]
impl WorkflowCallbackHandler for WorkflowCallbackServices {
    async fn handle(
        &self,
        request: WorkflowCallbackRequest,
        invoker: CallbackInvoker,
    ) -> Result<Value, RpcError> {
        match request {
            WorkflowCallbackRequest::Checkpoint(request) => {
                self.durable_callback(
                    &request.run_id,
                    &request.key,
                    DurableStepKind::Checkpoint,
                    None,
                    &request.callback_id,
                    &invoker,
                )
                .await
            }
            WorkflowCallbackRequest::Effect(request) => {
                self.durable_callback(
                    &request.run_id,
                    &request.key,
                    DurableStepKind::Effect,
                    Some(json!({"idempotencyKey": request.idempotency_key})),
                    &request.callback_id,
                    &invoker,
                )
                .await
            }
            WorkflowCallbackRequest::Exec(request) => self.exec_callback(&request).await,
            WorkflowCallbackRequest::Map(request) => self.map_callback(&request, &invoker).await,
            WorkflowCallbackRequest::AgentCreate(request) => self
                .agents
                .create(&request.run_id, request.model.as_deref())
                .await
                .and_then(|session| {
                    serde_json::to_value(session).map_err(|error| error.to_string())
                })
                .map_err(rpc_internal),
            WorkflowCallbackRequest::AgentFork(request) => self
                .agents
                .fork(
                    &request.run_id,
                    &request.session_id,
                    request.model.as_deref(),
                )
                .await
                .and_then(|session| {
                    serde_json::to_value(session).map_err(|error| error.to_string())
                })
                .map_err(rpc_internal),
            WorkflowCallbackRequest::AgentRun(request) => self.agent_run_callback(&request).await,
            WorkflowCallbackRequest::Human(request) => self.human_callback(&request).await,
            WorkflowCallbackRequest::Elevate(request) => {
                self.elevation_callback(&request, &invoker).await
            }
            WorkflowCallbackRequest::Log(request) => {
                self.logs
                    .log(&request.run_id, &request.message, request.data.as_ref());
                Ok(Value::Null)
            }
            WorkflowCallbackRequest::Unknown { method, .. } => Err(RpcError::new(
                RpcError::METHOD_NOT_FOUND,
                format!("unsupported workflow callback {method}"),
            )),
        }
    }
}

#[derive(Debug, Error)]
pub enum WorkflowRunError {
    #[error("{0}")]
    Persistence(String),
    #[error(transparent)]
    Host(#[from] WorkflowHostError),
    #[error("workflow source changed after discovery")]
    SourceChanged,
    #[error("workflow run \"{0}\" was not found")]
    RunNotFound(String),
    #[error("workflow run \"{run_id}\" cannot be resumed from status {status:?}")]
    InvalidResumeStatus {
        run_id: String,
        status: DurableRunStatus,
    },
    #[error("workflow run \"{run_id}\" belongs to workflow \"{actual}\", not \"{expected}\"")]
    WrongWorkflow {
        run_id: String,
        expected: String,
        actual: String,
    },
}

pub struct WorkflowRunner {
    host: Arc<WorkflowHost>,
    durability: Arc<dyn WorkflowDurability>,
    callbacks: WorkflowCallbackServices,
}

impl WorkflowRunner {
    #[must_use]
    pub fn new(
        host: Arc<WorkflowHost>,
        durability: Arc<dyn WorkflowDurability>,
        callbacks: WorkflowCallbackServices,
    ) -> Self {
        Self {
            host,
            durability,
            callbacks,
        }
    }

    pub async fn run(
        &self,
        record: &WorkflowRecord,
        project_dir: &Path,
        input: Value,
        cancellation: CancellationToken,
    ) -> Result<Value, WorkflowRunError> {
        if fingerprint_directory(&record.directory).ok().as_deref() != Some(&record.fingerprint) {
            return Err(WorkflowRunError::SourceChanged);
        }
        let run_id = Uuid::new_v4().to_string();
        self.durability
            .create_run(&run_id, record, project_dir, &input)
            .await
            .map_err(WorkflowRunError::Persistence)?;
        self.execute(&run_id, record, project_dir, input, cancellation)
            .await
    }

    pub async fn resume(
        &self,
        run_id: &str,
        record: &WorkflowRecord,
        cancellation: CancellationToken,
    ) -> Result<Value, WorkflowRunError> {
        let run = self
            .durability
            .load_run(run_id)
            .await
            .map_err(WorkflowRunError::Persistence)?
            .ok_or_else(|| WorkflowRunError::RunNotFound(run_id.to_owned()))?;
        if run.workflow_name != record.metadata.name {
            return Err(WorkflowRunError::WrongWorkflow {
                run_id: run_id.to_owned(),
                expected: record.metadata.name.clone(),
                actual: run.workflow_name,
            });
        }
        if run.status == DurableRunStatus::Completed {
            return run.output.ok_or_else(|| {
                WorkflowRunError::Persistence(format!(
                    "Completed workflow run \"{run_id}\" has no stored output."
                ))
            });
        }
        if !matches!(
            run.status,
            DurableRunStatus::Queued
                | DurableRunStatus::Running
                | DurableRunStatus::Waiting
                | DurableRunStatus::Interrupted
        ) {
            return Err(WorkflowRunError::InvalidResumeStatus {
                run_id: run_id.to_owned(),
                status: run.status,
            });
        }
        if run.source_entry_path != record.entry_path
            || run.source_fingerprint != record.fingerprint
            || !source_matches(record)
        {
            if run.status != DurableRunStatus::Running {
                self.durability
                    .mark_running(run_id)
                    .await
                    .map_err(WorkflowRunError::Persistence)?;
            }
            self.durability
                .mark_run(
                    run_id,
                    "version-mismatch",
                    Some("The workflow source changed after this run started."),
                )
                .await
                .map_err(WorkflowRunError::Persistence)?;
            return Err(WorkflowRunError::SourceChanged);
        }
        if run.status == DurableRunStatus::Running {
            self.durability
                .mark_run(run_id, "interrupted", None)
                .await
                .map_err(WorkflowRunError::Persistence)?;
        }
        self.execute(run_id, record, &run.project_dir, run.input, cancellation)
            .await
    }

    async fn execute(
        &self,
        run_id: &str,
        record: &WorkflowRecord,
        project_dir: &Path,
        input: Value,
        cancellation: CancellationToken,
    ) -> Result<Value, WorkflowRunError> {
        self.durability
            .mark_running(run_id)
            .await
            .map_err(WorkflowRunError::Persistence)?;
        self.callbacks
            .register_run(run_id, project_dir, cancellation.clone());
        let execution = self.host.run(RunWorkflowParams {
            entry_path: record.entry_path.display().to_string(),
            run_id: run_id.to_owned(),
            project_dir: project_dir.display().to_string(),
            input,
        });
        let result = tokio::select! {
            () = cancellation.cancelled() => {
                let _cancel_result = self.host.cancel(run_id, Some("cancelled".to_owned())).await;
                self.durability.mark_run(run_id, "cancelled", None).await
                    .map_err(WorkflowRunError::Persistence)?;
                Err(WorkflowHostError::Remote(RpcError::CANCELLED, "workflow cancelled".to_owned()))
            }
            result = execution => result,
        };
        self.callbacks.unregister_run(run_id);
        match result {
            Ok(result) => {
                self.durability
                    .complete_run(run_id, &result.value, result.presentation)
                    .await
                    .map_err(WorkflowRunError::Persistence)?;
                Ok(result.value)
            }
            Err(WorkflowHostError::Remote(code, error)) if code == HUMAN_SUSPENDED_ERROR => {
                self.durability
                    .mark_run(run_id, "waiting", Some(&error))
                    .await
                    .map_err(WorkflowRunError::Persistence)?;
                Err(WorkflowHostError::Remote(code, error).into())
            }
            Err(error) => {
                self.durability
                    .mark_run(run_id, "failed", Some(&error.to_string()))
                    .await
                    .map_err(WorkflowRunError::Persistence)?;
                Err(error.into())
            }
        }
    }
}

fn source_matches(record: &WorkflowRecord) -> bool {
    fingerprint_directory(&record.directory).ok().as_deref() == Some(&record.fingerprint)
}

fn rpc_invalid(message: impl ToString) -> RpcError {
    RpcError::new(RpcError::INVALID_PARAMS, message.to_string())
}

fn rpc_internal(message: impl ToString) -> RpcError {
    RpcError::new(RpcError::INTERNAL_ERROR, message.to_string())
}

fn hex_prefix(bytes: &[u8], characters: usize) -> String {
    bytes
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .take(characters)
        .map(|digit| char::from_digit(u32::from(digit), 16).unwrap_or('0'))
        .collect()
}

#[cfg(test)]
#[path = "workflow_tests.rs"]
mod tests;
