import { randomUUID } from "node:crypto";
import { mkdirSync } from "node:fs";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { configureSqliteDatabase } from "#src/services/SqliteDatabase.js";
import type {
  CreateScheduleInput,
  ScheduleNotification,
  ScheduleOccurrence,
  ScheduleOccurrenceStatus,
  ScheduleRecord,
  ScheduleStatus,
} from "#src/scheduling/types.js";
import type { JsonValue } from "#src/workflows/types.js";
import { ensureScheduleRunTriggers } from "#src/scheduling/ScheduleDatabaseSchema.js";
import {
  mapNotification,
  mapOccurrence,
  mapSchedule,
  scheduleText,
} from "#src/scheduling/ScheduleStoreMappers.js";

export class ScheduleStore {
  private readonly database: DatabaseSync;

  constructor(globalDir: string) {
    mkdirSync(globalDir, { recursive: true });
    this.database = new DatabaseSync(path.join(globalDir, "runs.sqlite"));
    configureSqliteDatabase(this.database);
    this.database.exec(`
      CREATE TABLE IF NOT EXISTS schedules (
        id TEXT PRIMARY KEY,
        project_dir TEXT NOT NULL,
        agent_name TEXT NOT NULL,
        workflow_name TEXT NOT NULL,
        input_json TEXT NOT NULL,
        cron TEXT NOT NULL,
        timezone TEXT NOT NULL,
        package_fingerprint TEXT NOT NULL,
        status TEXT NOT NULL,
        next_run_at TEXT NOT NULL,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
      );
      CREATE INDEX IF NOT EXISTS schedules_due
      ON schedules(status, next_run_at);

      CREATE TABLE IF NOT EXISTS schedule_occurrences (
        id TEXT PRIMARY KEY,
        schedule_id TEXT NOT NULL,
        scheduled_for TEXT NOT NULL,
        status TEXT NOT NULL,
        run_id TEXT,
        result_json TEXT,
        error TEXT,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        UNIQUE(schedule_id, scheduled_for),
        FOREIGN KEY(schedule_id) REFERENCES schedules(id) ON DELETE CASCADE
      );

      CREATE TABLE IF NOT EXISTS schedule_worker_leases (
        lease_key TEXT PRIMARY KEY,
        owner_id TEXT NOT NULL,
        expires_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
      );

      CREATE TABLE IF NOT EXISTS schedule_notifications (
        id TEXT PRIMARY KEY,
        project_dir TEXT NOT NULL,
        agent_name TEXT NOT NULL,
        schedule_id TEXT,
        occurrence_id TEXT,
        kind TEXT NOT NULL,
        message TEXT NOT NULL,
        is_read INTEGER NOT NULL DEFAULT 0,
        created_at TEXT NOT NULL
      );
    `);
    ensureScheduleRunTriggers(this.database);
  }

  create(input: CreateScheduleInput, nextRunAt: Date): ScheduleRecord {
    const id = randomUUID();
    const now = (input.now ?? new Date()).toISOString();
    this.database
      .prepare(
        `INSERT INTO schedules (
          id, project_dir, agent_name, workflow_name, input_json, cron,
          timezone, package_fingerprint, status, next_run_at, created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'active', ?, ?, ?)`,
      )
      .run(
        id,
        input.projectDir,
        input.agentName,
        input.workflowName,
        JSON.stringify(input.input),
        input.cron,
        input.timezone,
        input.packageFingerprint,
        nextRunAt.toISOString(),
        now,
        now,
      );
    return this.get(id)!;
  }

  get(id: string): ScheduleRecord | undefined {
    const row = this.database.prepare("SELECT * FROM schedules WHERE id = ?").get(id);
    return row ? mapSchedule(row) : undefined;
  }

  list(projectDir: string): ScheduleRecord[] {
    return this.database
      .prepare(
        `SELECT * FROM schedules WHERE project_dir = ?
         ORDER BY created_at DESC`,
      )
      .all(projectDir)
      .map(mapSchedule);
  }

  listDue(now: Date): ScheduleRecord[] {
    return this.database
      .prepare(
        `SELECT * FROM schedules
         WHERE status = 'active' AND next_run_at <= ?
         ORDER BY next_run_at`,
      )
      .all(now.toISOString())
      .map(mapSchedule);
  }

  setStatus(id: string, status: ScheduleStatus): boolean {
    const result = this.database
      .prepare(
        `UPDATE schedules SET status = ?, updated_at = ? WHERE id = ?`,
      )
      .run(status, new Date().toISOString(), id);
    return result.changes === 1 || result.changes === 1n;
  }

  reauthorize(
    id: string,
    packageFingerprint: string,
    nextRunAt: Date,
    expectedUpdatedAt?: string,
  ): boolean {
    const statement =
      expectedUpdatedAt === undefined
        ? this.database.prepare(
            `UPDATE schedules
             SET package_fingerprint = ?, status = 'active',
                 next_run_at = ?, updated_at = ?
             WHERE id = ?`,
          )
        : this.database.prepare(
            `UPDATE schedules
             SET package_fingerprint = ?, status = 'active',
                 next_run_at = ?, updated_at = ?
             WHERE id = ? AND updated_at = ?`,
          );
    const values = [
      packageFingerprint,
      nextRunAt.toISOString(),
      new Date().toISOString(),
      id,
    ];
    const result =
      expectedUpdatedAt === undefined
        ? statement.run(...values)
        : statement.run(...values, expectedUpdatedAt);
    return result.changes === 1 || result.changes === 1n;
  }

  updateNextRun(id: string, nextRunAt: Date): void {
    this.database
      .prepare(
        `UPDATE schedules SET next_run_at = ?, updated_at = ? WHERE id = ?`,
      )
      .run(nextRunAt.toISOString(), new Date().toISOString(), id);
  }

  claimDueOccurrence(
    scheduleId: string,
    scheduledFor: string,
    nextRunAt: Date,
  ): ScheduleOccurrence | undefined {
    this.database.exec("BEGIN IMMEDIATE");
    try {
      const blocked = this.hasNonTerminalOccurrence(scheduleId);
      const occurrence = this.createOccurrence(
        scheduleId,
        scheduledFor,
        blocked ? "skipped" : "pending",
        blocked ? "An earlier occurrence is still non-terminal." : undefined,
      );
      this.updateNextRun(scheduleId, nextRunAt);
      this.database.exec("COMMIT");
      return occurrence?.status === "pending" ? occurrence : undefined;
    } catch (error) {
      this.database.exec("ROLLBACK");
      throw error;
    }
  }

  delete(id: string): boolean {
    const result = this.database.prepare("DELETE FROM schedules WHERE id = ?").run(id);
    return result.changes === 1 || result.changes === 1n;
  }

  createOccurrence(
    scheduleId: string,
    scheduledFor: string,
    status: ScheduleOccurrenceStatus = "pending",
    error?: string,
  ): ScheduleOccurrence | undefined {
    const id = randomUUID();
    const now = new Date().toISOString();
    const result = this.database
      .prepare(
        `INSERT OR IGNORE INTO schedule_occurrences (
          id, schedule_id, scheduled_for, status, error, created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?)`,
      )
      .run(id, scheduleId, scheduledFor, status, error ?? null, now, now);
    if (result.changes !== 1 && result.changes !== 1n) return undefined;
    return this.getOccurrence(id);
  }

  getOccurrence(id: string): ScheduleOccurrence | undefined {
    const row = this.database
      .prepare("SELECT * FROM schedule_occurrences WHERE id = ?")
      .get(id);
    return row ? mapOccurrence(row) : undefined;
  }

  listOccurrences(scheduleId: string): ScheduleOccurrence[] {
    return this.database
      .prepare(
        `SELECT * FROM schedule_occurrences
         WHERE schedule_id = ? ORDER BY scheduled_for DESC`,
      )
      .all(scheduleId)
      .map(mapOccurrence);
  }

  listRecoverableOccurrences(): ScheduleOccurrence[] {
    return this.database
      .prepare(
        `SELECT * FROM schedule_occurrences
         WHERE status IN ('pending', 'running')
         ORDER BY created_at`,
      )
      .all()
      .map(mapOccurrence);
  }

  hasNonTerminalOccurrence(scheduleId: string): boolean {
    const row = this.database
      .prepare(
        `SELECT 1 FROM schedule_occurrences
         WHERE schedule_id = ? AND status IN ('pending', 'running', 'waiting')
         LIMIT 1`,
      )
      .get(scheduleId);
    return row !== undefined;
  }

  updateOccurrence(
    id: string,
    status: ScheduleOccurrenceStatus,
    options: { runId?: string; result?: JsonValue; error?: string } = {},
  ): void {
    this.database
      .prepare(
        `UPDATE schedule_occurrences
         SET status = ?, run_id = COALESCE(?, run_id),
             result_json = ?, error = ?, updated_at = ?
         WHERE id = ?`,
      )
      .run(
        status,
        options.runId ?? null,
        options.result === undefined ? null : JSON.stringify(options.result),
        options.error ?? null,
        new Date().toISOString(),
        id,
      );
  }

  acquireLease(
    leaseKey: string,
    ownerId: string,
    now: Date,
    leaseMs: number,
  ): boolean {
    const expiresAt = new Date(now.getTime() + leaseMs).toISOString();
    this.database.exec("BEGIN IMMEDIATE");
    try {
      const existing = this.database
        .prepare(
          `SELECT owner_id, expires_at FROM schedule_worker_leases
           WHERE lease_key = ?`,
        )
        .get(leaseKey);
      if (
        existing &&
        scheduleText(existing, "owner_id") !== ownerId &&
        scheduleText(existing, "expires_at") > now.toISOString()
      ) {
        this.database.exec("ROLLBACK");
        return false;
      }
      this.database
        .prepare(
          `INSERT INTO schedule_worker_leases (
            lease_key, owner_id, expires_at, updated_at
          ) VALUES (?, ?, ?, ?)
          ON CONFLICT(lease_key) DO UPDATE SET
            owner_id = excluded.owner_id,
            expires_at = excluded.expires_at,
            updated_at = excluded.updated_at`,
        )
        .run(leaseKey, ownerId, expiresAt, now.toISOString());
      this.database.exec("COMMIT");
      return true;
    } catch (error) {
      this.database.exec("ROLLBACK");
      throw error;
    }
  }

  notify(
    schedule: ScheduleRecord,
    kind: ScheduleNotification["kind"],
    message: string,
    occurrenceId?: string,
  ): void {
    this.database
      .prepare(
        `INSERT INTO schedule_notifications (
          id, project_dir, agent_name, schedule_id, occurrence_id,
          kind, message, created_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
      )
      .run(
        randomUUID(),
        schedule.projectDir,
        schedule.agentName,
        schedule.id,
        occurrenceId ?? null,
        kind,
        message,
        new Date().toISOString(),
      );
  }

  unread(projectDir: string): ScheduleNotification[] {
    return this.database
      .prepare(
        `SELECT * FROM schedule_notifications
         WHERE project_dir = ? AND is_read = 0 ORDER BY created_at`,
      )
      .all(projectDir)
      .map(mapNotification);
  }

  markNotificationsRead(projectDir: string): void {
    this.database
      .prepare(
        `UPDATE schedule_notifications SET is_read = 1
         WHERE project_dir = ? AND is_read = 0`,
      )
      .run(projectDir);
  }

  close(): void {
    this.database.close();
  }
}
