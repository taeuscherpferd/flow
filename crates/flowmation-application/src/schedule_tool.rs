use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use flowmation_domain::chat::{JsonSchema, JsonSchemaProperty, JsonSchemaType, JsonValueType};
use flowmation_domain::schedule::ScheduleRecord;
use flowmation_domain::tool::{ToolEffect, ToolPermissionMode, ToolResult};
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use crate::scheduling::{ScheduleRequest, ScheduleTiming};
use crate::tool::{Tool, ToolExecutionContext};
use crate::workflow::WorkflowRecord;

#[async_trait]
pub trait ScheduleToolRuntime: Debug + Send + Sync {
    async fn create(
        &self,
        request: ScheduleRequest,
        cancellation: &CancellationToken,
    ) -> Result<ScheduleRecord, String>;
}

#[derive(Debug)]
pub struct CreateScheduleTool {
    active_agent_name: String,
    agent_names: Vec<String>,
    workflow_names: Vec<String>,
    runtime: Arc<dyn ScheduleToolRuntime>,
}

impl CreateScheduleTool {
    #[must_use]
    pub fn new(
        active_agent_name: impl Into<String>,
        workflows: &[WorkflowRecord],
        runtime: Arc<dyn ScheduleToolRuntime>,
    ) -> Self {
        let active_agent_name = active_agent_name.into();
        let mut agent_names = workflows
            .iter()
            .filter_map(|workflow| workflow.agent_name.clone())
            .collect::<Vec<_>>();
        agent_names.push(active_agent_name.clone());
        agent_names.sort();
        agent_names.dedup();
        let mut workflow_names = workflows
            .iter()
            .map(|workflow| workflow.metadata.name.clone())
            .collect::<Vec<_>>();
        workflow_names.sort();
        workflow_names.dedup();
        Self {
            active_agent_name,
            agent_names,
            workflow_names,
            runtime,
        }
    }
}

#[async_trait]
impl Tool for CreateScheduleTool {
    fn name(&self) -> &str {
        "create_schedule"
    }

    fn description(&self) -> &str {
        "Create a confirmed one-shot or recurring schedule for an agent-owned workflow. The agent \
         defaults to the active agent. Provide exactly one of at or cron. Use inputText for string \
         workflows and input for object workflows."
    }

    fn parameters(&self) -> JsonSchema {
        JsonSchema {
            kind: JsonSchemaType::One(JsonValueType::Object),
            properties: BTreeMap::from([
                (
                    "agent".to_owned(),
                    JsonSchemaProperty {
                        kind: JsonSchemaType::One(JsonValueType::String),
                        description: Some(format!(
                            "Agent that owns the workflow; defaults to {}.",
                            self.active_agent_name
                        )),
                        allowed_values: Some(self.agent_names.clone()),
                        items: None,
                    },
                ),
                (
                    "at".to_owned(),
                    string_property(Some(
                        "RFC 3339 timestamp with an offset for a one-shot schedule.".to_owned(),
                    )),
                ),
                (
                    "cron".to_owned(),
                    string_property(Some("Five-field cron expression.".to_owned())),
                ),
                (
                    "input".to_owned(),
                    JsonSchemaProperty {
                        kind: JsonSchemaType::One(JsonValueType::Object),
                        description: Some(
                            "Structured input for an object-schema workflow.".to_owned(),
                        ),
                        allowed_values: None,
                        items: None,
                    },
                ),
                (
                    "inputText".to_owned(),
                    string_property(Some("Plain text input for a string workflow.".to_owned())),
                ),
                (
                    "name".to_owned(),
                    JsonSchemaProperty {
                        kind: JsonSchemaType::One(JsonValueType::String),
                        description: Some("Workflow to schedule.".to_owned()),
                        allowed_values: Some(self.workflow_names.clone()),
                        items: None,
                    },
                ),
                (
                    "timezone".to_owned(),
                    string_property(Some(
                        "IANA timezone for a cron schedule; defaults to the local timezone."
                            .to_owned(),
                    )),
                ),
            ]),
            required: vec!["name".to_owned()],
        }
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Schedule
    }

    fn permission_mode(&self) -> ToolPermissionMode {
        ToolPermissionMode::SelfManaged
    }

    async fn execute(
        &self,
        arguments: Map<String, Value>,
        context: &ToolExecutionContext,
    ) -> ToolResult {
        let request = match parse_schedule_request(&arguments, &self.active_agent_name) {
            Ok(request) => request,
            Err(error) => return ToolResult::failure(error),
        };
        if !self
            .agent_names
            .iter()
            .any(|agent_name| agent_name == &request.agent_name)
        {
            return ToolResult::failure(format!(
                "Agent \"{}\" is not available for schedule creation.",
                request.agent_name
            ));
        }
        if context.cancellation.is_cancelled() {
            return ToolResult::failure("Schedule creation cancelled.");
        }
        match self.runtime.create(request, &context.cancellation).await {
            Ok(schedule) => match serde_json::to_string_pretty(&schedule) {
                Ok(content) => ToolResult::success(content),
                Err(error) => ToolResult::failure(format!(
                    "Schedule was created, but its response could not be serialized: {error}"
                )),
            },
            Err(error) => ToolResult::failure(error),
        }
    }
}

pub fn parse_schedule_request(
    arguments: &Map<String, Value>,
    agent_name: &str,
) -> Result<ScheduleRequest, String> {
    let name = required_string(arguments, "name")?;
    let agent_name = optional_string(arguments, "agent")?.unwrap_or(agent_name);
    let at = optional_string(arguments, "at")?;
    let cron = optional_string(arguments, "cron")?;
    let timezone = optional_string(arguments, "timezone")?;
    let timing = match (at, cron) {
        (Some(at), None) => {
            if timezone.is_some() {
                return Err("Error: timezone is only valid with cron schedules.".to_owned());
            }
            let run_at = DateTime::parse_from_rfc3339(at)
                .map_err(|error| format!("Error: at must be an RFC 3339 timestamp: {error}"))?
                .with_timezone(&Utc);
            ScheduleTiming::Once { run_at }
        }
        (None, Some(expression)) => ScheduleTiming::Cron {
            expression: expression.to_owned(),
            timezone: timezone.map(str::to_owned),
        },
        (Some(_), Some(_)) => {
            return Err("Error: provide exactly one of at or cron.".to_owned());
        }
        (None, None) => return Err("Error: provide exactly one of at or cron.".to_owned()),
    };
    let input = match (arguments.get("input"), arguments.get("inputText")) {
        (Some(_), Some(_)) => {
            return Err("Error: provide input or inputText, not both.".to_owned());
        }
        (Some(input), None) if input.is_object() => input.clone(),
        (Some(_), None) => return Err("Error: input must be an object.".to_owned()),
        (None, Some(Value::String(input))) => Value::String(input.clone()),
        (None, Some(_)) => return Err("Error: inputText must be a string.".to_owned()),
        (None, None) => Value::String(String::new()),
    };
    Ok(ScheduleRequest {
        agent_name: agent_name.to_owned(),
        workflow_name: name.to_owned(),
        input,
        timing,
        now: None,
    })
}

fn required_string<'a>(arguments: &'a Map<String, Value>, name: &str) -> Result<&'a str, String> {
    optional_string(arguments, name)?
        .ok_or_else(|| format!("Error: {name} must be a non-empty string."))
}

fn optional_string<'a>(
    arguments: &'a Map<String, Value>,
    name: &str,
) -> Result<Option<&'a str>, String> {
    match arguments.get(name) {
        None => Ok(None),
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(Some(value)),
        Some(_) => Err(format!("Error: {name} must be a non-empty string.")),
    }
}

fn string_property(description: Option<String>) -> JsonSchemaProperty {
    JsonSchemaProperty {
        kind: JsonSchemaType::One(JsonValueType::String),
        description,
        allowed_values: None,
        items: None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use chrono::{DateTime, Utc};
    use flowmation_domain::ids::ScheduleId;
    use flowmation_domain::schedule::{ScheduleKind, ScheduleRecord, ScheduleStatus};
    use serde_json::{Map, json};
    use tokio_util::sync::CancellationToken;

    use super::{CreateScheduleTool, ScheduleToolRuntime, parse_schedule_request};
    use crate::policy::{
        AuthorizationDecision, FixedPermissionBroker, StandardAuthorizationPolicy,
    };
    use crate::scheduling::{ScheduleRequest, ScheduleTiming};
    use crate::tool::{EmptySecretsProvider, ExecutionMode, Tool, ToolExecutionContext};

    #[derive(Debug, Default)]
    struct RecordingRuntime {
        requests: Mutex<Vec<ScheduleRequest>>,
    }

    #[async_trait]
    impl ScheduleToolRuntime for RecordingRuntime {
        async fn create(
            &self,
            request: ScheduleRequest,
            _cancellation: &CancellationToken,
        ) -> Result<ScheduleRecord, String> {
            self.requests
                .lock()
                .map_err(|error| error.to_string())?
                .push(request.clone());
            let (kind, cron, timezone, next_run_at) = match request.timing {
                ScheduleTiming::Once { run_at } => (
                    ScheduleKind::Once,
                    String::new(),
                    "UTC".to_owned(),
                    run_at.to_rfc3339(),
                ),
                ScheduleTiming::Cron {
                    expression,
                    timezone,
                } => (
                    ScheduleKind::Cron,
                    expression,
                    timezone.unwrap_or_else(|| "UTC".to_owned()),
                    "2026-08-10T00:00:00Z".to_owned(),
                ),
            };
            Ok(ScheduleRecord {
                id: ScheduleId::new("schedule-1").map_err(|error| error.to_string())?,
                project_dir: PathBuf::from("/project"),
                agent_name: request.agent_name,
                workflow_name: request.workflow_name,
                input: request.input,
                kind,
                cron,
                timezone,
                package_fingerprint: "fingerprint".to_owned(),
                status: ScheduleStatus::Active,
                next_run_at,
                created_at: "2026-08-04T00:00:00Z".to_owned(),
                updated_at: "2026-08-04T00:00:00Z".to_owned(),
            })
        }
    }

    #[tokio::test]
    async fn tool_creates_and_returns_a_durable_schedule() -> Result<(), Box<dyn std::error::Error>>
    {
        let runtime = Arc::new(RecordingRuntime::default());
        let tool = CreateScheduleTool::new("finance", &[], runtime.clone());
        let result = tool
            .execute(
                object(json!({
                    "name": "remove-change",
                    "input": {"commit": "abc123"},
                    "at": "2026-08-09T09:00:00-06:00"
                }))?,
                &ToolExecutionContext {
                    cwd: PathBuf::from("/project"),
                    authorization: Arc::new(StandardAuthorizationPolicy::new(Arc::new(
                        FixedPermissionBroker::new(AuthorizationDecision::Allow),
                    ))),
                    secrets: Arc::new(EmptySecretsProvider),
                    execution_mode: ExecutionMode::Direct,
                    cancellation: CancellationToken::new(),
                },
            )
            .await;

        assert!(result.ok);
        assert!(result.content.contains("schedule-1"));
        let requests = runtime.requests.lock().map_err(|error| error.to_string())?;
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].agent_name, "finance");
        assert!(matches!(requests[0].timing, ScheduleTiming::Once { .. }));
        Ok(())
    }

    #[test]
    fn parses_one_shot_and_cron_requests() -> Result<(), Box<dyn std::error::Error>> {
        let once = parse_schedule_request(
            &object(json!({
                "name": "remove-change",
                "input": {"commit": "abc123"},
                "at": "2026-08-09T09:00:00-06:00"
            }))?,
            "main",
        )?;
        assert!(matches!(
            once.timing,
            ScheduleTiming::Once { run_at }
                if run_at == "2026-08-09T15:00:00Z".parse::<DateTime<Utc>>()?
        ));

        let recurring = parse_schedule_request(
            &object(json!({
                "agent": "finance",
                "name": "weekly-report",
                "inputText": "engineering",
                "cron": "0 9 * * 1",
                "timezone": "America/Denver"
            }))?,
            "main",
        )?;
        assert!(matches!(
            recurring.timing,
            ScheduleTiming::Cron { expression, timezone }
                if expression == "0 9 * * 1" && timezone.as_deref() == Some("America/Denver")
        ));
        assert_eq!(recurring.agent_name, "finance");
        Ok(())
    }

    #[test]
    fn requires_exactly_one_timing_mode() -> Result<(), Box<dyn std::error::Error>> {
        let neither = object(json!({"name": "report"}))?;
        let both = object(json!({
            "name": "report",
            "at": "2026-08-09T15:00:00Z",
            "cron": "0 9 * * 1"
        }))?;
        assert!(parse_schedule_request(&neither, "main").is_err());
        assert!(parse_schedule_request(&both, "main").is_err());
        Ok(())
    }

    fn object(value: serde_json::Value) -> Result<Map<String, serde_json::Value>, String> {
        value
            .as_object()
            .cloned()
            .ok_or_else(|| "expected object".to_owned())
    }
}
