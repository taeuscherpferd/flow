use std::collections::{BTreeMap, HashMap};
use std::fmt::Debug;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use flowmation_domain::chat::{
    FunctionDefinition, JsonSchema, JsonSchemaProperty, JsonSchemaType, JsonValueType,
    ToolDefinition, ToolDefinitionKind,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use crate::policy::{AuthorizationDecision, AuthorizationPolicy, PermissionRequest};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolEffect {
    Read,
    Write,
    Command,
    External,
    Schedule,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolPermissionMode {
    #[default]
    Effect,
    SelfManaged,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionMode {
    #[default]
    Direct,
    Delegated,
    Workflow,
    Scheduled,
}

pub trait SecretsProvider: Debug + Send + Sync {
    fn get(&self, name: &str) -> Option<String>;

    fn has(&self, name: &str) -> bool {
        self.get(name).is_some()
    }
}

#[derive(Debug, Default)]
pub struct EmptySecretsProvider;

impl SecretsProvider for EmptySecretsProvider {
    fn get(&self, _name: &str) -> Option<String> {
        None
    }
}

#[derive(Clone)]
pub struct ToolExecutionContext {
    pub cwd: PathBuf,
    pub authorization: Arc<dyn AuthorizationPolicy>,
    pub secrets: Arc<dyn SecretsProvider>,
    pub execution_mode: ExecutionMode,
    pub cancellation: CancellationToken,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolResult {
    pub ok: bool,
    pub content: String,
}

impl ToolResult {
    #[must_use]
    pub fn success(content: impl Into<String>) -> Self {
        Self {
            ok: true,
            content: content.into(),
        }
    }

    #[must_use]
    pub fn failure(content: impl Into<String>) -> Self {
        Self {
            ok: false,
            content: content.into(),
        }
    }
}

#[async_trait]
pub trait Tool: Debug + Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> JsonSchema;

    fn effect(&self) -> ToolEffect {
        ToolEffect::External
    }

    fn permission_mode(&self) -> ToolPermissionMode {
        ToolPermissionMode::Effect
    }

    async fn execute(
        &self,
        arguments: Map<String, Value>,
        context: &ToolExecutionContext,
    ) -> ToolResult;
}

#[derive(Clone, Debug, Default)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
    allowlist: Option<Vec<String>>,
}

impl ToolRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_allowlist(allowlist: Vec<String>) -> Self {
        Self {
            tools: HashMap::new(),
            allowlist: Some(allowlist),
        }
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        if self
            .allowlist
            .as_ref()
            .is_some_and(|allowlist| !allowlist.iter().any(|name| name == tool.name()))
        {
            return;
        }
        self.tools.insert(tool.name().to_owned(), tool);
    }

    #[must_use]
    pub(crate) fn excluding(&self, excluded_names: &[&str]) -> Self {
        let is_excluded = |name: &str| excluded_names.iter().any(|excluded| excluded == &name);
        Self {
            tools: self
                .tools
                .iter()
                .filter(|(name, _tool)| !is_excluded(name))
                .map(|(name, tool)| (name.clone(), Arc::clone(tool)))
                .collect(),
            allowlist: self.allowlist.as_ref().map(|allowlist| {
                allowlist
                    .iter()
                    .filter(|name| !is_excluded(name))
                    .cloned()
                    .collect()
            }),
        }
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    #[must_use]
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        let mut definitions: Vec<_> = self
            .tools
            .values()
            .map(|tool| ToolDefinition {
                kind: ToolDefinitionKind::Function,
                function: FunctionDefinition {
                    name: tool.name().to_owned(),
                    description: tool.description().to_owned(),
                    parameters: tool.parameters(),
                },
            })
            .collect();
        definitions.sort_by(|left, right| left.function.name.cmp(&right.function.name));
        definitions
    }

    pub async fn execute(
        &self,
        name: &str,
        arguments: Map<String, Value>,
        context: &ToolExecutionContext,
    ) -> ToolResult {
        let Some(tool) = self.get(name) else {
            return ToolResult::failure(format!("Error: no such tool \"{name}\""));
        };
        if context.cancellation.is_cancelled() {
            return ToolResult::failure("Tool execution cancelled.");
        }
        let request = PermissionRequest {
            tool_name: name.to_owned(),
            arguments: arguments.clone(),
            effect: tool.effect(),
            permission_mode: tool.permission_mode(),
            execution_mode: context.execution_mode,
        };
        match context.authorization.authorize(request).await {
            AuthorizationDecision::Allow => tool.execute(arguments, context).await,
            AuthorizationDecision::Deny => {
                ToolResult::failure(format!("Permission denied for tool \"{name}\"."))
            }
        }
    }
}

#[derive(Debug)]
pub struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }

    fn description(&self) -> &str {
        "Returns the supplied text."
    }

    fn parameters(&self) -> JsonSchema {
        object_schema(
            [(
                "text",
                string_schema_property(Some("Text to return.".to_owned())),
            )],
            ["text"],
        )
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Read
    }

    async fn execute(
        &self,
        arguments: Map<String, Value>,
        _context: &ToolExecutionContext,
    ) -> ToolResult {
        match arguments.get("text").and_then(Value::as_str) {
            Some(text) => ToolResult::success(text),
            None => ToolResult::failure("Expected string argument \"text\"."),
        }
    }
}

pub(crate) fn object_schema<const P: usize, const R: usize>(
    properties: [(&str, JsonSchemaProperty); P],
    required: [&str; R],
) -> JsonSchema {
    JsonSchema {
        kind: JsonSchemaType::One(JsonValueType::Object),
        properties: properties
            .into_iter()
            .map(|(name, property)| (name.to_owned(), property))
            .collect::<BTreeMap<_, _>>(),
        required: required.into_iter().map(str::to_owned).collect(),
    }
}

pub(crate) fn string_schema_property(description: Option<String>) -> JsonSchemaProperty {
    JsonSchemaProperty {
        kind: JsonSchemaType::One(JsonValueType::String),
        description,
        allowed_values: None,
        items: None,
    }
}
