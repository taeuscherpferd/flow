mod durability;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use flowmation_application::{
    AgentManager, AuthorizationDecision, AuthorizationPolicy, ConfigService, DurableRunStatus,
    HumanRequestBroker, ManagedWorkflowAgentRuntime, ModelProvider, PermissionRequest,
    ScheduleExecution, ScheduleWorker, WorkerExecutionResult, WorkflowAgentRuntime,
    WorkflowCallbackServices, WorkflowDurability, WorkflowLogSink, WorkflowRegistry,
    WorkflowRegistryRoot, WorkflowRunner,
};
use flowmation_domain::agent::PackageSource;
use flowmation_domain::fingerprint::fingerprint_directory;
use flowmation_domain::schedule::{ScheduleOccurrence, ScheduleOccurrenceStatus, ScheduleRecord};
use flowmation_ollama::OllamaProvider;
use flowmation_sqlite::{SqliteApplicationRepository, SqliteDatabase};
use flowmation_workflow_host::protocol::HumanCallback;
use flowmation_workflow_host::{WorkflowHost, WorkflowHostConfig};
use serde_json::Value;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

use self::durability::ScheduledDurability;

const POLL_INTERVAL: Duration = Duration::from_secs(15);

pub async fn run(database_path: PathBuf, once: bool) -> Result<(), String> {
    let global_dir = database_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let database = SqliteDatabase::open(&database_path).map_err(|error| error.to_string())?;
    let repository = Arc::new(SqliteApplicationRepository::from_database(database));
    let execution = Arc::new(ScheduledWorkflowExecution { global_dir });
    let worker = ScheduleWorker::new(repository, execution);
    loop {
        worker.tick(Utc::now()).await?;
        if once {
            return Ok(());
        }
        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                return result.map_err(|error| format!("worker signal handler failed: {error}"));
            }
            () = tokio::time::sleep(POLL_INTERVAL) => {}
        }
    }
}

struct ScheduledWorkflowExecution {
    global_dir: PathBuf,
}

#[async_trait]
impl ScheduleExecution for ScheduledWorkflowExecution {
    async fn source_matches(&self, schedule: &ScheduleRecord) -> Result<bool, String> {
        let Some(source) = resolve_source(&self.global_dir, schedule) else {
            return Ok(false);
        };
        let expected = schedule.package_fingerprint.clone();
        tokio::task::spawn_blocking(move || {
            Ok(fingerprint_directory(&source.authorization_directory)
                .is_ok_and(|fingerprint| fingerprint == expected))
        })
        .await
        .map_err(|error| format!("schedule source verification task failed: {error}"))?
    }

    async fn execute(
        &self,
        schedule: &ScheduleRecord,
        occurrence: &ScheduleOccurrence,
    ) -> WorkerExecutionResult {
        match self.execute_workflow(schedule, occurrence).await {
            Ok(result) => result,
            Err(error) => WorkerExecutionResult {
                run_id: occurrence.run_id.as_ref().map(ToString::to_string),
                status: ScheduleOccurrenceStatus::Failed,
                result: None,
                error: Some(error),
            },
        }
    }
}

impl ScheduledWorkflowExecution {
    async fn execute_workflow(
        &self,
        schedule: &ScheduleRecord,
        occurrence: &ScheduleOccurrence,
    ) -> Result<WorkerExecutionResult, String> {
        let source = resolve_source(&self.global_dir, schedule)
            .ok_or_else(|| "The scheduled workflow source no longer exists.".to_owned())?;
        if !matches!(
            fingerprint_directory(&source.authorization_directory),
            Ok(fingerprint) if fingerprint == schedule.package_fingerprint
        ) {
            return Err("The scheduled workflow source changed before execution.".to_owned());
        }
        let project_config_dir = schedule.project_dir.join(".work-agent");
        let config = ConfigService::new(&self.global_dir, &project_config_dir)
            .load()
            .await
            .map_err(|error| error.to_string())?;
        let providers = config
            .models
            .providers
            .iter()
            .map(|(name, provider)| {
                (
                    name.clone(),
                    Arc::new(OllamaProvider::new(&provider.base_url)) as Arc<dyn ModelProvider>,
                )
            })
            .collect::<BTreeMap<_, _>>();
        let repository = Arc::new(
            SqliteApplicationRepository::open_global_dir(&self.global_dir)
                .map_err(|error| error.to_string())?,
        );
        let mut manager = AgentManager::create(
            config,
            providers,
            Arc::new(ScheduledAuthorizationPolicy),
            None,
            repository.clone(),
        )
        .await
        .map_err(|error| error.to_string())?;
        if schedule.agent_name != "main" {
            manager
                .switch_agent(&schedule.agent_name)
                .map_err(|error| error.to_string())?;
        }
        let manager = Arc::new(AsyncMutex::new(manager));
        let agents: Arc<dyn WorkflowAgentRuntime> =
            Arc::new(ManagedWorkflowAgentRuntime::new(manager));
        let durability = Arc::new(ScheduledDurability::new(
            repository,
            &schedule.id.to_string(),
            occurrence.id.as_str(),
            &occurrence.scheduled_for,
            occurrence.run_id.as_ref().map(ToString::to_string),
            &self.global_dir,
        ));
        let callbacks = WorkflowCallbackServices::new(
            durability.clone(),
            Arc::new(NonInteractiveHumanBroker),
            agents,
            Arc::new(WorkerLogSink),
        );
        let host = Arc::new(
            WorkflowHost::spawn(
                WorkflowHostConfig::new(workflow_host_entry()),
                Arc::new(callbacks.clone()),
            )
            .await
            .map_err(|error| error.to_string())?,
        );
        let result = self
            .run_authorized(
                schedule,
                occurrence,
                source,
                host.clone(),
                durability.clone(),
                callbacks,
            )
            .await;
        let shutdown = host.shutdown().await.map_err(|error| error.to_string());
        match (result, shutdown) {
            (Ok(result), Ok(())) => Ok(result),
            (Err(error), _) | (Ok(_), Err(error)) => Err(error),
        }
    }

    async fn run_authorized(
        &self,
        schedule: &ScheduleRecord,
        occurrence: &ScheduleOccurrence,
        source: ScheduledSource,
        host: Arc<WorkflowHost>,
        durability: Arc<ScheduledDurability>,
        callbacks: WorkflowCallbackServices,
    ) -> Result<WorkerExecutionResult, String> {
        let mut registry = WorkflowRegistry::new(
            vec![WorkflowRegistryRoot {
                directory: source.workflow_root,
                source: source.package_source,
            }],
            host.clone(),
            Some(schedule.agent_name.clone()),
            Some(vec![schedule.workflow_name.clone()]),
        );
        registry.load().await.map_err(|error| error.to_string())?;
        let workflow = registry
            .get(&schedule.workflow_name)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "The authorized workflow {}/{} could not be loaded.",
                    schedule.agent_name, schedule.workflow_name
                )
            })?;
        let runner = WorkflowRunner::new(host, durability.clone(), callbacks);
        let cancellation = CancellationToken::new();
        let execution = if let Some(run_id) = &occurrence.run_id {
            if durability.load_run(run_id.as_str()).await?.is_none() {
                durability
                    .create_run(
                        run_id.as_str(),
                        &workflow,
                        &schedule.project_dir,
                        &schedule.input,
                    )
                    .await?;
            }
            runner
                .resume(run_id.as_str(), &workflow, cancellation)
                .await
        } else {
            runner
                .run(
                    &workflow,
                    &schedule.project_dir,
                    schedule.input.clone(),
                    cancellation,
                )
                .await
        };
        let run_id = durability.run_id()?;
        match execution {
            Ok(value) => Ok(WorkerExecutionResult {
                run_id: Some(run_id),
                status: ScheduleOccurrenceStatus::Completed,
                result: Some(value),
                error: None,
            }),
            Err(error) => {
                let status = durability.load_run(&run_id).await?.map_or(
                    ScheduleOccurrenceStatus::Failed,
                    |run| match run.status {
                        DurableRunStatus::Waiting => ScheduleOccurrenceStatus::Waiting,
                        _ => ScheduleOccurrenceStatus::Failed,
                    },
                );
                Ok(WorkerExecutionResult {
                    run_id: Some(run_id),
                    status,
                    result: None,
                    error: Some(error.to_string()),
                })
            }
        }
    }
}

struct ScheduledSource {
    authorization_directory: PathBuf,
    workflow_root: PathBuf,
    package_source: PackageSource,
}

fn resolve_source(global_dir: &Path, schedule: &ScheduleRecord) -> Option<ScheduledSource> {
    let project_config_dir = schedule.project_dir.join(".work-agent");
    if schedule.agent_name == "main" {
        return [
            (
                project_config_dir
                    .join("workflows")
                    .join(&schedule.workflow_name),
                PackageSource::Project,
            ),
            (
                global_dir.join("workflows").join(&schedule.workflow_name),
                PackageSource::Global,
            ),
        ]
        .into_iter()
        .find(|(directory, _)| directory.is_dir())
        .map(
            |(authorization_directory, package_source)| ScheduledSource {
                workflow_root: authorization_directory
                    .parent()
                    .unwrap_or(&authorization_directory)
                    .to_path_buf(),
                authorization_directory,
                package_source,
            },
        );
    }
    [
        (
            project_config_dir.join("agents").join(&schedule.agent_name),
            PackageSource::Project,
        ),
        (
            global_dir.join("agents").join(&schedule.agent_name),
            PackageSource::Global,
        ),
    ]
    .into_iter()
    .find(|(directory, _)| directory.is_dir())
    .map(
        |(authorization_directory, package_source)| ScheduledSource {
            workflow_root: authorization_directory.join("workflows"),
            authorization_directory,
            package_source,
        },
    )
}

#[derive(Debug)]
struct ScheduledAuthorizationPolicy;

#[async_trait]
impl AuthorizationPolicy for ScheduledAuthorizationPolicy {
    async fn authorize(&self, request: PermissionRequest) -> AuthorizationDecision {
        if request.effect == flowmation_application::ToolEffect::Read
            && request.permission_mode == flowmation_application::ToolPermissionMode::Effect
        {
            AuthorizationDecision::Allow
        } else {
            AuthorizationDecision::Deny
        }
    }
}

struct NonInteractiveHumanBroker;

#[async_trait]
impl HumanRequestBroker for NonInteractiveHumanBroker {
    async fn request(
        &self,
        _run_id: &str,
        _prompt: &HumanCallback,
    ) -> Result<Option<Value>, String> {
        Ok(None)
    }
}

struct WorkerLogSink;

impl WorkflowLogSink for WorkerLogSink {
    fn log(&self, run_id: &str, message: &str, data: Option<&Value>) {
        if let Some(data) = data {
            eprintln!("[{run_id}] {message}: {data}");
        } else {
            eprintln!("[{run_id}] {message}");
        }
    }
}

fn workflow_host_entry() -> PathBuf {
    std::env::var_os("FLOWMATION_WORKFLOW_HOST").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../workflow-host/dist/index.js"),
        PathBuf::from,
    )
}
