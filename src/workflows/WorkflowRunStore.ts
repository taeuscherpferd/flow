import { mkdirSync } from "node:fs";
import path from "node:path";
import { DatabaseSync, type SQLOutputValue } from "node:sqlite";
import type {
  JsonValue,
  WorkflowPresentation,
  WorkflowRunDetails,
  WorkflowRunStatus,
  WorkflowRunSummary,
} from "./types.js";

interface StoredRunRow {
  id: string;
  workflow_name: string;
  project_dir: string;
  source_entry_path: string;
  source_fingerprint: string;
  status: WorkflowRunStatus;
  presentation: WorkflowPresentation;
  input_json: string;
  output_json: string | null;
  error: string | null;
  created_at: string;
  updated_at: string;
}

interface StoredStepRow {
  key: string;
  kind: WorkflowStepKind;
  state: WorkflowStepState;
  input_json: string | null;
  output_json: string | null;
}

export interface CreateWorkflowRun {
  id: string;
  workflowName: string;
  projectDir: string;
  sourceEntryPath: string;
  sourceFingerprint: string;
  presentation: WorkflowPresentation;
  input: JsonValue;
}

export type WorkflowStepKind = "checkpoint" | "effect" | "human";
export type WorkflowStepState = "started" | "completed";

export interface WorkflowStep {
  key: string;
  kind: WorkflowStepKind;
  state: WorkflowStepState;
  input?: JsonValue;
  output?: JsonValue;
}

function mapSummary(row: StoredRunRow): WorkflowRunSummary {
  const summary: WorkflowRunSummary = {
    id: row.id,
    workflowName: row.workflow_name,
    projectDir: row.project_dir,
    status: row.status,
    presentation: row.presentation,
    createdAt: row.created_at,
    updatedAt: row.updated_at,
  };
  if (row.error !== null) summary.error = row.error;
  return summary;
}

function requiredString(
  row: Record<string, SQLOutputValue>,
  key: string,
): string {
  const value = row[key];
  if (typeof value !== "string") {
    throw new Error(`Invalid workflow database value for "${key}".`);
  }
  return value;
}

function optionalString(
  row: Record<string, SQLOutputValue>,
  key: string,
): string | null {
  const value = row[key];
  if (value === null) return null;
  if (typeof value !== "string") {
    throw new Error(`Invalid workflow database value for "${key}".`);
  }
  return value;
}

function mapStoredRow(row: Record<string, SQLOutputValue>): StoredRunRow {
  return {
    id: requiredString(row, "id"),
    workflow_name: requiredString(row, "workflow_name"),
    project_dir: requiredString(row, "project_dir"),
    source_entry_path: requiredString(row, "source_entry_path"),
    source_fingerprint: requiredString(row, "source_fingerprint"),
    status: requiredString(row, "status") as WorkflowRunStatus,
    presentation: requiredString(
      row,
      "presentation",
    ) as WorkflowPresentation,
    input_json: requiredString(row, "input_json"),
    output_json: optionalString(row, "output_json"),
    error: optionalString(row, "error"),
    created_at: requiredString(row, "created_at"),
    updated_at: requiredString(row, "updated_at"),
  };
}

function mapDetails(row: StoredRunRow): WorkflowRunDetails {
  const details: WorkflowRunDetails = {
    ...mapSummary(row),
    input: JSON.parse(row.input_json) as JsonValue,
    sourceFingerprint: row.source_fingerprint,
  };
  if (row.output_json !== null) {
    details.output = JSON.parse(row.output_json) as JsonValue;
  }
  return details;
}

export class WorkflowRunStore {
  private readonly database: DatabaseSync;

  constructor(globalDir: string) {
    mkdirSync(globalDir, { recursive: true });
    this.database = new DatabaseSync(path.join(globalDir, "runs.sqlite"));
    this.database.exec(`
      PRAGMA journal_mode = WAL;
      PRAGMA foreign_keys = ON;

      CREATE TABLE IF NOT EXISTS workflow_runs (
        id TEXT PRIMARY KEY,
        workflow_name TEXT NOT NULL,
        project_dir TEXT NOT NULL,
        source_entry_path TEXT NOT NULL,
        source_fingerprint TEXT NOT NULL,
        status TEXT NOT NULL,
        presentation TEXT NOT NULL,
        input_json TEXT NOT NULL,
        output_json TEXT,
        parent_run_id TEXT,
        depth INTEGER NOT NULL,
        error TEXT,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        FOREIGN KEY(parent_run_id) REFERENCES workflow_runs(id)
      );

      CREATE INDEX IF NOT EXISTS workflow_runs_project_updated
      ON workflow_runs(project_dir, updated_at DESC);

      CREATE TABLE IF NOT EXISTS workflow_steps (
        run_id TEXT NOT NULL,
        key TEXT NOT NULL,
        kind TEXT NOT NULL,
        state TEXT NOT NULL,
        input_json TEXT,
        output_json TEXT,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        PRIMARY KEY(run_id, key),
        FOREIGN KEY(run_id) REFERENCES workflow_runs(id) ON DELETE CASCADE
      );
    `);
  }

  createRun(input: CreateWorkflowRun): WorkflowRunDetails {
    const now = new Date().toISOString();
    this.database
      .prepare(
        `INSERT INTO workflow_runs (
          id, workflow_name, project_dir, source_entry_path,
          source_fingerprint, status, presentation, input_json,
          parent_run_id, depth, created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, 'queued', ?, ?, ?, ?, ?, ?)`,
      )
      .run(
        input.id,
        input.workflowName,
        input.projectDir,
        input.sourceEntryPath,
        input.sourceFingerprint,
        input.presentation,
        JSON.stringify(input.input),
        null,
        0,
        now,
        now,
      );
    return this.getRun(input.id)!;
  }

  getRun(id: string): WorkflowRunDetails | undefined {
    const row = this.database
      .prepare("SELECT * FROM workflow_runs WHERE id = ?")
      .get(id);
    return row ? mapDetails(mapStoredRow(row)) : undefined;
  }

  listRuns(projectDir: string, limit = 50): WorkflowRunSummary[] {
    const rows = this.database
      .prepare(
        `SELECT * FROM workflow_runs
         WHERE project_dir = ?
         ORDER BY updated_at DESC
         LIMIT ?`,
      )
      .all(projectDir, limit);
    return rows.map((row) => mapSummary(mapStoredRow(row)));
  }

  updateStatus(
    id: string,
    status: WorkflowRunStatus,
    error?: string,
  ): void {
    this.database
      .prepare(
        `UPDATE workflow_runs
         SET status = ?, error = ?, updated_at = ?
         WHERE id = ?`,
      )
      .run(status, error ?? null, new Date().toISOString(), id);
  }

  complete(
    id: string,
    output: JsonValue,
    presentation: WorkflowPresentation,
  ): boolean {
    const result = this.database
      .prepare(
        `UPDATE workflow_runs
         SET status = 'completed', output_json = ?, presentation = ?,
             error = NULL, updated_at = ?
         WHERE id = ? AND status = 'running'`,
      )
      .run(
        JSON.stringify(output),
        presentation,
        new Date().toISOString(),
        id,
      );
    return result.changes === 1 || result.changes === 1n;
  }

  transitionToRunning(id: string): boolean {
    const result = this.database
      .prepare(
        `UPDATE workflow_runs
         SET status = 'running', error = NULL, updated_at = ?
         WHERE id = ? AND status IN ('queued', 'waiting', 'interrupted')`,
      )
      .run(new Date().toISOString(), id);
    return result.changes === 1 || result.changes === 1n;
  }

  transitionRunningStatus(
    id: string,
    status: WorkflowRunStatus,
    error?: string,
  ): boolean {
    const result = this.database
      .prepare(
        `UPDATE workflow_runs
         SET status = ?, error = ?, updated_at = ?
         WHERE id = ? AND status = 'running'`,
      )
      .run(status, error ?? null, new Date().toISOString(), id);
    return result.changes === 1 || result.changes === 1n;
  }

  getStep(runId: string, key: string): WorkflowStep | undefined {
    const row = this.database
      .prepare(
        `SELECT key, kind, state, input_json, output_json
         FROM workflow_steps
         WHERE run_id = ? AND key = ?`,
      )
      .get(runId, key) as StoredStepRow | undefined;
    if (!row) return undefined;
    const step: WorkflowStep = {
      key: row.key,
      kind: row.kind,
      state: row.state,
    };
    if (row.input_json !== null) {
      step.input = JSON.parse(row.input_json) as JsonValue;
    }
    if (row.output_json !== null) {
      step.output = JSON.parse(row.output_json) as JsonValue;
    }
    return step;
  }

  startStep(
    runId: string,
    key: string,
    kind: WorkflowStepKind,
    input?: JsonValue,
  ): void {
    const now = new Date().toISOString();
    this.database
      .prepare(
        `INSERT INTO workflow_steps (
          run_id, key, kind, state, input_json, created_at, updated_at
        ) VALUES (?, ?, ?, 'started', ?, ?, ?)`,
      )
      .run(
        runId,
        key,
        kind,
        input === undefined ? null : JSON.stringify(input),
        now,
        now,
      );
  }

  completeStep(
    runId: string,
    key: string,
    output: JsonValue,
  ): void {
    this.database
      .prepare(
        `UPDATE workflow_steps
         SET state = 'completed', output_json = ?, updated_at = ?
         WHERE run_id = ? AND key = ?`,
      )
      .run(
        JSON.stringify(output),
        new Date().toISOString(),
        runId,
        key,
      );
  }

  close(): void {
    this.database.close();
  }
}
