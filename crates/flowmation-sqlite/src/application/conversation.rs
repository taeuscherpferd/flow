use std::path::PathBuf;

use flowmation_application::{
    ChatMessage, ChatRole as ApplicationChatRole, ConversationRepository, StoredConversation,
    ToolCall,
};
use flowmation_domain::agent::{AgentExecutionMode, AgentSessionRecord};
use flowmation_domain::ids::AgentSessionId;

use super::SqliteApplicationRepository;
use crate::{
    AgentSessionRecord as PersistenceSession, ChatRole, StoredAgentConversation, StoredChatMessage,
    StoredToolCall,
};

impl ConversationRepository for SqliteApplicationRepository {
    fn get(
        &self,
        project_dir: &str,
        agent_name: &str,
    ) -> Result<Option<StoredConversation>, String> {
        self.database()?
            .agent_conversations()
            .get(project_dir, agent_name)
            .map_err(|error| error.to_string())?
            .map(to_application_conversation)
            .transpose()
    }

    fn save(&self, conversation: &StoredConversation) -> Result<(), String> {
        let session = to_persistence_session(&conversation.session)?;
        let history: Vec<StoredChatMessage> = conversation
            .history
            .iter()
            .map(to_persistence_message)
            .collect();
        self.database()?
            .agent_conversations()
            .save(&session, &history)
            .map_err(|error| error.to_string())
    }

    fn clear(&self, project_dir: &str, agent_name: &str) -> Result<(), String> {
        self.database()?
            .agent_conversations()
            .clear(project_dir, agent_name)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

fn to_application_conversation(
    stored: StoredAgentConversation,
) -> Result<StoredConversation, String> {
    Ok(StoredConversation {
        session: AgentSessionRecord {
            id: AgentSessionId::new(stored.session.id).map_err(|error| error.to_string())?,
            project_dir: PathBuf::from(stored.session.project_dir),
            agent_name: stored.session.agent_name,
            mode: AgentExecutionMode::Direct,
            provider: stored.session.provider,
            model: stored.session.model,
            created_at: stored.session.created_at,
            updated_at: stored.session.updated_at,
        },
        history: stored
            .history
            .into_iter()
            .map(to_application_message)
            .collect(),
    })
}

fn to_persistence_session(session: &AgentSessionRecord) -> Result<PersistenceSession, String> {
    Ok(PersistenceSession {
        id: session.id.to_string(),
        project_dir: path_text(&session.project_dir)?,
        agent_name: session.agent_name.clone(),
        provider: session.provider.clone(),
        model: session.model.clone(),
        created_at: session.created_at.clone(),
        updated_at: session.updated_at.clone(),
    })
}

fn to_application_message(message: StoredChatMessage) -> ChatMessage {
    ChatMessage {
        role: match message.role {
            ChatRole::System => ApplicationChatRole::System,
            ChatRole::User => ApplicationChatRole::User,
            ChatRole::Assistant => ApplicationChatRole::Assistant,
            ChatRole::Tool => ApplicationChatRole::Tool,
        },
        content: message.content,
        thinking: message.thinking,
        tool_calls: message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .map(|call| ToolCall {
                id: call.id,
                name: call.name,
                arguments: call.arguments,
            })
            .collect(),
        tool_call_id: message.tool_call_id,
        tool_name: message.tool_name,
    }
}

fn to_persistence_message(message: &ChatMessage) -> StoredChatMessage {
    StoredChatMessage {
        role: match message.role {
            ApplicationChatRole::System => ChatRole::System,
            ApplicationChatRole::User => ChatRole::User,
            ApplicationChatRole::Assistant => ChatRole::Assistant,
            ApplicationChatRole::Tool => ChatRole::Tool,
        },
        content: message.content.clone(),
        thinking: message.thinking.clone(),
        tool_calls: (!message.tool_calls.is_empty()).then(|| {
            message
                .tool_calls
                .iter()
                .map(|call| StoredToolCall {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    arguments: call.arguments.clone(),
                })
                .collect()
        }),
        tool_call_id: message.tool_call_id.clone(),
        tool_name: message.tool_name.clone(),
    }
}

fn path_text(path: &std::path::Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("Path is not valid UTF-8: {}", path.display()))
}
