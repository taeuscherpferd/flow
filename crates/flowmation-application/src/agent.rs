use std::sync::Arc;

use thiserror::Error;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::provider::{
    ChatCompletionOptions, ChatCompletionRequest, ChatMessage, ChatRole, ModelProvider,
    ProviderError, ThinkingMode,
};
use crate::tool::{ToolExecutionContext, ToolRegistry};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AgentTools {
    #[default]
    Default,
    None,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AgentTurnOptions {
    pub tools: AgentTools,
    pub thinking: Option<ThinkingMode>,
}

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("agent turn was cancelled")]
    Cancelled,
    #[error(transparent)]
    Provider(#[from] ProviderError),
}

pub struct AgentService {
    provider: Arc<dyn ModelProvider>,
    model: String,
    context_window: u64,
    tools: Arc<ToolRegistry>,
    history: Vec<ChatMessage>,
    tool_context: ToolExecutionContext,
}

impl AgentService {
    #[must_use]
    pub fn new(
        provider: Arc<dyn ModelProvider>,
        model: impl Into<String>,
        context_window: u64,
        tools: Arc<ToolRegistry>,
        system_prompt: impl Into<String>,
        tool_context: ToolExecutionContext,
    ) -> Self {
        Self {
            provider,
            model: model.into(),
            context_window,
            tools,
            history: vec![ChatMessage::new(ChatRole::System, system_prompt)],
            tool_context,
        }
    }

    #[must_use]
    pub fn from_history(
        provider: Arc<dyn ModelProvider>,
        model: impl Into<String>,
        context_window: u64,
        tools: Arc<ToolRegistry>,
        history: Vec<ChatMessage>,
        tool_context: ToolExecutionContext,
    ) -> Self {
        Self {
            provider,
            model: model.into(),
            context_window,
            tools,
            history,
            tool_context,
        }
    }

    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn set_model(
        &mut self,
        provider: Arc<dyn ModelProvider>,
        model: impl Into<String>,
        context_window: u64,
    ) {
        self.provider = provider;
        self.model = model.into();
        self.context_window = context_window;
    }

    pub fn register_direct_tool(&mut self, tool: Arc<dyn crate::tool::Tool>) {
        Arc::make_mut(&mut self.tools).register(tool);
    }

    #[must_use]
    pub fn create_session_service(
        &self,
        provider: Arc<dyn ModelProvider>,
        model: impl Into<String>,
        context_window: u64,
        history: Option<Vec<ChatMessage>>,
    ) -> Self {
        let history = history.unwrap_or_else(|| {
            self.history
                .iter()
                .filter(|message| message.role == ChatRole::System)
                .cloned()
                .collect()
        });
        Self::from_history(
            provider,
            model,
            context_window,
            Arc::new(self.tools.excluding(&["run_workflow"])),
            history,
            self.tool_context.clone(),
        )
    }

    pub async fn handle_user_message(
        &mut self,
        text: impl Into<String>,
        options: AgentTurnOptions,
        cancellation: &CancellationToken,
    ) -> Result<String, AgentError> {
        if cancellation.is_cancelled() {
            return Err(AgentError::Cancelled);
        }
        self.history
            .push(ChatMessage::new(ChatRole::User, text.into()));
        self.compact_if_needed();

        loop {
            if cancellation.is_cancelled() {
                return Err(AgentError::Cancelled);
            }
            let result = self
                .provider
                .chat(
                    ChatCompletionRequest {
                        model: self.model.clone(),
                        messages: self.history.clone(),
                        tools: if options.tools == AgentTools::Default {
                            self.tools.definitions()
                        } else {
                            Vec::new()
                        },
                        options: ChatCompletionOptions {
                            num_ctx: Some(self.context_window),
                            thinking: options.thinking,
                        },
                    },
                    cancellation,
                )
                .await?;
            if cancellation.is_cancelled() {
                return Err(AgentError::Cancelled);
            }
            let response = if options.tools == AgentTools::None {
                ChatMessage {
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                    tool_name: None,
                    ..result.message
                }
            } else {
                result.message
            };
            let content = response.content.clone();
            let tool_calls = response.tool_calls.clone();
            self.history.push(response);
            if options.tools == AgentTools::None || tool_calls.is_empty() {
                return Ok(content);
            }
            for call in tool_calls {
                if cancellation.is_cancelled() {
                    return Err(AgentError::Cancelled);
                }
                let mut tool_context = self.tool_context.clone();
                tool_context.cancellation = cancellation.child_token();
                let result = self
                    .tools
                    .execute(&call.name, call.arguments, &tool_context)
                    .await;
                if cancellation.is_cancelled() {
                    return Err(AgentError::Cancelled);
                }
                self.history.push(ChatMessage {
                    role: ChatRole::Tool,
                    content: result.content,
                    thinking: None,
                    tool_calls: Vec::new(),
                    tool_call_id: Some(call.id),
                    tool_name: Some(call.name),
                });
            }
        }
    }

    pub fn clear_history(&mut self, system_contexts: &[String]) {
        self.history
            .retain(|message| message.role == ChatRole::System);
        self.history.truncate(1);
        self.history.extend(
            system_contexts
                .iter()
                .filter(|context| !context.trim().is_empty())
                .map(|context| ChatMessage::new(ChatRole::System, context)),
        );
    }

    pub fn inject_skill_body(&mut self, name: &str, body: &str) {
        self.history.push(ChatMessage::new(
            ChatRole::User,
            format!("[Loaded skill \"{name}\" per user request]\n\n{body}"),
        ));
    }

    pub fn inject_system_context(&mut self, content: &str) {
        if !content.trim().is_empty() {
            self.history
                .push(ChatMessage::new(ChatRole::System, content));
        }
    }

    pub fn replace_system_context(&mut self, previous: &str, next: &str) {
        if !previous.is_empty() {
            self.history
                .retain(|message| message.role != ChatRole::System || message.content != previous);
        }
        self.inject_system_context(next);
    }

    #[must_use]
    pub fn snapshot_history(&self) -> Vec<ChatMessage> {
        self.history.clone()
    }

    pub fn restore_history(&mut self, history: Vec<ChatMessage>) {
        self.history = history;
    }

    fn compact_if_needed(&mut self) {
        let estimated_tokens: usize = self
            .history
            .iter()
            .map(|message| {
                (message.content.len() + message.thinking.as_ref().map_or(0, String::len))
                    .div_ceil(4)
            })
            .sum();
        if estimated_tokens < (self.context_window as usize * 85 / 100) {
            return;
        }
        let systems: Vec<_> = self
            .history
            .iter()
            .filter(|message| message.role == ChatRole::System)
            .cloned()
            .collect();
        let conversation: Vec<_> = self
            .history
            .iter()
            .filter(|message| message.role != ChatRole::System)
            .cloned()
            .collect();
        let target = self.context_window as usize * 45 / 100;
        let mut retained_tokens = 0;
        let mut start = conversation.len();
        while start > 0 && retained_tokens < target {
            start -= 1;
            retained_tokens += (conversation[start].content.len()
                + conversation[start].thinking.as_ref().map_or(0, String::len))
            .div_ceil(4);
        }
        while start < conversation.len() && conversation[start].role != ChatRole::User {
            start += 1;
        }
        self.history = systems;
        self.history.push(ChatMessage::new(
            ChatRole::System,
            format!("[Conversation history compacted: {start} older messages were removed.]"),
        ));
        self.history.extend(conversation.into_iter().skip(start));
    }
}

pub struct AgentSession {
    pub id: Uuid,
    provider_name: String,
    default_thinking: Option<ThinkingMode>,
    service: AgentService,
}

impl AgentSession {
    #[must_use]
    pub fn new(
        provider_name: impl Into<String>,
        default_thinking: Option<ThinkingMode>,
        service: AgentService,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            provider_name: provider_name.into(),
            default_thinking,
            service,
        }
    }

    #[must_use]
    pub fn model(&self) -> (&str, &str) {
        (&self.provider_name, self.service.model())
    }

    pub fn retarget(
        &mut self,
        provider_name: impl Into<String>,
        provider: Arc<dyn ModelProvider>,
        model: impl Into<String>,
        context_window: u64,
    ) {
        self.provider_name = provider_name.into();
        self.service.set_model(provider, model, context_window);
    }

    pub async fn run(
        &mut self,
        prompt: impl Into<String>,
        options: AgentTurnOptions,
        cancellation: &CancellationToken,
    ) -> Result<String, AgentError> {
        self.service
            .handle_user_message(
                prompt,
                AgentTurnOptions {
                    thinking: options.thinking.or(self.default_thinking),
                    ..options
                },
                cancellation,
            )
            .await
    }

    #[must_use]
    pub fn snapshot_history(&self) -> Vec<ChatMessage> {
        self.service.snapshot_history()
    }

    pub fn restore_history(&mut self, history: Vec<ChatMessage>) {
        self.service.restore_history(history);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use serde_json::Map;
    use tokio::sync::Notify;

    use super::{AgentError, AgentService, AgentSession, AgentTools, AgentTurnOptions};
    use crate::policy::{
        AuthorizationDecision, FixedPermissionBroker, StandardAuthorizationPolicy,
    };
    use crate::provider::{
        ChatCompletionRequest, ChatCompletionResult, ChatMessage, ChatRole, ModelProvider,
        ProviderError, ThinkingMode, ToolCall,
    };
    use crate::tool::{
        EchoTool, EmptySecretsProvider, ExecutionMode, Tool, ToolExecutionContext, ToolRegistry,
        ToolResult, object_schema,
    };
    use tokio_util::sync::CancellationToken;

    #[derive(Debug)]
    struct RecordingProvider {
        requests: Mutex<Vec<ChatCompletionRequest>>,
        responses: Mutex<VecDeque<ChatCompletionResult>>,
    }

    #[async_trait]
    impl ModelProvider for RecordingProvider {
        fn id(&self) -> &str {
            "recording"
        }

        async fn chat(
            &self,
            request: ChatCompletionRequest,
            _cancellation: &CancellationToken,
        ) -> Result<ChatCompletionResult, ProviderError> {
            self.requests
                .lock()
                .map_err(|error| ProviderError::Unavailable(error.to_string()))?
                .push(request);
            self.responses
                .lock()
                .map_err(|error| ProviderError::Unavailable(error.to_string()))?
                .pop_front()
                .ok_or_else(|| ProviderError::InvalidResponse("no response queued".to_owned()))
        }
    }

    fn service(provider: Arc<RecordingProvider>) -> AgentService {
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(EchoTool));
        service_with_tools(provider, tools)
    }

    fn service_with_tools(provider: Arc<RecordingProvider>, tools: ToolRegistry) -> AgentService {
        let authorization = Arc::new(StandardAuthorizationPolicy::new(Arc::new(
            FixedPermissionBroker::new(AuthorizationDecision::Allow),
        )));
        AgentService::new(
            provider,
            "test-model",
            8_192,
            Arc::new(tools),
            "system",
            ToolExecutionContext {
                cwd: std::env::temp_dir(),
                authorization,
                secrets: Arc::new(EmptySecretsProvider),
                execution_mode: ExecutionMode::Direct,
                cancellation: CancellationToken::new(),
            },
        )
    }

    #[derive(Debug)]
    struct BlockingTool {
        started: Arc<Notify>,
    }

    #[async_trait]
    impl Tool for BlockingTool {
        fn name(&self) -> &str {
            "blocking"
        }

        fn description(&self) -> &str {
            "Waits for cancellation."
        }

        fn parameters(&self) -> flowmation_domain::chat::JsonSchema {
            object_schema([], [])
        }

        async fn execute(
            &self,
            _arguments: Map<String, serde_json::Value>,
            context: &ToolExecutionContext,
        ) -> ToolResult {
            self.started.notify_one();
            context.cancellation.cancelled().await;
            ToolResult::failure("Tool execution cancelled.")
        }
    }

    #[derive(Debug)]
    struct CountingTool {
        executions: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Tool for CountingTool {
        fn name(&self) -> &str {
            "skipped"
        }

        fn description(&self) -> &str {
            "Records each execution."
        }

        fn parameters(&self) -> flowmation_domain::chat::JsonSchema {
            object_schema([], [])
        }

        async fn execute(
            &self,
            _arguments: Map<String, serde_json::Value>,
            _context: &ToolExecutionContext,
        ) -> ToolResult {
            self.executions.fetch_add(1, Ordering::SeqCst);
            ToolResult::success("unexpected")
        }
    }

    #[tokio::test]
    async fn omits_and_ignores_tools_when_disabled() -> Result<(), Box<dyn std::error::Error>> {
        let provider = Arc::new(RecordingProvider {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(VecDeque::from([ChatCompletionResult {
                message: ChatMessage {
                    role: ChatRole::Assistant,
                    content: "complete".to_owned(),
                    thinking: None,
                    tool_calls: vec![ToolCall {
                        id: "ignored".to_owned(),
                        name: "echo".to_owned(),
                        arguments: Map::new(),
                    }],
                    tool_call_id: None,
                    tool_name: None,
                },
            }])),
        });
        let mut service = service(provider.clone());
        let content = service
            .handle_user_message(
                "Do not use tools",
                AgentTurnOptions {
                    tools: AgentTools::None,
                    thinking: None,
                },
                &CancellationToken::new(),
            )
            .await?;
        assert_eq!(content, "complete");
        let requests = provider
            .requests
            .lock()
            .map_err(|error| error.to_string())?;
        assert!(requests[0].tools.is_empty());
        assert!(service.snapshot_history()[2].tool_calls.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn keeps_tools_enabled_and_omits_thinking_by_default()
    -> Result<(), Box<dyn std::error::Error>> {
        let provider = Arc::new(RecordingProvider {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(VecDeque::from([ChatCompletionResult {
                message: ChatMessage::new(ChatRole::Assistant, "complete"),
            }])),
        });
        let mut service = service(Arc::clone(&provider));
        service
            .handle_user_message(
                "Use provider defaults",
                AgentTurnOptions::default(),
                &CancellationToken::new(),
            )
            .await?;
        let requests = provider
            .requests
            .lock()
            .map_err(|error| error.to_string())?;
        assert!(!requests[0].tools.is_empty());
        assert_eq!(requests[0].options.thinking, None);
        Ok(())
    }

    #[tokio::test]
    async fn tool_free_history_retains_thinking_and_strips_tool_calls()
    -> Result<(), Box<dyn std::error::Error>> {
        let provider = Arc::new(RecordingProvider {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(VecDeque::from([ChatCompletionResult {
                message: ChatMessage {
                    role: ChatRole::Assistant,
                    content: "complete".to_owned(),
                    thinking: Some("private reasoning".to_owned()),
                    tool_calls: vec![ToolCall {
                        id: "ignored".to_owned(),
                        name: "echo".to_owned(),
                        arguments: Map::new(),
                    }],
                    tool_call_id: None,
                    tool_name: None,
                },
            }])),
        });
        let mut service = service(provider);
        service
            .handle_user_message(
                "Answer directly",
                AgentTurnOptions {
                    tools: AgentTools::None,
                    thinking: None,
                },
                &CancellationToken::new(),
            )
            .await?;
        let response = service.snapshot_history().pop().ok_or("missing response")?;
        assert_eq!(response.thinking.as_deref(), Some("private reasoning"));
        assert!(response.tool_calls.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn thinking_applies_to_every_request_in_tool_loop()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut arguments = Map::new();
        arguments.insert(
            "text".to_owned(),
            serde_json::Value::String("tool output".to_owned()),
        );
        let provider = Arc::new(RecordingProvider {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(VecDeque::from([
                ChatCompletionResult {
                    message: ChatMessage {
                        role: ChatRole::Assistant,
                        content: String::new(),
                        thinking: Some("use a tool".to_owned()),
                        tool_calls: vec![ToolCall {
                            id: "call-1".to_owned(),
                            name: "echo".to_owned(),
                            arguments,
                        }],
                        tool_call_id: None,
                        tool_name: None,
                    },
                },
                ChatCompletionResult {
                    message: ChatMessage {
                        role: ChatRole::Assistant,
                        content: "complete".to_owned(),
                        thinking: Some("answer".to_owned()),
                        tool_calls: Vec::new(),
                        tool_call_id: None,
                        tool_name: None,
                    },
                },
            ])),
        });
        let mut service = service(provider.clone());
        service
            .handle_user_message(
                "Use a tool",
                AgentTurnOptions {
                    tools: AgentTools::Default,
                    thinking: Some(ThinkingMode::High),
                },
                &CancellationToken::new(),
            )
            .await?;
        let requests = provider
            .requests
            .lock()
            .map_err(|error| error.to_string())?;
        assert_eq!(requests.len(), 2);
        assert!(
            requests
                .iter()
                .all(|request| request.options.thinking == Some(ThinkingMode::High))
        );
        assert_eq!(
            requests[1].messages[2].thinking.as_deref(),
            Some("use a tool")
        );
        Ok(())
    }

    #[tokio::test]
    async fn continues_tool_loop_until_the_model_finishes() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut responses = VecDeque::new();
        for index in 0..9 {
            let mut arguments = Map::new();
            arguments.insert(
                "text".to_owned(),
                serde_json::Value::String(format!("tool output {index}")),
            );
            responses.push_back(ChatCompletionResult {
                message: ChatMessage {
                    role: ChatRole::Assistant,
                    content: String::new(),
                    thinking: None,
                    tool_calls: vec![ToolCall {
                        id: format!("call-{index}"),
                        name: "echo".to_owned(),
                        arguments,
                    }],
                    tool_call_id: None,
                    tool_name: None,
                },
            });
        }
        responses.push_back(ChatCompletionResult {
            message: ChatMessage::new(ChatRole::Assistant, "complete"),
        });
        let provider = Arc::new(RecordingProvider {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(responses),
        });
        let mut service = service(Arc::clone(&provider));

        let content = service
            .handle_user_message(
                "Complete a long tool-driven task",
                AgentTurnOptions::default(),
                &CancellationToken::new(),
            )
            .await?;

        assert_eq!(content, "complete");
        assert_eq!(
            provider
                .requests
                .lock()
                .map_err(|error| error.to_string())?
                .len(),
            10
        );
        Ok(())
    }

    #[tokio::test]
    async fn clear_restores_static_system_context() -> Result<(), Box<dyn std::error::Error>> {
        let provider = Arc::new(RecordingProvider {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(VecDeque::from([ChatCompletionResult {
                message: ChatMessage::new(ChatRole::Assistant, "done"),
            }])),
        });
        let mut service = service(provider);
        service
            .handle_user_message(
                "temporary",
                AgentTurnOptions::default(),
                &CancellationToken::new(),
            )
            .await?;
        service.clear_history(&["workflow context".to_owned()]);
        assert_eq!(
            service.snapshot_history(),
            vec![
                ChatMessage::new(ChatRole::System, "system"),
                ChatMessage::new(ChatRole::System, "workflow context")
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn cancellation_aborts_active_tool_and_skips_remaining_calls()
    -> Result<(), Box<dyn std::error::Error>> {
        let provider = Arc::new(RecordingProvider {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(VecDeque::from([ChatCompletionResult {
                message: ChatMessage {
                    role: ChatRole::Assistant,
                    content: String::new(),
                    thinking: None,
                    tool_calls: vec![
                        ToolCall {
                            id: "blocking-call".to_owned(),
                            name: "blocking".to_owned(),
                            arguments: Map::new(),
                        },
                        ToolCall {
                            id: "skipped-call".to_owned(),
                            name: "skipped".to_owned(),
                            arguments: Map::new(),
                        },
                    ],
                    tool_call_id: None,
                    tool_name: None,
                },
            }])),
        });
        let started = Arc::new(Notify::new());
        let skipped_executions = Arc::new(AtomicUsize::new(0));
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(BlockingTool {
            started: Arc::clone(&started),
        }));
        tools.register(Arc::new(CountingTool {
            executions: Arc::clone(&skipped_executions),
        }));
        let mut service = service_with_tools(provider, tools);
        let cancellation = CancellationToken::new();
        let execution = service.handle_user_message(
            "Run both tools",
            AgentTurnOptions::default(),
            &cancellation,
        );
        tokio::pin!(execution);
        tokio::select! {
            () = started.notified() => {}
            result = &mut execution => {
                return Err(format!("agent completed before tool cancellation: {result:?}").into());
            }
        }
        cancellation.cancel();
        assert!(matches!(execution.await, Err(AgentError::Cancelled)));
        assert_eq!(skipped_executions.load(Ordering::SeqCst), 0);
        Ok(())
    }

    #[tokio::test]
    async fn session_thinking_override_is_not_sticky() -> Result<(), Box<dyn std::error::Error>> {
        let provider = Arc::new(RecordingProvider {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(VecDeque::from([
                ChatCompletionResult {
                    message: ChatMessage {
                        thinking: Some("reasoning-1".to_owned()),
                        ..ChatMessage::new(ChatRole::Assistant, "first")
                    },
                },
                ChatCompletionResult {
                    message: ChatMessage {
                        thinking: Some("reasoning-2".to_owned()),
                        ..ChatMessage::new(ChatRole::Assistant, "second")
                    },
                },
            ])),
        });
        let mut session = AgentSession::new("recording", None, service(Arc::clone(&provider)));
        session
            .run(
                "deep task",
                AgentTurnOptions {
                    tools: AgentTools::Default,
                    thinking: Some(ThinkingMode::Off),
                },
                &CancellationToken::new(),
            )
            .await?;
        session
            .run(
                "default task",
                AgentTurnOptions::default(),
                &CancellationToken::new(),
            )
            .await?;
        let requests = provider
            .requests
            .lock()
            .map_err(|error| error.to_string())?;
        assert_eq!(requests[0].options.thinking, Some(ThinkingMode::Off));
        assert_eq!(requests[1].options.thinking, None);
        Ok(())
    }

    #[tokio::test]
    async fn copied_session_history_retains_prior_thinking()
    -> Result<(), Box<dyn std::error::Error>> {
        let source_provider = Arc::new(RecordingProvider {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(VecDeque::from([ChatCompletionResult {
                message: ChatMessage {
                    thinking: Some("reasoning-1".to_owned()),
                    ..ChatMessage::new(ChatRole::Assistant, "first")
                },
            }])),
        });
        let mut source = AgentSession::new("recording", None, service(source_provider));
        source
            .run(
                "first",
                AgentTurnOptions::default(),
                &CancellationToken::new(),
            )
            .await?;

        let fork_provider = Arc::new(RecordingProvider {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(VecDeque::from([ChatCompletionResult {
                message: ChatMessage::new(ChatRole::Assistant, "second"),
            }])),
        });
        let mut fork_service = service(Arc::clone(&fork_provider));
        fork_service.restore_history(source.snapshot_history());
        let mut fork = AgentSession::new("recording", None, fork_service);
        fork.run(
            "second",
            AgentTurnOptions::default(),
            &CancellationToken::new(),
        )
        .await?;
        let requests = fork_provider
            .requests
            .lock()
            .map_err(|error| error.to_string())?;
        assert_eq!(
            requests[0].messages[2].thinking.as_deref(),
            Some("reasoning-1")
        );
        Ok(())
    }
}
