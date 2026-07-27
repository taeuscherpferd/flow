import type { AgentSessionRecord } from "#src/agents/types.js";
import type { ChatMessage } from "#src/providers/types.js";
import { configureSqliteDatabase } from "#src/services/SqliteDatabase.js";
import { mkdirSync } from "node:fs";
import path from "node:path";
import { DatabaseSync, type SQLOutputValue } from "node:sqlite";

interface ConversationRow {
  id: string;
  project_dir: string;
  agent_name: string;
  mode: "direct";
  provider: string;
  model: string;
  history_json: string;
  created_at: string;
  updated_at: string;
}

function stringValue(row: Record<string, SQLOutputValue>, key: string): string {
  const value = row[key];
  if (typeof value !== "string") {
    throw new Error(`Invalid conversation database value for "${key}".`);
  }
  return value;
}

function mapRow(row: Record<string, SQLOutputValue>): ConversationRow {
  return {
    id: stringValue(row, "id"),
    project_dir: stringValue(row, "project_dir"),
    agent_name: stringValue(row, "agent_name"),
    mode: "direct",
    provider: stringValue(row, "provider"),
    model: stringValue(row, "model"),
    history_json: stringValue(row, "history_json"),
    created_at: stringValue(row, "created_at"),
    updated_at: stringValue(row, "updated_at"),
  };
}

export interface StoredAgentConversation {
  session: AgentSessionRecord;
  history: ChatMessage[];
}

export class AgentConversationStore {
  private readonly database: DatabaseSync;

  constructor(globalDir: string) {
    mkdirSync(globalDir, { recursive: true });
    this.database = new DatabaseSync(path.join(globalDir, "runs.sqlite"));
    configureSqliteDatabase(this.database);
    this.database.exec(`
      CREATE TABLE IF NOT EXISTS agent_conversations (
        id TEXT PRIMARY KEY,
        project_dir TEXT NOT NULL,
        agent_name TEXT NOT NULL,
        mode TEXT NOT NULL,
        provider TEXT NOT NULL,
        model TEXT NOT NULL,
        history_json TEXT NOT NULL,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        UNIQUE(project_dir, agent_name)
      );
    `);
  }

  get(
    projectDir: string,
    agentName: string,
  ): StoredAgentConversation | undefined {
    const raw = this.database
      .prepare(
        `SELECT * FROM agent_conversations
         WHERE project_dir = ? AND agent_name = ?`,
      )
      .get(projectDir, agentName);
    if (!raw) return undefined;
    const row = mapRow(raw);
    return {
      session: {
        id: row.id,
        projectDir: row.project_dir,
        agentName: row.agent_name,
        mode: "direct",
        provider: row.provider,
        model: row.model,
        createdAt: row.created_at,
        updatedAt: row.updated_at,
      },
      history: JSON.parse(row.history_json) as ChatMessage[],
    };
  }

  save(session: AgentSessionRecord, history: ChatMessage[]): void {
    const now = new Date().toISOString();
    const nonSystemHistory = history.filter(
      (message) => message.role !== "system",
    );
    this.database
      .prepare(
        `INSERT INTO agent_conversations (
          id, project_dir, agent_name, mode, provider, model,
          history_json, created_at, updated_at
        ) VALUES (?, ?, ?, 'direct', ?, ?, ?, ?, ?)
        ON CONFLICT(project_dir, agent_name) DO UPDATE SET
          provider = excluded.provider,
          model = excluded.model,
          history_json = excluded.history_json,
          updated_at = excluded.updated_at`,
      )
      .run(
        session.id,
        session.projectDir,
        session.agentName,
        session.provider,
        session.model,
        JSON.stringify(nonSystemHistory),
        session.createdAt,
        now,
      );
  }

  clear(projectDir: string, agentName: string): void {
    this.database
      .prepare(
        `UPDATE agent_conversations
         SET history_json = '[]', updated_at = ?
         WHERE project_dir = ? AND agent_name = ?`,
      )
      .run(new Date().toISOString(), projectDir, agentName);
  }

  close(): void {
    this.database.close();
  }
}
