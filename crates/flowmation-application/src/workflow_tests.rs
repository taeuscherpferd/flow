#![allow(clippy::expect_used)]

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use flowmation_domain::agent::PackageSource;
use flowmation_domain::fingerprint::fingerprint_directory;
use flowmation_workflow_host::protocol::{
    AgentInvocationPolicy, AgentRunCallback, AgentRunOptions, AgentRunResult, AgentSession,
    ExecCallback, HumanCallback, HumanRequestKind, ModelRef, WorkflowMetadata,
    WorkflowPresentation, WorkflowThinking, WorkflowTools,
};
use flowmation_workflow_host::{WorkflowHost, WorkflowHostConfig, WorkflowHostError};
use serde_json::{Value, json};
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;

use super::{
    DurableRun, DurableRunStatus, DurableStep, DurableStepKind, HumanRequestBroker,
    WorkflowAgentRuntime, WorkflowCallbackServices, WorkflowDurability, WorkflowInspector,
    WorkflowLogSink, WorkflowRecord, WorkflowRegistry, WorkflowRegistryRoot, WorkflowRunError,
    WorkflowRunner, map_concurrently, source_matches,
};

#[derive(Default)]
struct MemoryDurability {
    runs: Mutex<HashMap<String, DurableRun>>,
    steps: Mutex<HashMap<(String, String), DurableStep>>,
}

#[async_trait]
impl WorkflowDurability for MemoryDurability {
    async fn create_run(
        &self,
        run_id: &str,
        record: &WorkflowRecord,
        project_dir: &Path,
        input: &Value,
    ) -> Result<(), String> {
        self.runs.lock().map_err(|error| error.to_string())?.insert(
            run_id.to_owned(),
            DurableRun {
                workflow_name: record.metadata.name.clone(),
                project_dir: project_dir.to_path_buf(),
                source_entry_path: record.entry_path.clone(),
                source_fingerprint: record.fingerprint.clone(),
                status: DurableRunStatus::Queued,
                input: input.clone(),
                output: None,
            },
        );
        Ok(())
    }

    async fn mark_running(&self, run_id: &str) -> Result<(), String> {
        self.run_mut(run_id, |run| run.status = DurableRunStatus::Running)?;
        Ok(())
    }

    async fn load_run(&self, run_id: &str) -> Result<Option<DurableRun>, String> {
        Ok(self
            .runs
            .lock()
            .map_err(|error| error.to_string())?
            .get(run_id)
            .cloned())
    }

    async fn complete_run(
        &self,
        run_id: &str,
        output: &Value,
        _presentation: WorkflowPresentation,
    ) -> Result<(), String> {
        self.run_mut(run_id, |run| {
            run.status = DurableRunStatus::Completed;
            run.output = Some(output.clone());
        })?;
        Ok(())
    }

    async fn mark_run(
        &self,
        run_id: &str,
        status: &str,
        _error: Option<&str>,
    ) -> Result<(), String> {
        let status = match status {
            "waiting" => DurableRunStatus::Waiting,
            "interrupted" => DurableRunStatus::Interrupted,
            "failed" => DurableRunStatus::Failed,
            "cancelled" => DurableRunStatus::Cancelled,
            "version-mismatch" => DurableRunStatus::VersionMismatch,
            value => return Err(format!("unsupported test status {value}")),
        };
        self.run_mut(run_id, |run| run.status = status)?;
        Ok(())
    }

    async fn step(&self, run_id: &str, key: &str) -> Result<Option<DurableStep>, String> {
        Ok(self
            .steps
            .lock()
            .map_err(|error| error.to_string())?
            .get(&(run_id.to_owned(), key.to_owned()))
            .cloned())
    }

    async fn start_step(
        &self,
        run_id: &str,
        key: &str,
        kind: DurableStepKind,
        input: Option<&Value>,
    ) -> Result<(), String> {
        self.steps
            .lock()
            .map_err(|error| error.to_string())?
            .insert(
                (run_id.to_owned(), key.to_owned()),
                DurableStep {
                    kind,
                    input: input.cloned(),
                    output: None,
                    completed: false,
                },
            );
        Ok(())
    }

    async fn complete_step(&self, run_id: &str, key: &str, output: &Value) -> Result<(), String> {
        let mut steps = self.steps.lock().map_err(|error| error.to_string())?;
        let step = steps
            .get_mut(&(run_id.to_owned(), key.to_owned()))
            .ok_or_else(|| "step was not started".to_owned())?;
        step.output = Some(output.clone());
        step.completed = true;
        Ok(())
    }
}

impl MemoryDurability {
    fn run_mut(&self, run_id: &str, update: impl FnOnce(&mut DurableRun)) -> Result<(), String> {
        let mut runs = self.runs.lock().map_err(|error| error.to_string())?;
        let run = runs
            .get_mut(run_id)
            .ok_or_else(|| format!("run {run_id} does not exist"))?;
        update(run);
        Ok(())
    }

    fn only_run(&self) -> DurableRun {
        self.runs
            .lock()
            .expect("runs should lock")
            .values()
            .next()
            .expect("one run should exist")
            .clone()
    }

    fn only_run_id(&self) -> String {
        self.runs
            .lock()
            .expect("runs should lock")
            .keys()
            .next()
            .expect("one run should exist")
            .clone()
    }
}

struct QueuedHumanBroker {
    responses: Mutex<VecDeque<Option<Value>>>,
    requests: AtomicUsize,
}

struct ConcurrentHumanBroker {
    active: AtomicUsize,
    maximum: AtomicUsize,
    requests: AtomicUsize,
    fail_first: bool,
}

#[async_trait]
impl HumanRequestBroker for ConcurrentHumanBroker {
    async fn request(
        &self,
        _run_id: &str,
        prompt: &HumanCallback,
    ) -> Result<Option<Value>, String> {
        let request = self.requests.fetch_add(1, Ordering::SeqCst);
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.maximum.fetch_max(active, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(20)).await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        if self.fail_first && request == 0 {
            Err("prompt failed".to_owned())
        } else {
            Ok(Some(json!(prompt.prompt)))
        }
    }
}

#[async_trait]
impl HumanRequestBroker for QueuedHumanBroker {
    async fn request(
        &self,
        _run_id: &str,
        _prompt: &HumanCallback,
    ) -> Result<Option<Value>, String> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        self.responses
            .lock()
            .map_err(|error| error.to_string())?
            .pop_front()
            .ok_or_else(|| "no queued human response".to_owned())
    }
}

#[derive(Default)]
struct RecordingAgents {
    requests: Mutex<Vec<AgentRunCallback>>,
}

#[async_trait]
impl WorkflowAgentRuntime for RecordingAgents {
    async fn create(&self, _run_id: &str, _model: Option<&str>) -> Result<AgentSession, String> {
        Ok(test_session())
    }

    async fn fork(
        &self,
        _run_id: &str,
        _session_id: &str,
        _model: Option<&str>,
    ) -> Result<AgentSession, String> {
        Ok(test_session())
    }

    async fn retarget(
        &self,
        _run_id: &str,
        _session_id: &str,
        _model: &str,
    ) -> Result<AgentSession, String> {
        Ok(test_session())
    }

    async fn run(&self, request: &AgentRunCallback) -> Result<AgentRunResult, String> {
        self.requests
            .lock()
            .map_err(|error| error.to_string())?
            .push(request.clone());
        Ok(AgentRunResult {
            content: "done".to_owned(),
            model: test_session().model,
        })
    }
}

struct SilentLogs;

impl WorkflowLogSink for SilentLogs {
    fn log(&self, _run_id: &str, _message: &str, _data: Option<&Value>) {}
}

fn test_session() -> AgentSession {
    AgentSession {
        id: "session-1".to_owned(),
        model: ModelRef {
            provider: "test".to_owned(),
            model: "small".to_owned(),
            active: true,
        },
    }
}

fn services(
    durability: Arc<MemoryDurability>,
    human: Arc<dyn HumanRequestBroker>,
    agents: Arc<RecordingAgents>,
) -> WorkflowCallbackServices {
    WorkflowCallbackServices::new(durability, human, agents, Arc::new(SilentLogs))
}

#[tokio::test]
async fn concurrent_human_requests_are_serialized() {
    let broker = Arc::new(ConcurrentHumanBroker {
        active: AtomicUsize::new(0),
        maximum: AtomicUsize::new(0),
        requests: AtomicUsize::new(0),
        fail_first: false,
    });
    let callbacks = services(
        Arc::new(MemoryDurability::default()),
        broker.clone(),
        Arc::new(RecordingAgents::default()),
    );
    let first = human_prompt("run-1", "First?");
    let second = human_prompt("run-1", "Second?");

    let (first_result, second_result) = tokio::join!(
        callbacks.human_callback(&first),
        callbacks.human_callback(&second)
    );

    assert_eq!(
        first_result.expect("first prompt should complete"),
        "First?"
    );
    assert_eq!(
        second_result.expect("second prompt should complete"),
        "Second?"
    );
    assert_eq!(broker.maximum.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn human_request_queue_continues_after_a_rejected_prompt() {
    let broker = Arc::new(ConcurrentHumanBroker {
        active: AtomicUsize::new(0),
        maximum: AtomicUsize::new(0),
        requests: AtomicUsize::new(0),
        fail_first: true,
    });
    let callbacks = services(
        Arc::new(MemoryDurability::default()),
        broker,
        Arc::new(RecordingAgents::default()),
    );

    assert_eq!(
        callbacks
            .human_callback(&human_prompt("run-1", "First?"))
            .await
            .expect_err("first prompt should fail")
            .message,
        "prompt failed"
    );
    assert_eq!(
        callbacks
            .human_callback(&human_prompt("run-1", "Second?"))
            .await
            .expect("the queue should recover"),
        "Second?"
    );
}

fn human_prompt(run_id: &str, prompt: &str) -> HumanCallback {
    HumanCallback {
        run_id: run_id.to_owned(),
        kind: HumanRequestKind::Text,
        prompt: prompt.to_owned(),
        details: None,
        choices: None,
    }
}

#[tokio::test]
async fn completed_checkpoint_and_effect_values_are_reused() {
    let durability = Arc::new(MemoryDurability::default());
    let callbacks = services(
        Arc::clone(&durability),
        Arc::new(QueuedHumanBroker {
            responses: Mutex::new(VecDeque::new()),
            requests: AtomicUsize::new(0),
        }),
        Arc::new(RecordingAgents::default()),
    );
    let calls = Arc::new(AtomicUsize::new(0));

    for (kind, key, input) in [
        (DurableStepKind::Checkpoint, "draft", None),
        (
            DurableStepKind::Effect,
            "publish",
            Some(json!({"idempotencyKey": "branch-123"})),
        ),
    ] {
        let calls_for_first = Arc::clone(&calls);
        let first = callbacks
            .durable_value("run-1", key, kind, input.clone(), move || async move {
                calls_for_first.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"saved": true}))
            })
            .await
            .expect("the first durable operation should complete");
        let calls_for_resume = Arc::clone(&calls);
        let resumed = callbacks
            .durable_value("run-1", key, kind, input, move || async move {
                calls_for_resume.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"saved": false}))
            })
            .await
            .expect("the resumed durable operation should reuse its output");
        assert_eq!(first, resumed);
    }

    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn human_response_uses_the_same_occurrence_after_suspension() {
    let durability = Arc::new(MemoryDurability::default());
    let human = Arc::new(QueuedHumanBroker {
        responses: Mutex::new(VecDeque::from([None, Some(json!("yes"))])),
        requests: AtomicUsize::new(0),
    });
    let callbacks = services(
        Arc::clone(&durability),
        human.clone(),
        Arc::new(RecordingAgents::default()),
    );
    let prompt = HumanCallback {
        run_id: "run-1".to_owned(),
        kind: HumanRequestKind::Text,
        prompt: "Continue?".to_owned(),
        details: None,
        choices: None,
    };

    callbacks.register_run("run-1", Path::new("/project"), CancellationToken::new());
    let suspended = callbacks.human_callback(&prompt).await;
    assert_eq!(
        suspended
            .expect_err("the first request should suspend")
            .code,
        -32_010
    );
    callbacks.unregister_run("run-1");

    callbacks.register_run("run-1", Path::new("/project"), CancellationToken::new());
    assert_eq!(
        callbacks
            .human_callback(&prompt)
            .await
            .expect("the resumed request should complete"),
        json!("yes")
    );
    callbacks.unregister_run("run-1");

    callbacks.register_run("run-1", Path::new("/project"), CancellationToken::new());
    assert_eq!(
        callbacks
            .human_callback(&prompt)
            .await
            .expect("a later resume should reuse the stored response"),
        json!("yes")
    );
    assert_eq!(human.requests.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn map_limits_concurrency_and_preserves_input_order() {
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let result = map_concurrently(vec![json!(1), json!(2), json!(3), json!(4)], 2, {
        let active = Arc::clone(&active);
        let maximum = Arc::clone(&maximum);
        move |index, value| {
            let active = Arc::clone(&active);
            let maximum = Arc::clone(&maximum);
            async move {
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                maximum.fetch_max(current, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis((4 - index) as u64 * 5)).await;
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(json!(value.as_i64().unwrap_or_default() * 2))
            }
        }
    })
    .await
    .expect("map should complete");

    assert_eq!(result, json!([2, 4, 6, 8]));
    assert_eq!(maximum.load(Ordering::SeqCst), 2);
}

#[cfg(unix)]
#[tokio::test]
async fn exec_captures_io_environment_and_failure_status() {
    let root = tempdir().expect("temporary directory should be created");
    let callbacks = services(
        Arc::new(MemoryDurability::default()),
        Arc::new(QueuedHumanBroker {
            responses: Mutex::new(VecDeque::new()),
            requests: AtomicUsize::new(0),
        }),
        Arc::new(RecordingAgents::default()),
    );
    callbacks.register_run("run-1", root.path(), CancellationToken::new());
    let success = callbacks
        .exec_callback(&ExecCallback {
            run_id: "run-1".to_owned(),
            command: "/bin/sh".to_owned(),
            args: vec![
                "-c".to_owned(),
                "read value; printf '%s:%s' \"$FLOW_TEST\" \"$value\"".to_owned(),
            ],
            options: json!({
                "env": {"FLOW_TEST": "environment"},
                "input": "stdin\n"
            }),
        })
        .await
        .expect("command should succeed");
    assert_eq!(success["stdout"], "environment:stdin");
    assert_eq!(success["command"], "/bin/sh");
    assert_eq!(success["exitCode"], 0);

    let failure = ExecCallback {
        run_id: "run-1".to_owned(),
        command: "/bin/sh".to_owned(),
        args: vec!["-c".to_owned(), "printf nope >&2; exit 7".to_owned()],
        options: json!({}),
    };
    assert_eq!(
        callbacks
            .exec_callback(&failure)
            .await
            .expect_err("a failed command should be rejected")
            .message,
        "nope"
    );
    let allowed = callbacks
        .exec_callback(&ExecCallback {
            options: json!({"allowFailure": true}),
            ..failure
        })
        .await
        .expect("allowFailure should return a failed command result");
    assert_eq!(allowed["exitCode"], 7);
    assert_eq!(allowed["stderr"], "nope");
}

#[cfg(unix)]
#[tokio::test]
async fn exec_cancellation_terminates_descendant_processes() {
    let root = tempdir().expect("temporary directory should be created");
    let started = root.path().join("started");
    let descendant = root.path().join("descendant");
    let cancellation = CancellationToken::new();
    let callbacks = Arc::new(services(
        Arc::new(MemoryDurability::default()),
        Arc::new(QueuedHumanBroker {
            responses: Mutex::new(VecDeque::new()),
            requests: AtomicUsize::new(0),
        }),
        Arc::new(RecordingAgents::default()),
    ));
    callbacks.register_run("run-1", root.path(), cancellation.clone());
    let request = ExecCallback {
        run_id: "run-1".to_owned(),
        command: "/bin/sh".to_owned(),
        args: vec![
            "-c".to_owned(),
            format!(
                "(sleep 0.4; touch '{}') & touch '{}'; sleep 30",
                descendant.display(),
                started.display()
            ),
        ],
        options: json!({}),
    };
    let execution_callbacks = Arc::clone(&callbacks);
    let execution = tokio::spawn(async move { execution_callbacks.exec_callback(&request).await });
    wait_for_path(&started).await;
    cancellation.cancel();

    let error = execution
        .await
        .expect("exec task should join")
        .expect_err("exec should be cancelled");
    assert_eq!(
        error.code,
        flowmation_workflow_host::protocol::RpcError::CANCELLED
    );
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(!descendant.exists());
}

async fn wait_for_path(path: &Path) {
    for _attempt in 0..100 {
        if path.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for {}", path.display());
}

struct PathMetadataInspector;

#[async_trait]
impl WorkflowInspector for PathMetadataInspector {
    async fn inspect(&self, entry_path: &Path) -> Result<WorkflowMetadata, String> {
        let name = entry_path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .ok_or_else(|| "entry has no workflow directory".to_owned())?;
        let description = std::fs::read_to_string(entry_path)
            .map_err(|error| error.to_string())?
            .trim()
            .to_owned();
        Ok(WorkflowMetadata {
            name: name.to_owned(),
            description,
            input_schema: None,
            agent_invocation: AgentInvocationPolicy::Disabled,
            presentation: WorkflowPresentation::Direct,
        })
    }
}

#[tokio::test]
async fn project_workflows_override_global_and_ambiguous_entries_are_skipped() {
    let root = tempdir().expect("temporary directory should be created");
    let global = root.path().join("global");
    let project = root.path().join("project");
    write_workflow(&global, "hello", "global");
    write_workflow(&project, "hello", "project");
    let ambiguous = global.join("ambiguous");
    std::fs::create_dir_all(&ambiguous).expect("ambiguous directory should be created");
    std::fs::write(ambiguous.join("WORKFLOW.js"), "ambiguous")
        .expect("JavaScript entry should be written");
    std::fs::write(ambiguous.join("WORKFLOW.ts"), "ambiguous")
        .expect("TypeScript entry should be written");
    let mut registry = WorkflowRegistry::new(
        vec![
            WorkflowRegistryRoot {
                directory: global,
                source: PackageSource::Global,
            },
            WorkflowRegistryRoot {
                directory: project,
                source: PackageSource::Project,
            },
        ],
        Arc::new(PathMetadataInspector),
        None,
        None,
    );

    registry.load().await.expect("registry should load");

    let hello = registry.get("hello").expect("hello should be discovered");
    assert_eq!(hello.source, PackageSource::Project);
    assert_eq!(hello.metadata.description, "project");
    assert!(registry.get("ambiguous").is_none());
    assert!(
        registry
            .warnings()
            .iter()
            .any(|warning| warning.contains("both WORKFLOW"))
    );
}

#[tokio::test]
async fn workflow_fingerprint_changes_when_a_helper_changes() {
    let root = tempdir().expect("temporary directory should be created");
    let workflows = root.path().join("workflows");
    let directory = workflows.join("dependent");
    write_workflow(&workflows, "dependent", "dependent workflow");
    std::fs::write(directory.join("helper.js"), "first").expect("helper should be written");
    let mut registry = WorkflowRegistry::new(
        vec![WorkflowRegistryRoot {
            directory: workflows,
            source: PackageSource::Project,
        }],
        Arc::new(PathMetadataInspector),
        None,
        None,
    );
    registry.load().await.expect("registry should load");
    let discovered = registry
        .get("dependent")
        .expect("workflow should be found")
        .clone();
    let first = discovered.fingerprint.clone();

    std::fs::write(root.path().join("workflows/dependent/helper.js"), "second")
        .expect("helper should be changed");
    assert!(!source_matches(&discovered));
    registry.load().await.expect("registry should reload");

    assert_ne!(
        registry
            .get("dependent")
            .expect("workflow should remain discoverable")
            .fingerprint,
        first
    );
}

fn write_workflow(root: &Path, name: &str, description: &str) {
    let directory = root.join(name);
    std::fs::create_dir_all(&directory).expect("workflow directory should be created");
    std::fs::write(directory.join("WORKFLOW.js"), description)
        .expect("workflow entry should be written");
}

struct IntegrationRuntime {
    _directory: tempfile::TempDir,
    record: WorkflowRecord,
    durability: Arc<MemoryDurability>,
    agents: Arc<RecordingAgents>,
    host: Arc<WorkflowHost>,
    runner: WorkflowRunner,
}

async fn integration_runtime(
    name: &str,
    source: &str,
    human_responses: VecDeque<Option<Value>>,
) -> Result<IntegrationRuntime, Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let workflow_directory = directory.path().join(name);
    std::fs::create_dir_all(&workflow_directory)?;
    let entry_path = workflow_directory.join("WORKFLOW.js");
    std::fs::write(&entry_path, source)?;
    let record = WorkflowRecord {
        metadata: WorkflowMetadata {
            name: name.to_owned(),
            description: format!("Test workflow {name}"),
            input_schema: None,
            agent_invocation: AgentInvocationPolicy::Disabled,
            presentation: WorkflowPresentation::Direct,
        },
        directory: workflow_directory.clone(),
        entry_path,
        fingerprint: fingerprint_directory(&workflow_directory)?,
        source: PackageSource::Project,
        agent_name: None,
        resource_id: None,
    };
    let durability = Arc::new(MemoryDurability::default());
    let agents = Arc::new(RecordingAgents::default());
    let callbacks = WorkflowCallbackServices::new(
        durability.clone(),
        Arc::new(QueuedHumanBroker {
            responses: Mutex::new(human_responses),
            requests: AtomicUsize::new(0),
        }),
        agents.clone(),
        Arc::new(SilentLogs),
    );
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let config = WorkflowHostConfig::new(workspace.join("workflow-host/dist/index.js"));
    let host = Arc::new(WorkflowHost::spawn(config, Arc::new(callbacks.clone())).await?);
    let runner = WorkflowRunner::new(host.clone(), durability.clone(), callbacks);
    Ok(IntegrationRuntime {
        _directory: directory,
        record,
        durability,
        agents,
        host,
        runner,
    })
}

#[tokio::test]
async fn runner_resume_reuses_checkpoint_and_human_but_reruns_ordinary_agent_calls()
-> Result<(), Box<dyn std::error::Error>> {
    let runtime = integration_runtime(
        "resumable",
        r#"
          import { defineWorkflow } from "flowmation/workflow";
          export default defineWorkflow({
            name: "resumable",
            description: "Exercises durable resume",
            async run(context) {
              const agent = await context.agents.create({ model: "small" });
              const ordinary = await agent.run("ordinary");
              const saved = await context.checkpoint(
                "saved-agent",
                () => agent.run("checkpointed"),
              );
              const answer = await context.human.ask({ prompt: "Continue?" });
              return {
                ordinary: ordinary.content,
                saved: saved.content,
                answer,
              };
            },
          });
        "#,
        VecDeque::from([None, Some(json!("yes"))]),
    )
    .await?;

    let first = runtime
        .runner
        .run(
            &runtime.record,
            runtime._directory.path(),
            json!(""),
            CancellationToken::new(),
        )
        .await;
    assert!(matches!(
        first,
        Err(WorkflowRunError::Host(WorkflowHostError::Remote(
            -32_010,
            _
        )))
    ));
    assert_eq!(
        runtime.durability.only_run().status,
        DurableRunStatus::Waiting
    );
    let run_id = runtime.durability.only_run_id();

    let output = runtime
        .runner
        .resume(&run_id, &runtime.record, CancellationToken::new())
        .await?;

    assert_eq!(
        output,
        json!({"ordinary": "done", "saved": "done", "answer": "yes"})
    );
    assert_eq!(
        runtime.durability.only_run().status,
        DurableRunStatus::Completed
    );
    let prompts = runtime
        .agents
        .requests
        .lock()
        .expect("agent requests should lock")
        .iter()
        .map(|request| request.prompt.clone())
        .collect::<Vec<_>>();
    assert_eq!(prompts, ["ordinary", "checkpointed", "ordinary"]);
    runtime.host.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn runner_resume_rejects_changed_source_and_records_version_mismatch()
-> Result<(), Box<dyn std::error::Error>> {
    let runtime = integration_runtime(
        "source-check",
        r#"
          import { defineWorkflow } from "flowmation/workflow";
          export default defineWorkflow({
            name: "source-check",
            description: "Waits for a response",
            async run(context) {
              await context.human.ask({ prompt: "Continue?" });
              return "complete";
            },
          });
        "#,
        VecDeque::from([None, Some(json!("yes"))]),
    )
    .await?;
    let first = runtime
        .runner
        .run(
            &runtime.record,
            runtime._directory.path(),
            json!(""),
            CancellationToken::new(),
        )
        .await;
    assert!(first.is_err());
    let run_id = runtime.durability.only_run_id();
    std::fs::write(runtime.record.directory.join("helper.js"), "changed")?;

    let resumed = runtime
        .runner
        .resume(&run_id, &runtime.record, CancellationToken::new())
        .await;

    assert!(matches!(resumed, Err(WorkflowRunError::SourceChanged)));
    assert_eq!(
        runtime.durability.only_run().status,
        DurableRunStatus::VersionMismatch
    );
    runtime.host.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn elevation_thinking_is_scoped_to_operation_session_runs()
-> Result<(), Box<dyn std::error::Error>> {
    let runtime = integration_runtime(
        "elevation-thinking",
        r#"
          import { defineWorkflow } from "flowmation/workflow";
          export default defineWorkflow({
            name: "elevation-thinking",
            description: "Scopes elevated thinking",
            async run(context) {
              const agent = await context.agents.create({ model: "small" });
              await context.elevate({
                model: "reviewer",
                thinking: "high",
                attempts: 1,
                context: { mode: "reuse", session: agent },
                operation: async ({ session }) => {
                  await session.run("elevated");
                  await session.run("explicit", { thinking: "off" });
                  return "checked";
                },
                check: () => true,
              });
              await agent.run("ordinary");
              return "complete";
            },
          });
        "#,
        VecDeque::new(),
    )
    .await?;

    assert_eq!(
        runtime
            .runner
            .run(
                &runtime.record,
                runtime._directory.path(),
                json!(""),
                CancellationToken::new(),
            )
            .await?,
        "complete"
    );
    let requests = runtime
        .agents
        .requests
        .lock()
        .expect(
            "agent requests should lock()
        ",
        )
        .clone();
    assert_eq!(
        requests
            .iter()
            .map(|request| request.options.thinking)
            .collect::<Vec<_>>(),
        [
            Some(WorkflowThinking::High),
            Some(WorkflowThinking::Off),
            None
        ]
    );
    assert!(
        requests
            .iter()
            .all(|request| request.options.tools == Some(WorkflowTools::Default))
    );
    runtime.host.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn agent_callback_forwards_thinking_and_tools_modes() {
    let agents = Arc::new(RecordingAgents::default());
    let callbacks = services(
        Arc::new(MemoryDurability::default()),
        Arc::new(QueuedHumanBroker {
            responses: Mutex::new(VecDeque::new()),
            requests: AtomicUsize::new(0),
        }),
        agents.clone(),
    );
    let request = AgentRunCallback {
        run_id: "run-1".to_owned(),
        session_id: "session-1".to_owned(),
        prompt: "review".to_owned(),
        options: AgentRunOptions {
            tools: Some(WorkflowTools::None),
            thinking: Some(WorkflowThinking::High),
        },
    };
    callbacks
        .agent_run_callback(&request)
        .await
        .expect("agent callback should complete");

    let recorded = agents.requests.lock().expect("requests should lock");
    assert_eq!(recorded.as_slice(), [request]);
}
