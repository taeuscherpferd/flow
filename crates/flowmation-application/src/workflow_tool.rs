use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::Arc;

use async_trait::async_trait;
use flowmation_domain::chat::{JsonSchema, JsonSchemaProperty, JsonSchemaType, JsonValueType};
use flowmation_domain::schema::{WorkflowSchema, validate_schema};
use flowmation_workflow_host::protocol::AgentInvocationPolicy;
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use crate::tool::{Tool, ToolEffect, ToolExecutionContext, ToolPermissionMode, ToolResult};
use crate::workflow::WorkflowRecord;

#[derive(Clone, Debug, PartialEq)]
pub struct WorkflowConfirmation {
    pub workflow_name: String,
    pub description: String,
    pub input: Value,
    pub message: String,
}

impl WorkflowConfirmation {
    fn new(record: &WorkflowRecord, input: Value) -> Self {
        let serialized_input =
            serde_json::to_string_pretty(&input).unwrap_or_else(|_| input.to_string());
        Self {
            workflow_name: record.metadata.name.clone(),
            description: record.metadata.description.clone(),
            input,
            message: format!(
                "Run workflow \"{}\"?\n\n{}\n\nInput:\n{}",
                record.metadata.name, record.metadata.description, serialized_input
            ),
        }
    }
}

#[async_trait]
pub trait WorkflowToolRuntime: Debug + Send + Sync {
    async fn resolve(&self, name: &str) -> Option<WorkflowRecord>;

    async fn invoke(
        &self,
        record: &WorkflowRecord,
        input: Value,
        cancellation: &CancellationToken,
    ) -> Result<String, String>;

    async fn confirm(&self, confirmation: WorkflowConfirmation) -> bool;
}

#[derive(Debug)]
pub struct RunWorkflowTool {
    eligible_workflows: Vec<WorkflowRecord>,
    runtime: Arc<dyn WorkflowToolRuntime>,
}

impl RunWorkflowTool {
    #[must_use]
    pub fn new(workflows: &[WorkflowRecord], runtime: Arc<dyn WorkflowToolRuntime>) -> Self {
        Self {
            eligible_workflows: workflows
                .iter()
                .filter(|record| {
                    record.metadata.agent_invocation != AgentInvocationPolicy::Disabled
                })
                .cloned()
                .collect(),
            runtime,
        }
    }
}

#[async_trait]
impl Tool for RunWorkflowTool {
    fn name(&self) -> &str {
        "run_workflow"
    }

    fn description(&self) -> &str {
        "Run an eligible developer workflow when it directly matches the user's request. Use \
         inputText for string workflows and input for schema-based object workflows."
    }

    fn parameters(&self) -> JsonSchema {
        let workflow_names = self
            .eligible_workflows
            .iter()
            .map(|record| record.metadata.name.clone())
            .collect();
        let structured_contracts = structured_input_contracts(&self.eligible_workflows);
        JsonSchema {
            kind: JsonSchemaType::One(JsonValueType::Object),
            properties: BTreeMap::from([
                (
                    "input".to_owned(),
                    JsonSchemaProperty {
                        kind: JsonSchemaType::One(JsonValueType::Object),
                        description: Some(if structured_contracts.is_empty() {
                            "Structured input for an object-schema workflow.".to_owned()
                        } else {
                            format!(
                                "Structured input for an object-schema workflow. Match the \
                                 selected workflow's schema:\n{structured_contracts}"
                            )
                        }),
                        allowed_values: None,
                        items: None,
                    },
                ),
                (
                    "inputText".to_owned(),
                    JsonSchemaProperty {
                        kind: JsonSchemaType::One(JsonValueType::String),
                        description: Some("Plain text input for a string workflow.".to_owned()),
                        allowed_values: None,
                        items: None,
                    },
                ),
                (
                    "name".to_owned(),
                    JsonSchemaProperty {
                        kind: JsonSchemaType::One(JsonValueType::String),
                        description: Some("The workflow name.".to_owned()),
                        allowed_values: Some(workflow_names),
                        items: None,
                    },
                ),
            ]),
            required: vec!["name".to_owned()],
        }
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::External
    }

    fn permission_mode(&self) -> ToolPermissionMode {
        ToolPermissionMode::SelfManaged
    }

    async fn execute(
        &self,
        arguments: Map<String, Value>,
        context: &ToolExecutionContext,
    ) -> ToolResult {
        if context.cancellation.is_cancelled() {
            return ToolResult::failure("Workflow execution cancelled.");
        }
        let Some(name) = arguments.get("name").and_then(Value::as_str) else {
            return ToolResult::failure("Error: workflow name must be a string.");
        };
        let Some(record) = self.runtime.resolve(name).await else {
            return ToolResult::failure(format!("Error: workflow \"{name}\" is not eligible."));
        };
        if record.metadata.agent_invocation == AgentInvocationPolicy::Disabled {
            return ToolResult::failure(format!("Error: workflow \"{name}\" is not eligible."));
        }
        let input = match workflow_input(&record, &arguments) {
            Ok(input) => input,
            Err(message) => return ToolResult::failure(message),
        };
        if record.metadata.agent_invocation == AgentInvocationPolicy::Confirm
            && !self
                .runtime
                .confirm(WorkflowConfirmation::new(&record, input.clone()))
                .await
        {
            return ToolResult::failure(format!("The user declined workflow \"{name}\"."));
        }
        if context.cancellation.is_cancelled() {
            return ToolResult::failure("Workflow execution cancelled.");
        }
        match self
            .runtime
            .invoke(&record, input, &context.cancellation)
            .await
        {
            Ok(content) => ToolResult::success(content),
            Err(_message) if context.cancellation.is_cancelled() => {
                ToolResult::failure("Workflow execution cancelled.")
            }
            Err(message) => {
                ToolResult::failure(format!("Error running workflow \"{name}\": {message}"))
            }
        }
    }
}

#[must_use]
pub fn build_workflow_system_context(workflows: &[WorkflowRecord]) -> String {
    let eligible = workflows
        .iter()
        .filter(|record| record.metadata.agent_invocation != AgentInvocationPolicy::Disabled)
        .collect::<Vec<_>>();
    if eligible.is_empty() {
        return String::new();
    }
    let listing = eligible
        .iter()
        .map(|record| {
            let input = record.metadata.input_schema.as_ref().map_or_else(
                || "plain text input".to_owned(),
                |schema| {
                    if schema.get("type").and_then(Value::as_str) == Some("object") {
                        format!("structured input matching {}", compact_json(schema))
                    } else {
                        "plain text input".to_owned()
                    }
                },
            );
            format!(
                "- {} ({}, {}): {}",
                record.metadata.name,
                policy_name(record.metadata.agent_invocation),
                input,
                record.metadata.description
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "## Available Workflows\n\nUse run_workflow only when one of these workflows directly \
         matches the request.\n\n{listing}"
    )
}

fn workflow_input(
    record: &WorkflowRecord,
    arguments: &Map<String, Value>,
) -> Result<Value, String> {
    let schema_value = record.metadata.input_schema.as_ref();
    let input = if schema_value
        .and_then(|schema| schema.get("type"))
        .and_then(Value::as_str)
        == Some("object")
    {
        let Some(input) = arguments.get("input").filter(|input| input.is_object()) else {
            return Err(format!(
                "Error: workflow \"{}\" requires object input.",
                record.metadata.name
            ));
        };
        input.clone()
    } else {
        let Some(input) = arguments.get("inputText").and_then(Value::as_str) else {
            return Err(format!(
                "Error: workflow \"{}\" requires inputText.",
                record.metadata.name
            ));
        };
        Value::String(input.to_owned())
    };
    if let Some(schema_value) = schema_value {
        let schema: WorkflowSchema =
            serde_json::from_value(schema_value.clone()).map_err(|error| {
                format!(
                    "Error: workflow \"{}\" has an invalid input schema: {error}",
                    record.metadata.name
                )
            })?;
        let validation = validate_schema(&schema, &input);
        if !validation.valid {
            return Err(format!(
                "Error: invalid input for workflow \"{}\": {}",
                record.metadata.name,
                validation.errors.join(" ")
            ));
        }
    }
    Ok(input)
}

fn structured_input_contracts(workflows: &[WorkflowRecord]) -> String {
    workflows
        .iter()
        .filter_map(|record| {
            record.metadata.input_schema.as_ref().and_then(|schema| {
                (schema.get("type").and_then(Value::as_str) == Some("object"))
                    .then(|| format!("{}: {}", record.metadata.name, compact_json(schema)))
            })
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn compact_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
}

const fn policy_name(policy: AgentInvocationPolicy) -> &'static str {
    match policy {
        AgentInvocationPolicy::Disabled => "disabled",
        AgentInvocationPolicy::Confirm => "confirm",
        AgentInvocationPolicy::Automatic => "automatic",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use flowmation_domain::agent::PackageSource;
    use flowmation_workflow_host::protocol::{
        AgentInvocationPolicy, WorkflowMetadata, WorkflowPresentation,
    };
    use serde_json::{Map, Value, json};
    use tokio_util::sync::CancellationToken;

    use super::{
        RunWorkflowTool, WorkflowConfirmation, WorkflowToolRuntime, build_workflow_system_context,
    };
    use crate::policy::{
        AuthorizationDecision, FixedPermissionBroker, StandardAuthorizationPolicy,
    };
    use crate::tool::{EmptySecretsProvider, ExecutionMode, Tool, ToolExecutionContext};
    use crate::workflow::WorkflowRecord;

    #[derive(Debug)]
    struct RecordingRuntime {
        records: Mutex<BTreeMap<String, WorkflowRecord>>,
        invocations: Mutex<Vec<(String, Value)>>,
        confirmations: Mutex<Vec<WorkflowConfirmation>>,
    }

    #[async_trait]
    impl WorkflowToolRuntime for RecordingRuntime {
        async fn resolve(&self, name: &str) -> Option<WorkflowRecord> {
            self.records.lock().ok()?.get(name).cloned()
        }

        async fn invoke(
            &self,
            record: &WorkflowRecord,
            input: Value,
            _cancellation: &CancellationToken,
        ) -> Result<String, String> {
            self.invocations
                .lock()
                .map_err(|error| error.to_string())?
                .push((record.metadata.name.clone(), input));
            Ok("completed".to_owned())
        }

        async fn confirm(&self, confirmation: WorkflowConfirmation) -> bool {
            self.confirmations
                .lock()
                .map(|mut confirmations| confirmations.push(confirmation))
                .is_ok()
        }
    }

    #[tokio::test]
    async fn runs_eligible_workflows_and_confirms_with_input_details()
    -> Result<(), Box<dyn std::error::Error>> {
        let records = vec![
            record("hidden", AgentInvocationPolicy::Disabled, None),
            record("review", AgentInvocationPolicy::Confirm, None),
            record(
                "structured",
                AgentInvocationPolicy::Automatic,
                Some(json!({
                    "type": "object",
                    "properties": {"value": {"type": "string"}},
                    "required": ["value"]
                })),
            ),
        ];
        let runtime = Arc::new(RecordingRuntime {
            records: Mutex::new(
                records
                    .iter()
                    .map(|record| (record.metadata.name.clone(), record.clone()))
                    .collect(),
            ),
            invocations: Mutex::new(Vec::new()),
            confirmations: Mutex::new(Vec::new()),
        });
        let tool = RunWorkflowTool::new(
            &records,
            Arc::clone(&runtime) as Arc<dyn WorkflowToolRuntime>,
        );

        let review = tool
            .execute(
                Map::from_iter([
                    ("name".to_owned(), Value::String("review".to_owned())),
                    ("inputText".to_owned(), Value::String("change".to_owned())),
                ]),
                &tool_context(),
            )
            .await;
        let structured = tool
            .execute(
                Map::from_iter([
                    ("name".to_owned(), Value::String("structured".to_owned())),
                    ("input".to_owned(), json!({"value": "ok"})),
                ]),
                &tool_context(),
            )
            .await;

        assert!(review.ok);
        assert!(structured.ok);
        let confirmations = runtime
            .confirmations
            .lock()
            .map_err(|error| error.to_string())?;
        assert_eq!(confirmations.len(), 1);
        assert_eq!(confirmations[0].description, "review workflow");
        assert_eq!(confirmations[0].input, Value::String("change".to_owned()));
        assert!(confirmations[0].message.contains("review workflow"));
        assert!(confirmations[0].message.contains("\"change\""));
        drop(confirmations);
        assert_eq!(
            *runtime
                .invocations
                .lock()
                .map_err(|error| error.to_string())?,
            vec![
                ("review".to_owned(), Value::String("change".to_owned())),
                ("structured".to_owned(), json!({"value": "ok"})),
            ]
        );
        let parameters = tool.parameters();
        assert_eq!(
            parameters.properties["name"].allowed_values,
            Some(vec!["review".to_owned(), "structured".to_owned()])
        );
        assert!(
            parameters.properties["input"]
                .description
                .as_deref()
                .unwrap_or_default()
                .contains("\"required\":[\"value\"]")
        );
        assert!(build_workflow_system_context(&records).contains("structured input matching"));
        Ok(())
    }

    #[tokio::test]
    async fn cached_tool_uses_current_workflow_policy() -> Result<(), Box<dyn std::error::Error>> {
        let original = record("deploy", AgentInvocationPolicy::Automatic, None);
        let current = record("deploy", AgentInvocationPolicy::Confirm, None);
        let runtime = Arc::new(RecordingRuntime {
            records: Mutex::new(BTreeMap::from([("deploy".to_owned(), current)])),
            invocations: Mutex::new(Vec::new()),
            confirmations: Mutex::new(Vec::new()),
        });
        let tool = RunWorkflowTool::new(
            &[original],
            Arc::clone(&runtime) as Arc<dyn WorkflowToolRuntime>,
        );

        let result = tool
            .execute(
                Map::from_iter([
                    ("name".to_owned(), Value::String("deploy".to_owned())),
                    (
                        "inputText".to_owned(),
                        Value::String("production".to_owned()),
                    ),
                ]),
                &tool_context(),
            )
            .await;

        assert!(result.ok);
        assert_eq!(
            runtime
                .confirmations
                .lock()
                .map_err(|error| error.to_string())?
                .len(),
            1
        );
        Ok(())
    }

    fn record(
        name: &str,
        policy: AgentInvocationPolicy,
        input_schema: Option<Value>,
    ) -> WorkflowRecord {
        WorkflowRecord {
            metadata: WorkflowMetadata {
                name: name.to_owned(),
                description: format!("{name} workflow"),
                input_schema,
                agent_invocation: policy,
                presentation: WorkflowPresentation::Direct,
            },
            directory: PathBuf::from(name),
            entry_path: PathBuf::from(name).join("WORKFLOW.js"),
            fingerprint: name.to_owned(),
            source: PackageSource::Global,
            agent_name: None,
            resource_id: None,
        }
    }

    fn tool_context() -> ToolExecutionContext {
        ToolExecutionContext {
            cwd: std::env::temp_dir(),
            authorization: Arc::new(StandardAuthorizationPolicy::new(Arc::new(
                FixedPermissionBroker::new(AuthorizationDecision::Allow),
            ))),
            secrets: Arc::new(EmptySecretsProvider),
            execution_mode: ExecutionMode::Direct,
            cancellation: CancellationToken::new(),
        }
    }
}
