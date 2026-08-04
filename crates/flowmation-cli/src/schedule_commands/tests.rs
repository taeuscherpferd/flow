use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use flowmation_application::{
    HumanRequestBroker, ScheduleRequest, ScheduleTiming, ScheduleToolRuntime, WorkflowInspector,
    WorkflowRegistry,
};
use flowmation_domain::fingerprint::fingerprint_directory;
use flowmation_sqlite::SqliteApplicationRepository;
use flowmation_workflow_host::protocol::{
    AgentInvocationPolicy, HumanCallback, WorkflowMetadata, WorkflowPresentation,
};
use serde_json::Value;
use tempfile::tempdir;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use super::{CliScheduleContext, CliScheduleRuntime};

struct ApprovingHumanBroker;

#[async_trait]
impl HumanRequestBroker for ApprovingHumanBroker {
    async fn request(
        &self,
        _workflow_name: &str,
        _callback: &HumanCallback,
    ) -> Result<Option<Value>, String> {
        Ok(Some(Value::Bool(true)))
    }
}

struct PathInspector;

#[async_trait]
impl WorkflowInspector for PathInspector {
    async fn inspect(&self, entry_path: &Path) -> Result<WorkflowMetadata, String> {
        let name = entry_path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .ok_or_else(|| "Workflow entry has no parent directory.".to_owned())?;
        Ok(WorkflowMetadata {
            name: name.to_owned(),
            description: "Test workflow".to_owned(),
            input_schema: None,
            agent_invocation: AgentInvocationPolicy::Disabled,
            presentation: WorkflowPresentation::Direct,
        })
    }
}

#[tokio::test]
async fn coordinator_creates_specialist_schedule_with_full_package_fingerprint()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let global_dir = root.path().join("global");
    let project_config_dir = root.path().join("project/.work-agent");
    let package_directory = project_config_dir.join("agents/finance");
    let workflow_directory = package_directory.join("workflows/report");
    tokio::fs::create_dir_all(&workflow_directory).await?;
    tokio::fs::write(package_directory.join("AGENT.yaml"), "name: finance").await?;
    tokio::fs::write(package_directory.join("SOUL.md"), "Finance specialist").await?;
    tokio::fs::write(workflow_directory.join("WORKFLOW.js"), "export default {};").await?;

    let inspector: Arc<dyn WorkflowInspector> = Arc::new(PathInspector);
    let registry = Arc::new(Mutex::new(WorkflowRegistry::new(
        Vec::new(),
        Arc::clone(&inspector),
        None,
        None,
    )));
    let repository = Arc::new(SqliteApplicationRepository::open_global_dir(
        root.path().join("database"),
    )?);
    let runtime = CliScheduleRuntime::new(
        registry,
        inspector,
        repository,
        CliScheduleContext {
            global_dir,
            project_config_dir,
            project_dir: root.path().join("project"),
            active_agent_name: "main".to_owned(),
            permitted_agent_names: vec!["main".to_owned(), "finance".to_owned()],
        },
        Arc::new(ApprovingHumanBroker),
    );

    let workflows = runtime.available_workflows().await?;
    assert!(workflows.iter().any(|workflow| {
        workflow.agent_name.as_deref() == Some("finance") && workflow.metadata.name == "report"
    }));
    let schedule = runtime
        .create(
            ScheduleRequest {
                agent_name: "finance".to_owned(),
                workflow_name: "report".to_owned(),
                input: Value::String("quarterly".to_owned()),
                timing: ScheduleTiming::Once {
                    run_at: "2030-01-01T00:00:00Z".parse::<DateTime<Utc>>()?,
                },
                now: Some("2026-08-04T00:00:00Z".parse::<DateTime<Utc>>()?),
            },
            &CancellationToken::new(),
        )
        .await?;

    assert_eq!(schedule.agent_name, "finance");
    assert_eq!(
        schedule.package_fingerprint,
        fingerprint_directory(&package_directory)?
    );
    assert_ne!(
        schedule.package_fingerprint,
        workflows
            .iter()
            .find(|workflow| workflow.metadata.name == "report")
            .ok_or("Specialist workflow was not discovered.")?
            .fingerprint
    );
    Ok(())
}

#[tokio::test]
async fn specialist_cannot_schedule_another_agent() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let inspector: Arc<dyn WorkflowInspector> = Arc::new(PathInspector);
    let runtime = CliScheduleRuntime::new(
        Arc::new(Mutex::new(WorkflowRegistry::new(
            Vec::new(),
            Arc::clone(&inspector),
            None,
            None,
        ))),
        inspector,
        Arc::new(SqliteApplicationRepository::open_global_dir(
            root.path().join("database"),
        )?),
        CliScheduleContext {
            global_dir: root.path().join("global"),
            project_config_dir: root.path().join("project/.work-agent"),
            project_dir: root.path().join("project"),
            active_agent_name: "finance".to_owned(),
            permitted_agent_names: vec!["finance".to_owned()],
        },
        Arc::new(ApprovingHumanBroker),
    );

    let result = runtime
        .create(
            ScheduleRequest {
                agent_name: "main".to_owned(),
                workflow_name: "report".to_owned(),
                input: Value::String(String::new()),
                timing: ScheduleTiming::Once {
                    run_at: "2030-01-01T00:00:00Z".parse::<DateTime<Utc>>()?,
                },
                now: Some("2026-08-04T00:00:00Z".parse::<DateTime<Utc>>()?),
            },
            &CancellationToken::new(),
        )
        .await;

    assert!(result.is_err_and(|error| error.contains("not available")));
    Ok(())
}
