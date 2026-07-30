use chrono::{SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use crate::{AgentSessionRecord, Result, StoredAgentConversation, StoredChatMessage};

struct RawConversation {
    id: String,
    project_dir: String,
    agent_name: String,
    provider: String,
    model: String,
    history_json: String,
    created_at: String,
    updated_at: String,
}

pub struct AgentConversationRepository<'connection> {
    connection: &'connection mut Connection,
}

#[allow(clippy::missing_errors_doc)]
impl<'connection> AgentConversationRepository<'connection> {
    #[allow(clippy::redundant_pub_crate)]
    pub(crate) const fn new(connection: &'connection mut Connection) -> Self {
        Self { connection }
    }

    pub fn get(
        &self,
        project_dir: &str,
        agent_name: &str,
    ) -> Result<Option<StoredAgentConversation>> {
        let raw = self
            .connection
            .query_row(
                "SELECT id, project_dir, agent_name, provider, model,
                        history_json, created_at, updated_at
                 FROM agent_conversations
                 WHERE project_dir = ? AND agent_name = ?",
                params![project_dir, agent_name],
                |row| {
                    Ok(RawConversation {
                        id: row.get(0)?,
                        project_dir: row.get(1)?,
                        agent_name: row.get(2)?,
                        provider: row.get(3)?,
                        model: row.get(4)?,
                        history_json: row.get(5)?,
                        created_at: row.get(6)?,
                        updated_at: row.get(7)?,
                    })
                },
            )
            .optional()?;
        raw.map(map_conversation).transpose()
    }

    pub fn save(
        &mut self,
        session: &AgentSessionRecord,
        history: &[StoredChatMessage],
    ) -> Result<()> {
        self.save_at(session, history, &now())
    }

    pub fn save_at(
        &mut self,
        session: &AgentSessionRecord,
        history: &[StoredChatMessage],
        updated_at: &str,
    ) -> Result<()> {
        let non_system_history = history
            .iter()
            .filter(|message| message.role != crate::ChatRole::System)
            .collect::<Vec<_>>();
        self.connection.execute(
            "INSERT INTO agent_conversations (
               id, project_dir, agent_name, mode, provider, model,
               history_json, created_at, updated_at
             ) VALUES (?, ?, ?, 'direct', ?, ?, ?, ?, ?)
             ON CONFLICT(project_dir, agent_name) DO UPDATE SET
               provider = excluded.provider,
               model = excluded.model,
               history_json = excluded.history_json,
               updated_at = excluded.updated_at",
            params![
                session.id,
                session.project_dir,
                session.agent_name,
                session.provider,
                session.model,
                serde_json::to_string(&non_system_history)?,
                session.created_at,
                updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn clear(&mut self, project_dir: &str, agent_name: &str) -> Result<bool> {
        self.clear_at(project_dir, agent_name, &now())
    }

    pub fn clear_at(
        &mut self,
        project_dir: &str,
        agent_name: &str,
        updated_at: &str,
    ) -> Result<bool> {
        let changed = self.connection.execute(
            "UPDATE agent_conversations
             SET history_json = '[]', updated_at = ?
             WHERE project_dir = ? AND agent_name = ?",
            params![updated_at, project_dir, agent_name],
        )?;
        Ok(changed == 1)
    }
}

fn map_conversation(raw: RawConversation) -> Result<StoredAgentConversation> {
    Ok(StoredAgentConversation {
        session: AgentSessionRecord {
            id: raw.id,
            project_dir: raw.project_dir,
            agent_name: raw.agent_name,
            provider: raw.provider,
            model: raw.model,
            created_at: raw.created_at,
            updated_at: raw.updated_at,
        },
        history: serde_json::from_str(&raw.history_json)?,
    })
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}
