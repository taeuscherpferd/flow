use std::collections::BTreeSet;
use std::fmt::{Debug, Formatter};
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use flowmation_application::{
    HumanRequestBroker, ScheduleRequest, ScheduleService, ScheduleToolRuntime,
    ScheduledWorkflowCatalog, WorkflowInspector, WorkflowRecord, WorkflowRegistry,
    WorkflowRegistryRoot, parse_schedule_request,
};
use flowmation_domain::agent::PackageSource;
use flowmation_domain::fingerprint::fingerprint_directory;
use flowmation_domain::schedule::ScheduleRecord;
use flowmation_domain::schema::{WorkflowSchema, validate_schema};
use flowmation_sqlite::SqliteApplicationRepository;
use flowmation_workflow_host::protocol::{HumanCallback, HumanRequestKind};
use serde_json::Value;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

pub struct CliScheduleRuntime {
    registry: Arc<Mutex<WorkflowRegistry>>,
    inspector: Arc<dyn WorkflowInspector>,
    repository: Arc<SqliteApplicationRepository>,
    global_dir: PathBuf,
    project_config_dir: PathBuf,
    project_dir: PathBuf,
    active_agent_name: String,
    permitted_agent_names: BTreeSet<String>,
    human: Arc<dyn HumanRequestBroker>,
}

pub struct CliScheduleContext {
    pub global_dir: PathBuf,
    pub project_config_dir: PathBuf,
    pub project_dir: PathBuf,
    pub active_agent_name: String,
    pub permitted_agent_names: Vec<String>,
}

impl CliScheduleRuntime {
    pub fn new(
        registry: Arc<Mutex<WorkflowRegistry>>,
        inspector: Arc<dyn WorkflowInspector>,
        repository: Arc<SqliteApplicationRepository>,
        context: CliScheduleContext,
        human: Arc<dyn HumanRequestBroker>,
    ) -> Self {
        Self {
            registry,
            inspector,
            repository,
            global_dir: context.global_dir,
            project_config_dir: context.project_config_dir,
            project_dir: context.project_dir,
            active_agent_name: context.active_agent_name,
            permitted_agent_names: context.permitted_agent_names.into_iter().collect(),
            human,
        }
    }

    pub async fn create_from_json(
        &self,
        raw: &str,
        cancellation: &CancellationToken,
    ) -> Result<ScheduleRecord, String> {
        let value = serde_json::from_str::<Value>(raw)
            .map_err(|error| format!("Schedule creation expects a JSON object: {error}"))?;
        let arguments = value
            .as_object()
            .cloned()
            .ok_or_else(|| "Schedule creation expects a JSON object.".to_owned())?;
        let request = parse_schedule_request(&arguments, &self.active_agent_name)?;
        ScheduleToolRuntime::create(self, request, cancellation).await
    }

    pub async fn available_workflows(&self) -> Result<Vec<WorkflowRecord>, String> {
        let mut workflows = Vec::new();
        for agent_name in &self.permitted_agent_names {
            if agent_name == "main" {
                let mut registry = self.registry.lock().await;
                registry.load().await.map_err(|error| error.to_string())?;
                workflows.extend(registry.list().into_iter().cloned().map(|mut workflow| {
                    workflow.agent_name = Some("main".to_owned());
                    workflow.resource_id = Some(format!("main/{}", workflow.metadata.name));
                    workflow
                }));
            } else {
                let (specialist_workflows, _fingerprint) =
                    self.load_specialist_workflow(agent_name, None).await?;
                workflows.extend(specialist_workflows);
            }
        }
        Ok(workflows)
    }

    async fn load_specialist_workflow(
        &self,
        agent_name: &str,
        requested: Option<&str>,
    ) -> Result<(Vec<WorkflowRecord>, String), String> {
        let (package_directory, source) = self.specialist_package(agent_name)?;
        let mut registry = WorkflowRegistry::new(
            vec![WorkflowRegistryRoot {
                directory: package_directory.join("workflows"),
                source,
            }],
            Arc::clone(&self.inspector),
            Some(agent_name.to_owned()),
            requested.map(|name| vec![name.to_owned()]),
        );
        registry.load().await.map_err(|error| error.to_string())?;
        let workflows = registry.list().into_iter().cloned().collect();
        let fingerprint = fingerprint_directory(&package_directory).map_err(|error| {
            format!(
                "Could not fingerprint agent package {}: {error}",
                package_directory.display()
            )
        })?;
        Ok((workflows, fingerprint))
    }

    fn specialist_package(&self, agent_name: &str) -> Result<(PathBuf, PackageSource), String> {
        let project = self.project_config_dir.join("agents").join(agent_name);
        if project.is_dir() {
            return Ok((project, PackageSource::Project));
        }
        let global = self.global_dir.join("agents").join(agent_name);
        if global.is_dir() {
            return Ok((global, PackageSource::Global));
        }
        Err(format!("Unknown agent \"{agent_name}\"."))
    }
}

impl Debug for CliScheduleRuntime {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CliScheduleRuntime")
            .field("project_dir", &self.project_dir)
            .field("active_agent_name", &self.active_agent_name)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl ScheduleToolRuntime for CliScheduleRuntime {
    async fn create(
        &self,
        request: ScheduleRequest,
        cancellation: &CancellationToken,
    ) -> Result<ScheduleRecord, String> {
        if !self.permitted_agent_names.contains(&request.agent_name) {
            return Err(format!(
                "Agent \"{}\" is not available for schedule creation from {}.",
                request.agent_name, self.active_agent_name
            ));
        }
        if cancellation.is_cancelled() {
            return Err("Schedule creation cancelled.".to_owned());
        }
        let (workflow, package_fingerprint) = if request.agent_name == "main" {
            let workflow = {
                let mut registry = self.registry.lock().await;
                registry.load().await.map_err(|error| error.to_string())?;
                registry.get(&request.workflow_name).cloned()
            }
            .ok_or_else(|| format!("Unknown workflow \"{}\".", request.workflow_name))?;
            let fingerprint = workflow.fingerprint.clone();
            (workflow, fingerprint)
        } else {
            let (mut workflows, fingerprint) = self
                .load_specialist_workflow(&request.agent_name, Some(&request.workflow_name))
                .await?;
            let workflow = workflows
                .pop()
                .ok_or_else(|| unknown_workflow(&request.agent_name, &request.workflow_name))?;
            (workflow, fingerprint)
        };
        let catalog = Arc::new(SingleWorkflowCatalog {
            workflow,
            package_fingerprint,
        });
        let service = ScheduleService::new(&self.project_dir, catalog, self.repository.clone());
        let details = service.preview_confirmation(&request)?;
        let response = self
            .human
            .request(
                "",
                &HumanCallback {
                    run_id: String::new(),
                    kind: HumanRequestKind::Approval,
                    prompt: "Create this workflow schedule?".to_owned(),
                    details: Some(details),
                    choices: None,
                },
            )
            .await?;
        if response != Some(Value::Bool(true)) {
            return Err("The user declined schedule creation.".to_owned());
        }
        if cancellation.is_cancelled() {
            return Err("Schedule creation cancelled.".to_owned());
        }
        service.create(&request)
    }
}

struct SingleWorkflowCatalog {
    workflow: WorkflowRecord,
    package_fingerprint: String,
}

impl ScheduledWorkflowCatalog for SingleWorkflowCatalog {
    fn resolve(&self, agent_name: &str, requested: &str) -> Result<WorkflowRecord, String> {
        if self.workflow.agent_name.as_deref().unwrap_or("main") == agent_name
            && requested == self.workflow.metadata.name
        {
            Ok(self.workflow.clone())
        } else {
            Err(unknown_workflow(agent_name, requested))
        }
    }

    fn validate_input(&self, workflow: &WorkflowRecord, input: &Value) -> Result<(), String> {
        let Some(schema) = &workflow.metadata.input_schema else {
            return if input.is_string() {
                Ok(())
            } else {
                Err(format!(
                    "Workflow \"{}\" expects string input.",
                    workflow.metadata.name
                ))
            };
        };
        let schema: WorkflowSchema =
            serde_json::from_value(schema.clone()).map_err(|error| error.to_string())?;
        let validation = validate_schema(&schema, input);
        if validation.valid {
            Ok(())
        } else {
            Err(validation.errors.join("\n"))
        }
    }

    fn package_fingerprint(&self, _agent_name: &str, _workflow: &WorkflowRecord) -> String {
        self.package_fingerprint.clone()
    }
}

fn unknown_workflow(agent_name: &str, workflow_name: &str) -> String {
    format!("Unknown workflow \"{agent_name}/{workflow_name}\".")
}

#[cfg(test)]
mod tests;
