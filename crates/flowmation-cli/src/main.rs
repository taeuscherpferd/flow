mod command;
mod line_editor;
mod model_setup;
#[cfg(test)]
mod permission_prompt;
mod provider_factory;
mod schedule_commands;
mod skill_usage;
mod spinner;
mod worker;
mod workflow_commands;
mod workflow_host_entry;

use std::collections::HashSet;
use std::fmt::{Debug, Formatter};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use clap::{Parser, Subcommand};
use flowmation_application::scheduling::ScheduleRepository;
use flowmation_application::{
    AgentManager, AuthorizationDecision, AuthorizationPolicy, ConfigService, CreateScheduleTool,
    AgentActivity, FixedPermissionBroker, HumanRequestBroker, ManagedWorkflowAgentRuntime,
    StandardAuthorizationPolicy, WorkflowAgentRuntime, WorkflowCallbackServices,
    WorkflowConfirmation, WorkflowDurability, WorkflowLogSink, WorkflowRegistry,
    WorkflowRegistryRoot, WorkflowRunner, WorkflowToolRuntime,
};
use flowmation_codex::{
    CodexAccountStatus, CodexModel, CodexProvider, OPENAI_SUBSCRIPTION_PROVIDER_NAME,
};
use flowmation_domain::agent::PackageSource;
use flowmation_domain::ids::ScheduleId;
use flowmation_domain::input_history::{InputHistory, InputHistoryStore};
use flowmation_domain::schedule::ScheduleKind;
use flowmation_sqlite::{
    SqliteApplicationRepository, WorkflowPresentation as StoredWorkflowPresentation,
    WorkflowRunDetails, WorkflowRunStatus,
};
use flowmation_workflow_host::protocol::{HumanCallback, HumanRequestKind};
use flowmation_workflow_host::{WorkflowHost, WorkflowHostConfig, WorkflowHostError};
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::command::{BUILTIN_COMMANDS, HELP_TEXT, ReplCommand, parse_repl_line};
use crate::model_setup::{ModelSetupIo, ModelSetupResult, ModelSetupService, format_openai_model};
use crate::provider_factory::create_model_providers;
use crate::schedule_commands::{CliScheduleContext, CliScheduleRuntime};
use crate::skill_usage::SkillUsageTracker;
use crate::spinner::Spinner;
use crate::workflow_commands::WorkflowRunScope;

const READY_TEXT: &str = "Ready. Type a message, or \"/help\" for commands.";
const WELCOME_TEXT: &str = "Welcome to flowmation. Before we can get started you will need to \
                            setup a provider and a model. Use /model to get started.";
const OPENAI_MODEL_PREFIX: &str = "openai/";
const OPENAI_COMPATIBLE_SETUP_NAME: &str = "openai-api";

#[derive(Debug, Parser)]
#[command(name = "flowmation", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<InternalCommand>,
}

#[derive(Debug, Subcommand)]
enum InternalCommand {
    #[command(hide = true)]
    Worker {
        #[arg(long)]
        once: bool,
        #[arg(long)]
        database: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let cli = Cli::parse();
    match cli.command {
        Some(InternalCommand::Worker { once, database }) => run_worker(once, database).await,
        None => run_repl().await,
    }
}

async fn run_worker(once: bool, database: Option<PathBuf>) -> Result<(), String> {
    let path = database.unwrap_or_else(default_database_path);
    worker::run(path, once).await
}

struct Runtime {
    config: flowmation_domain::config::ResolvedConfig,
    manager: Arc<Mutex<AgentManager>>,
    repository: Arc<SqliteApplicationRepository>,
    workflow_agents: Arc<dyn WorkflowAgentRuntime>,
    workflows: Option<WorkflowRuntime>,
    workflow_debug: Arc<AtomicBool>,
}

struct WorkflowRuntime {
    host: Arc<WorkflowHost>,
    registry: Arc<Mutex<WorkflowRegistry>>,
    runner: Arc<WorkflowRunner>,
}

struct CliWorkflowToolRuntime {
    registry: Arc<Mutex<WorkflowRegistry>>,
    runner: Arc<WorkflowRunner>,
    durability: Arc<dyn WorkflowDurability>,
    project_dir: PathBuf,
    agent_name: String,
}

impl Debug for CliWorkflowToolRuntime {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CliWorkflowToolRuntime")
            .field("project_dir", &self.project_dir)
            .field("agent_name", &self.agent_name)
            .finish_non_exhaustive()
    }
}

fn interactive_authorization() -> Arc<dyn AuthorizationPolicy> {
    Arc::new(StandardAuthorizationPolicy::new(Arc::new(
        FixedPermissionBroker::new(AuthorizationDecision::Allow),
    )))
}

impl Runtime {
    async fn create(config: flowmation_domain::config::ResolvedConfig) -> Result<Self, String> {
        let providers = create_model_providers(&config.models);
        let repository = Arc::new(
            SqliteApplicationRepository::open_global_dir(&config.global_dir)
                .map_err(|error| error.to_string())?,
        );
        let authorization = interactive_authorization();
        let manager = AgentManager::create(
            config.clone(),
            providers.clone(),
            authorization.clone(),
            None,
            repository.clone(),
        )
        .await
        .map_err(|error| error.to_string())?;
        let workflow_agent_manager = AgentManager::create(
            config.clone(),
            providers,
            authorization,
            None,
            repository.clone(),
        )
        .await
        .map_err(|error| error.to_string())?;
        Ok(Self {
            config,
            manager: Arc::new(Mutex::new(manager)),
            repository,
            workflow_agents: Arc::new(ManagedWorkflowAgentRuntime::new(Arc::new(Mutex::new(
                workflow_agent_manager,
            )))),
            workflows: None,
            workflow_debug: Arc::new(AtomicBool::new(false)),
        })
    }

    async fn ensure_workflows(&mut self) -> Result<&mut WorkflowRuntime, String> {
        if self.workflows.is_none() {
            let callbacks = WorkflowCallbackServices::new(
                self.repository.clone(),
                Arc::new(TerminalHumanBroker),
                Arc::clone(&self.workflow_agents),
                Arc::new(TerminalLogSink {
                    enabled: Arc::clone(&self.workflow_debug),
                }),
            );
            let host = Arc::new(
                WorkflowHost::spawn(
                    WorkflowHostConfig::new(workflow_host_entry::entry_path()?),
                    Arc::new(callbacks.clone()),
                )
                .await
                .map_err(host_error)?,
            );
            let mut registry = WorkflowRegistry::new(
                vec![
                    WorkflowRegistryRoot {
                        directory: self.config.global_dir.join("workflows"),
                        source: PackageSource::Global,
                    },
                    WorkflowRegistryRoot {
                        directory: self.config.project_dir.join("workflows"),
                        source: PackageSource::Project,
                    },
                ],
                host.clone(),
                None,
                None,
            );
            registry.load().await.map_err(|error| error.to_string())?;
            let registry = Arc::new(Mutex::new(registry));
            let runner = Arc::new(WorkflowRunner::new(
                host.clone(),
                self.repository.clone(),
                callbacks,
            ));
            self.workflows = Some(WorkflowRuntime {
                host,
                registry,
                runner,
            });
        }
        let (agent_name, permitted_agent_names) = self.schedule_agent_context().await;
        let workflows = self
            .workflows
            .as_ref()
            .ok_or_else(|| "workflow runtime did not initialize".to_owned())?;
        let records = workflows
            .registry
            .lock()
            .await
            .list()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        let tool_runtime: Arc<dyn WorkflowToolRuntime> = Arc::new(CliWorkflowToolRuntime {
            registry: Arc::clone(&workflows.registry),
            runner: Arc::clone(&workflows.runner),
            durability: self.repository.clone(),
            project_dir: self.project_root(),
            agent_name: agent_name.clone(),
        });
        let schedule_runtime = Arc::new(CliScheduleRuntime::new(
            Arc::clone(&workflows.registry),
            workflows.host.clone(),
            Arc::clone(&self.repository),
            CliScheduleContext {
                global_dir: self.config.global_dir.clone(),
                project_config_dir: self.config.project_dir.clone(),
                project_dir: self.project_root(),
                active_agent_name: agent_name.clone(),
                permitted_agent_names,
            },
            Arc::new(TerminalHumanBroker),
        ));
        let schedule_workflows = schedule_runtime.available_workflows().await?;
        let mut manager = self.manager.lock().await;
        manager.configure_workflows(&records, tool_runtime);
        manager.register_direct_tool(Arc::new(CreateScheduleTool::new(
            agent_name,
            &schedule_workflows,
            schedule_runtime,
        )));
        drop(manager);
        self.workflows
            .as_mut()
            .ok_or_else(|| "workflow runtime did not initialize".to_owned())
    }

    async fn run_scope(&self) -> Result<WorkflowRunScope, String> {
        WorkflowRunScope::new(
            &self.config.global_dir,
            &self.project_root(),
            self.manager.lock().await.active_name(),
        )
    }

    fn project_root(&self) -> PathBuf {
        self.config
            .project_dir
            .parent()
            .unwrap_or(&self.config.project_dir)
            .to_path_buf()
    }

    async fn schedule_runtime(&mut self) -> Result<Arc<CliScheduleRuntime>, String> {
        self.ensure_workflows().await?;
        let (agent_name, permitted_agent_names) = self.schedule_agent_context().await;
        let project_dir = self.project_root();
        let repository = Arc::clone(&self.repository);
        let workflows = self
            .workflows
            .as_ref()
            .ok_or_else(|| "workflow runtime did not initialize".to_owned())?;
        Ok(Arc::new(CliScheduleRuntime::new(
            Arc::clone(&workflows.registry),
            workflows.host.clone(),
            repository,
            CliScheduleContext {
                global_dir: self.config.global_dir.clone(),
                project_config_dir: self.config.project_dir.clone(),
                project_dir,
                active_agent_name: agent_name,
                permitted_agent_names,
            },
            Arc::new(TerminalHumanBroker),
        )))
    }

    async fn schedule_agent_context(&self) -> (String, Vec<String>) {
        let manager = self.manager.lock().await;
        let active = manager.active_name().to_owned();
        let permitted = if active == "main" {
            manager
                .list_agents()
                .into_iter()
                .map(|agent| agent.name)
                .collect()
        } else {
            vec![active.clone()]
        };
        (active, permitted)
    }

    async fn shutdown(&self) {
        if let Some(workflows) = &self.workflows {
            let _result = workflows.host.shutdown().await;
        }
    }
}

#[async_trait]
impl WorkflowToolRuntime for CliWorkflowToolRuntime {
    async fn resolve(&self, name: &str) -> Option<flowmation_application::WorkflowRecord> {
        let mut registry = self.registry.lock().await;
        if registry.load().await.is_err() {
            return None;
        }
        registry.get(name).cloned().map(|mut record| {
            record.agent_name = Some(self.agent_name.clone());
            record.resource_id = Some(format!("{}/{name}", self.agent_name));
            record
        })
    }

    async fn invoke(
        &self,
        record: &flowmation_application::WorkflowRecord,
        input: Value,
        cancellation: &CancellationToken,
    ) -> Result<String, String> {
        let run_id = Uuid::new_v4().to_string();
        self.durability
            .create_run(&run_id, record, &self.project_dir, &input)
            .await?;
        let value = self
            .runner
            .resume(&run_id, record, cancellation.child_token())
            .await
            .map_err(|error| error.to_string())?;
        serde_json::to_string(&json!({
            "runId": run_id,
            "workflow": record.metadata.name,
            "result": value,
        }))
        .map_err(|error| error.to_string())
    }

    async fn confirm(&self, confirmation: WorkflowConfirmation) -> bool {
        let response = TerminalHumanBroker
            .request(
                "",
                &HumanCallback {
                    run_id: String::new(),
                    kind: HumanRequestKind::Approval,
                    prompt: format!(
                        "Allow the agent to run workflow \"{}\"?",
                        confirmation.workflow_name
                    ),
                    details: Some(format!(
                        "{}\n\nInput:\n{}",
                        confirmation.description,
                        serde_json::to_string_pretty(&confirmation.input)
                            .unwrap_or_else(|_| confirmation.input.to_string())
                    )),
                    choices: None,
                },
            )
            .await;
        matches!(response, Ok(Some(Value::Bool(true))))
    }
}

async fn run_repl() -> Result<(), String> {
    let config_service = ConfigService::default();
    let mut config = config_service
        .load()
        .await
        .map_err(|error| error.to_string())?;
    let history_store = InputHistoryStore::new(&config.global_dir);
    let entries = history_store.load().unwrap_or_default();
    let mut history = InputHistory::new(entries, 500).map_err(|error| error.to_string())?;
    let mut runtime = if config.models.has_configured_default_model() {
        println!("{READY_TEXT}");
        Some(Runtime::create(config.clone()).await?)
    } else {
        println!("{WELCOME_TEXT}");
        None
    };

    loop {
        let (identity, commands) = repl_prompt_context(runtime.as_mut()).await?;
        let Some(answer) =
            read_repl_line(&format!("[{identity}] > "), history.snapshot(), commands).await?
        else {
            break;
        };
        let line = answer.trim();
        if line.is_empty() {
            continue;
        }
        history.record(line);
        if let Err(error) = history_store.append(line, history.limit()) {
            eprintln!("Warning: could not save input history: {error}");
        }
        match parse_repl_line(line) {
            ReplCommand::Exit => break,
            ReplCommand::Help => println!("{HELP_TEXT}"),
            ReplCommand::Empty => {}
            ReplCommand::Message(message) => {
                let Some(active) = runtime.as_mut() else {
                    println!("{WELCOME_TEXT}");
                    continue;
                };
                active.ensure_workflows().await?;
                let mut manager = active.manager.lock().await;
                let response = respond_to_user(&mut manager, &message).await;
                match response {
                    Ok(response) => println!("{response}"),
                    Err(error) => eprintln!("{error}"),
                }
            }
            ReplCommand::Agent(name) => {
                let Some(active) = runtime.as_mut() else {
                    println!("{WELCOME_TEXT}");
                    continue;
                };
                let mut manager = active.manager.lock().await;
                if let Some(name) = name {
                    match manager.switch_agent(&name) {
                        Ok(true) => println!("Switched to agent \"{name}\"."),
                        Ok(false) => println!("Already using agent \"{name}\"."),
                        Err(error) => println!("{error}"),
                    }
                } else {
                    for agent in manager.list_agents() {
                        println!(
                            "  {}{} — {} [{:?}]",
                            agent.name,
                            if agent.active { " (active)" } else { "" },
                            agent.description,
                            agent.source
                        );
                    }
                }
            }
            ReplCommand::Clear => {
                if let Some(active) = runtime.as_mut() {
                    let mut manager = active.manager.lock().await;
                    manager
                        .clear_active_history()
                        .map_err(|error| error.to_string())?;
                    println!("Cleared the {} conversation.", manager.active_name());
                } else {
                    println!("{WELCOME_TEXT}");
                }
            }
            ReplCommand::Model(requested) => {
                let requested_openai_model = requested
                    .as_deref()
                    .and_then(|value| value.strip_prefix(OPENAI_MODEL_PREFIX));
                let configure_openai = requested.as_deref()
                    == Some(OPENAI_SUBSCRIPTION_PROVIDER_NAME)
                    || requested_openai_model.is_some_and(|model| {
                        config
                            .models
                            .resolve_model(&format!("{OPENAI_MODEL_PREFIX}{model}"))
                            .is_err()
                    });
                let setup_result = if configure_openai {
                    Some(setup_openai_model(&config_service, requested_openai_model).await?)
                } else if requested.as_deref() == Some(OPENAI_COMPATIBLE_SETUP_NAME) {
                    Some(setup_openai_compatible_model(&config_service).await?)
                } else {
                    None
                };
                if let Some(setup_result) = setup_result {
                    let active_agent = if let Some(active) = runtime.as_ref() {
                        Some(active.manager.lock().await.active_name().to_owned())
                    } else {
                        None
                    };
                    if let ModelSetupResult::Completed {
                        provider, model, ..
                    } = setup_result
                    {
                        config = config_service
                            .load()
                            .await
                            .map_err(|error| error.to_string())?;
                        let configured_runtime = Runtime::create(config.clone()).await?;
                        let reference = format!("{provider}/{model}");
                        let mut manager = configured_runtime.manager.lock().await;
                        if let Some(active_agent) = active_agent.as_deref() {
                            manager
                                .switch_agent(active_agent)
                                .map_err(|error| error.to_string())?;
                        }
                        manager
                            .set_model(&reference)
                            .map_err(|error| error.to_string())?;
                        drop(manager);
                        runtime = Some(configured_runtime);
                        println!("Added and switched to {reference}.");
                    }
                } else if let Some(active) = runtime.as_mut() {
                    let mut manager = active.manager.lock().await;
                    if let Some(requested) = requested {
                        match manager.set_model(&requested) {
                            Ok(true) => println!("Switched to {requested}."),
                            Ok(false) => println!("Already using {requested}."),
                            Err(error) => println!("{error}"),
                        }
                    } else {
                        let (provider, model) = manager.current_model();
                        let current = format!("{provider}/{model}");
                        let references = config.models.list_model_references();
                        let configured_openai = references
                            .iter()
                            .filter(|reference| {
                                reference.provider == OPENAI_SUBSCRIPTION_PROVIDER_NAME
                            })
                            .map(|reference| reference.model.clone())
                            .collect::<HashSet<_>>();
                        println!("Configured models:");
                        for reference in references {
                            let name = format!("{}/{}", reference.provider, reference.model);
                            let current_marker = if name == current { " (current)" } else { "" };
                            println!("  {name}{current_marker}");
                        }
                        drop(manager);
                        println!("Use /model <provider/model> to switch.");
                        print_openai_models(&configured_openai).await;
                    }
                } else if setup_first_model(&config_service).await? {
                    config = config_service
                        .load()
                        .await
                        .map_err(|error| error.to_string())?;
                    runtime = Some(Runtime::create(config.clone()).await?);
                    println!("{READY_TEXT}");
                }
            }
            ReplCommand::Workflows => {
                let Some(active) = runtime.as_mut() else {
                    println!("No workflows discovered.");
                    continue;
                };
                let workflows = active.ensure_workflows().await?;
                let registry = workflows.registry.lock().await;
                if registry.list().is_empty() {
                    println!("No workflows discovered.");
                } else {
                    println!("Workflows:");
                    for workflow in registry.list() {
                        println!(
                            "  {} ({:?}, agent: {:?}) — {}",
                            workflow.metadata.name,
                            workflow.source,
                            workflow.metadata.agent_invocation,
                            workflow.metadata.description
                        );
                    }
                }
            }
            ReplCommand::Workflow(command) | ReplCommand::Dynamic(command) => {
                let Some(active) = runtime.as_mut() else {
                    println!("{WELCOME_TEXT}");
                    continue;
                };
                if !run_workflow_command(active, &command).await? {
                    let (name, remainder) = split_command(&command);
                    let mut manager = active.manager.lock().await;
                    if manager.load_skill(name) {
                        println!("Loaded skill: {name}");
                        if !remainder.is_empty() {
                            let response = respond_to_user(&mut manager, remainder).await?;
                            println!("{response}");
                        }
                    } else {
                        println!("Unknown command, workflow, or skill: /{name}");
                    }
                }
            }
            ReplCommand::Runs => {
                let Some(active) = runtime.as_ref() else {
                    println!("{WELCOME_TEXT}");
                    continue;
                };
                show_runs(active).await?;
            }
            ReplCommand::WorkflowDebug(value) => {
                let Some(active) = runtime.as_ref() else {
                    println!("{WELCOME_TEXT}");
                    continue;
                };
                set_workflow_debug(active, value.as_deref());
            }
            ReplCommand::Run(id) => {
                let Some(active) = runtime.as_mut() else {
                    println!("{WELCOME_TEXT}");
                    continue;
                };
                inspect_run(active, &id).await?;
            }
            ReplCommand::Resume(id) => {
                let Some(active) = runtime.as_mut() else {
                    println!("{WELCOME_TEXT}");
                    continue;
                };
                resume_run(active, &id).await?;
            }
            ReplCommand::Cancel(id) => {
                let Some(active) = runtime.as_ref() else {
                    println!("{WELCOME_TEXT}");
                    continue;
                };
                cancel_run(active, &id).await?;
            }
            ReplCommand::Schedules => {
                let Some(active) = runtime.as_ref() else {
                    println!("{WELCOME_TEXT}");
                    continue;
                };
                let project_dir = active
                    .config
                    .project_dir
                    .parent()
                    .unwrap_or(&active.config.project_dir);
                for schedule in ScheduleRepository::list(active.repository.as_ref(), project_dir)? {
                    println!(
                        "  {} — {}/{} {} [{:?}]",
                        schedule.id,
                        schedule.agent_name,
                        schedule.workflow_name,
                        schedule_timing(&schedule),
                        schedule.status
                    );
                }
            }
            ReplCommand::Schedule(command) => {
                let Some(active) = runtime.as_mut() else {
                    println!("{WELCOME_TEXT}");
                    continue;
                };
                handle_schedule_command(active, &command).await?;
            }
        }
    }
    if let Some(runtime) = runtime {
        runtime.shutdown().await;
    }
    Ok(())
}

async fn repl_prompt_context(
    runtime: Option<&mut Runtime>,
) -> Result<(String, Vec<String>), String> {
    let mut commands = BUILTIN_COMMANDS
        .iter()
        .map(|command| (*command).to_owned())
        .collect::<Vec<_>>();
    let Some(runtime) = runtime else {
        return Ok(("main".to_owned(), commands));
    };
    let workflow_names = {
        let workflows = runtime.ensure_workflows().await?;
        workflows
            .registry
            .lock()
            .await
            .list()
            .into_iter()
            .map(|workflow| workflow.metadata.name.clone())
            .collect::<Vec<_>>()
    };
    let manager = runtime.manager.lock().await;
    let identity = manager.active_name().to_owned();
    commands.extend(workflow_names);
    commands.extend(manager.list_skill_names());
    Ok((identity, commands))
}

async fn respond_to_user(manager: &mut AgentManager, message: &str) -> Result<String, String> {
    let mut activities = manager.subscribe_activity();
    let mut spinner = Spinner::start("");
    let mut skill_usage = SkillUsageTracker::default();
    let cancellation = CancellationToken::new();
    let response = manager.handle_user_message(message, &cancellation);
    tokio::pin!(response);
    let mut listening = true;
    let response = loop {
        tokio::select! {
            biased;
            activity = activities.recv(), if listening => {
                match activity {
                    Ok(activity) => update_skill_usage(&mut skill_usage, &mut spinner, activity),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_count)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => listening = false,
                }
            }
            result = &mut response => break result.map_err(|error| error.to_string()),
        }
    };
    while let Ok(activity) = activities.try_recv() {
        update_skill_usage(&mut skill_usage, &mut spinner, activity);
    }
    spinner.stop().await;
    if let Some(summary) = skill_usage.summary() {
        println!("{summary}");
    }
    response
}

fn update_skill_usage(
    skill_usage: &mut SkillUsageTracker,
    spinner: &mut Spinner,
    activity: AgentActivity,
) {
    if let Some(label) = skill_usage.observe(activity) {
        spinner.set_label(&label);
    }
}

async fn run_workflow_command(runtime: &mut Runtime, command: &str) -> Result<bool, String> {
    let (name, raw_input) = split_command(command);
    let project_dir = runtime.project_root();
    let agent_name = runtime.manager.lock().await.active_name().to_owned();
    let repository = Arc::clone(&runtime.repository);
    let workflows = runtime.ensure_workflows().await?;
    let (record, input) = {
        let registry = workflows.registry.lock().await;
        let Some(mut record) = registry.get(name).cloned() else {
            return Ok(false);
        };
        record.agent_name = Some(agent_name.clone());
        record.resource_id = Some(format!("{agent_name}/{name}"));
        let input = registry.parse_input(&record, raw_input)?;
        (record, input)
    };
    let run_id = Uuid::new_v4().to_string();
    repository
        .create_run(&run_id, &record, &project_dir, &input)
        .await?;
    let result = workflows
        .runner
        .resume(&run_id, &record, CancellationToken::new())
        .await;
    let value = match result {
        Ok(value) => value,
        Err(error) => {
            println!(
                "Workflow \"{}\" failed ({run_id}): {error}",
                record.metadata.name
            );
            return Ok(true);
        }
    };
    let agent_presentation = runtime
        .run_scope()
        .await?
        .get(&run_id)?
        .is_some_and(|run| run.summary.presentation == StoredWorkflowPresentation::Agent);
    display_workflow_value(runtime, &record.metadata.name, agent_presentation, &value).await?;
    Ok(true)
}

async fn show_runs(runtime: &Runtime) -> Result<(), String> {
    let runs = runtime.run_scope().await?.list(20)?;
    if runs.is_empty() {
        println!("No workflow runs.");
        return Ok(());
    }
    for run in runs {
        println!(
            "{}  {:16}  {}/{}  {}",
            run.id,
            run.status.as_str(),
            run.agent_name,
            run.workflow_name,
            run.updated_at
        );
    }
    Ok(())
}

async fn inspect_run(runtime: &mut Runtime, run_id: &str) -> Result<(), String> {
    if run_id.is_empty() {
        println!("Usage: /run <id>");
        return Ok(());
    }
    let Some(run) = runtime.run_scope().await?.get(run_id)? else {
        println!("No workflow run found with id \"{run_id}\".");
        return Ok(());
    };
    println!(
        "{}  {}  {}/{}  {}",
        run.summary.id,
        run.summary.status.as_str(),
        run.summary.agent_name,
        run.summary.workflow_name,
        run.summary.updated_at
    );
    println!("Input:\n{}", pretty_value(&run.input)?);
    display_stored_run(runtime, &run).await
}

async fn resume_run(runtime: &mut Runtime, run_id: &str) -> Result<(), String> {
    if run_id.is_empty() {
        println!("Usage: /resume <id>");
        return Ok(());
    }
    let Some(run) = runtime.run_scope().await?.get(run_id)? else {
        println!("No workflow run found with id \"{run_id}\".");
        return Ok(());
    };
    let presentation = run.summary.presentation;
    let workflow_name = run.summary.workflow_name.clone();
    let workflows = runtime.ensure_workflows().await?;
    let record = {
        let registry = workflows.registry.lock().await;
        registry.get(&workflow_name).cloned()
    };
    let Some(record) = record else {
        println!("Workflow \"{workflow_name}\" is no longer available.");
        return Ok(());
    };
    let result = workflows
        .runner
        .resume(run_id, &record, CancellationToken::new())
        .await;
    match result {
        Ok(value) => {
            display_workflow_value(
                runtime,
                &workflow_name,
                presentation == StoredWorkflowPresentation::Agent,
                &value,
            )
            .await
        }
        Err(error) => {
            println!("{error}");
            Ok(())
        }
    }
}

async fn cancel_run(runtime: &Runtime, run_id: &str) -> Result<(), String> {
    if run_id.is_empty() {
        println!("Usage: /cancel <id>");
        return Ok(());
    }
    match runtime.run_scope().await?.cancel(run_id) {
        Ok(Some(run)) => println!(
            "Workflow \"{}\" is {}.",
            run.summary.workflow_name,
            run.summary.status.as_str()
        ),
        Ok(None) => println!("No workflow run found with id \"{run_id}\"."),
        Err(error) => println!("{error}"),
    }
    Ok(())
}

fn set_workflow_debug(runtime: &Runtime, request: Option<&str>) {
    match request {
        Some("on") => runtime.workflow_debug.store(true, Ordering::Relaxed),
        Some("off") => runtime.workflow_debug.store(false, Ordering::Relaxed),
        Some(_) => {
            println!("Usage: /workflow-debug [on|off]");
            return;
        }
        None => {}
    }
    let enabled = runtime.workflow_debug.load(Ordering::Relaxed);
    println!(
        "Workflow debug logging is {}.",
        if enabled { "on" } else { "off" }
    );
}

async fn display_stored_run(runtime: &Runtime, run: &WorkflowRunDetails) -> Result<(), String> {
    if run.summary.status != WorkflowRunStatus::Completed {
        if let Some(error) = &run.summary.error {
            println!(
                "Workflow \"{}\" is {} ({}): {error}",
                run.summary.workflow_name,
                run.summary.status.as_str(),
                run.summary.id
            );
        }
        return Ok(());
    }
    let Some(output) = &run.output else {
        println!(
            "Workflow \"{}\" completed without a stored output ({}).",
            run.summary.workflow_name, run.summary.id
        );
        return Ok(());
    };
    display_workflow_value(
        runtime,
        &run.summary.workflow_name,
        run.summary.presentation == StoredWorkflowPresentation::Agent,
        output,
    )
    .await
}

async fn display_workflow_value(
    runtime: &Runtime,
    workflow_name: &str,
    agent_presentation: bool,
    value: &Value,
) -> Result<(), String> {
    if agent_presentation {
        let response = runtime
            .manager
            .lock()
            .await
            .present_workflow_result(workflow_name, value, &CancellationToken::new())
            .await
            .map_err(|error| error.to_string())?;
        println!("{response}");
    } else if let Some(value) = value.as_str() {
        println!("{value}");
    } else {
        println!("{}", pretty_value(value)?);
    }
    Ok(())
}

fn pretty_value(value: &Value) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(|error| error.to_string())
}

async fn handle_schedule_command(runtime: &mut Runtime, command: &str) -> Result<(), String> {
    if let Some(request) = command.strip_prefix("create ") {
        let schedule = runtime
            .schedule_runtime()
            .await?
            .create_from_json(request, &CancellationToken::new())
            .await?;
        println!(
            "Created schedule {} — {}.",
            schedule.id,
            schedule_timing(&schedule)
        );
        return Ok(());
    }
    let parts = command.split_whitespace().collect::<Vec<_>>();
    match parts.as_slice() {
        [id] => {
            let id = ScheduleId::new(*id).map_err(|error| error.to_string())?;
            if let Some(schedule) = ScheduleRepository::get(runtime.repository.as_ref(), &id)? {
                println!("{schedule:#?}");
            } else {
                println!("Unknown schedule \"{id}\".");
            }
        }
        [operation @ ("pause" | "resume"), id] => {
            let id = ScheduleId::new(*id).map_err(|error| error.to_string())?;
            let status = if *operation == "pause" {
                flowmation_domain::schedule::ScheduleStatus::Paused
            } else {
                flowmation_domain::schedule::ScheduleStatus::Active
            };
            let Some(schedule) = ScheduleRepository::get(runtime.repository.as_ref(), &id)? else {
                println!("Unknown schedule \"{id}\".");
                return Ok(());
            };
            if !schedule.status.can_transition_to(status) {
                println!(
                    "Schedule \"{id}\" cannot transition from {:?} to {:?}.",
                    schedule.status, status
                );
                return Ok(());
            }
            ScheduleRepository::set_status(runtime.repository.as_ref(), &id, status)?;
        }
        ["delete", id] => {
            let id = ScheduleId::new(*id).map_err(|error| error.to_string())?;
            if !ScheduleRepository::delete(runtime.repository.as_ref(), &id)? {
                println!("Unknown schedule \"{id}\".");
            }
        }
        _ => println!("Usage: /schedule create <json> | <id> | pause|resume|delete <id>"),
    }
    Ok(())
}

fn schedule_timing(schedule: &flowmation_domain::schedule::ScheduleRecord) -> String {
    match schedule.kind {
        ScheduleKind::Cron => format!("cron {} ({})", schedule.cron, schedule.timezone),
        ScheduleKind::Once => format!("once at {}", schedule.next_run_at),
    }
}

async fn setup_first_model(service: &ConfigService) -> Result<bool, String> {
    match ModelSetupService::new(service, &TerminalSetupIo)
        .run()
        .await?
    {
        ModelSetupResult::Completed {
            config_path,
            provider,
            model,
        } => {
            println!(
                "Configured \"{provider}/{model}\" in {}.",
                config_path.display()
            );
            Ok(true)
        }
        ModelSetupResult::Cancelled => Ok(false),
    }
}

async fn setup_openai_model(
    service: &ConfigService,
    requested_model: Option<&str>,
) -> Result<ModelSetupResult, String> {
    ModelSetupService::new(service, &TerminalSetupIo)
        .run_openai(requested_model)
        .await
}

async fn setup_openai_compatible_model(
    service: &ConfigService,
) -> Result<ModelSetupResult, String> {
    ModelSetupService::new(service, &TerminalSetupIo)
        .run_openai_compatible()
        .await
}

async fn print_openai_models(configured_openai: &HashSet<String>) {
    let spinner = Spinner::start("Checking OpenAI models");
    let provider = CodexProvider::default();
    let catalog = provider.model_catalog().await;
    spinner.stop().await;
    let (account, models) = match catalog {
        Ok(catalog) => catalog,
        Err(error) => {
            println!("OpenAI models unavailable: {error}");
            return;
        }
    };
    if !account.uses_chatgpt_subscription() {
        if let Some(account_type) = account.account_type.as_deref() {
            println!(
                "OpenAI models require ChatGPT sign-in; Codex currently uses \"{account_type}\". \
Run /model openai to switch."
            );
        } else {
            println!("OpenAI models require ChatGPT sign-in. Run /model openai to sign in.");
        }
        return;
    }
    let models = models
        .into_iter()
        .filter(|model| !configured_openai.contains(&model.id))
        .collect::<Vec<_>>();
    if models.is_empty() {
        println!("No additional OpenAI models are available.");
        return;
    }
    println!("OpenAI models available to add:");
    for model in models {
        println!("{}", format_openai_model(&model));
    }
    println!("Use /model openai/<name> to add and switch.");
}

async fn read_codex_account(provider: &CodexProvider) -> Result<CodexAccountStatus, String> {
    let spinner = Spinner::start("Checking ChatGPT sign-in");
    let result = provider.account_status().await;
    spinner.stop().await;
    result.map_err(|error| error.to_string())
}

async fn read_codex_models(provider: &CodexProvider) -> Result<Vec<CodexModel>, String> {
    let spinner = Spinner::start("Loading OpenAI models");
    let result = provider.list_models().await;
    spinner.stop().await;
    result.map_err(|error| error.to_string())
}

fn split_command(command: &str) -> (&str, &str) {
    command
        .split_once(char::is_whitespace)
        .map_or((command, ""), |(name, remainder)| (name, remainder.trim()))
}

async fn read_line(prompt: &str) -> Result<Option<String>, String> {
    let prompt = prompt.to_owned();
    tokio::task::spawn_blocking(move || {
        line_editor::read_line(&prompt, Vec::new(), Vec::new(), false)
    })
    .await
    .map_err(|error| format!("input task failed: {error}"))?
}

async fn read_repl_line(
    prompt: &str,
    history: Vec<String>,
    commands: Vec<String>,
) -> Result<Option<String>, String> {
    let prompt = prompt.to_owned();
    tokio::task::spawn_blocking(move || line_editor::read_line(&prompt, history, commands, false))
        .await
        .map_err(|error| format!("input task failed: {error}"))?
}

struct TerminalSetupIo;

#[async_trait]
impl ModelSetupIo for TerminalSetupIo {
    async fn prompt(&self, prompt: &str) -> Result<Option<String>, String> {
        read_line(prompt).await
    }

    async fn authenticate_openai(&self) -> Result<(), String> {
        let provider = CodexProvider::default();
        let account = read_codex_account(&provider).await?;
        if account.uses_chatgpt_subscription() {
            return Ok(());
        }
        if let Some(account_type) = account.account_type.as_deref() {
            println!(
                "Codex currently uses \"{account_type}\" authentication. OpenAI models in \
Flowmation use ChatGPT sign-in."
            );
            println!("Completing this sign-in will replace Codex's current authentication.");
        } else {
            println!("Sign in to ChatGPT to use OpenAI models.");
        }
        provider
            .login_with_device_code(|login| {
                println!("Open {}", login.verification_url);
                println!("Enter the one-time code: {}", login.user_code);
                println!("Waiting for Codex sign-in to complete...");
            })
            .await
            .map_err(|error| error.to_string())?;
        let account = read_codex_account(&provider).await?;
        if account.uses_chatgpt_subscription() {
            Ok(())
        } else {
            Err("Codex sign-in completed without ChatGPT subscription authentication.".to_owned())
        }
    }

    async fn discover_openai_models(&self) -> Result<Vec<CodexModel>, String> {
        read_codex_models(&CodexProvider::default()).await
    }

    fn output(&self, message: &str) {
        println!("{message}");
    }
}

#[derive(Debug)]
struct TerminalHumanBroker;

#[async_trait]
impl HumanRequestBroker for TerminalHumanBroker {
    async fn request(
        &self,
        _run_id: &str,
        prompt: &HumanCallback,
    ) -> Result<Option<Value>, String> {
        if let Some(details) = &prompt.details {
            println!("{details}");
        }
        match prompt.kind {
            HumanRequestKind::Approval => {
                let answer = read_line(&format!("{} [y/N] ", prompt.prompt)).await?;
                Ok(Some(Value::Bool(answer.is_some_and(|answer| {
                    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
                }))))
            }
            HumanRequestKind::Choice => request_choice(prompt).await,
            HumanRequestKind::Text => {
                let Some(answer) = read_line(&format!("{} ", prompt.prompt)).await? else {
                    return Ok(None);
                };
                Ok(Some(Value::String(answer)))
            }
        }
    }
}

async fn request_choice(prompt: &HumanCallback) -> Result<Option<Value>, String> {
    let choices = prompt.choices.as_deref().unwrap_or_default();
    for (index, choice) in choices.iter().enumerate() {
        println!("  {}. {} ({})", index + 1, choice.label, choice.value);
        if let Some(description) = &choice.description {
            println!("     {description}");
        }
    }
    loop {
        let Some(answer) = read_line(&format!("{} ", prompt.prompt)).await? else {
            return Ok(None);
        };
        let answer = answer.trim();
        let by_number = answer
            .parse::<usize>()
            .ok()
            .and_then(|index| index.checked_sub(1))
            .and_then(|index| choices.get(index));
        let by_value = choices.iter().find(|choice| choice.value == answer);
        if let Some(choice) = by_number.or(by_value) {
            return Ok(Some(Value::String(choice.value.clone())));
        }
        println!("Choose one of the listed numbers or values.");
    }
}

struct TerminalLogSink {
    enabled: Arc<AtomicBool>,
}

impl WorkflowLogSink for TerminalLogSink {
    fn log(&self, run_id: &str, message: &str, data: Option<&Value>) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        if let Some(data) = data {
            eprintln!("[workflow:{run_id}] {message}: {data}");
        } else {
            eprintln!("[workflow:{run_id}] {message}");
        }
    }
}

fn host_error(error: WorkflowHostError) -> String {
    format!("Could not start JavaScript workflow host: {error}")
}

fn default_database_path() -> PathBuf {
    std::env::var_os("HOME").map_or_else(
        || PathBuf::from(".work-agent/runs.sqlite"),
        |home| PathBuf::from(home).join(".work-agent/runs.sqlite"),
    )
}

#[cfg(test)]
mod tests {
    use flowmation_application::{
        AuthorizationDecision, ExecutionMode, PermissionRequest, ToolEffect, ToolPermissionMode,
    };
    use serde_json::Map;

    use super::interactive_authorization;

    #[tokio::test]
    async fn interactive_authorization_allows_effectful_tools_without_prompting() {
        let authorization = interactive_authorization();
        for effect in [
            ToolEffect::Write,
            ToolEffect::Command,
            ToolEffect::External,
            ToolEffect::Schedule,
        ] {
            assert_eq!(
                authorization
                    .authorize(PermissionRequest {
                        tool_name: "effectful-tool".to_owned(),
                        arguments: Map::new(),
                        effect,
                        permission_mode: ToolPermissionMode::Effect,
                        execution_mode: ExecutionMode::Direct,
                    })
                    .await,
                AuthorizationDecision::Allow
            );
        }
    }
}
