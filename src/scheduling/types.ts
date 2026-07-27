import type { JsonValue } from "#src/workflows/types.js";

export type ScheduleStatus =
  | "active"
  | "paused"
  | "needs-reauthorization";

export type ScheduleOccurrenceStatus =
  | "pending"
  | "running"
  | "completed"
  | "failed"
  | "waiting"
  | "skipped"
  | "invalidated";

export interface ScheduleRecord {
  id: string;
  projectDir: string;
  agentName: string;
  workflowName: string;
  input: JsonValue;
  cron: string;
  timezone: string;
  packageFingerprint: string;
  status: ScheduleStatus;
  nextRunAt: string;
  createdAt: string;
  updatedAt: string;
}

export interface ScheduleOccurrence {
  id: string;
  scheduleId: string;
  scheduledFor: string;
  status: ScheduleOccurrenceStatus;
  runId?: string;
  result?: JsonValue;
  error?: string;
  createdAt: string;
  updatedAt: string;
}

export interface ScheduleNotification {
  id: string;
  projectDir: string;
  agentName: string;
  scheduleId?: string;
  occurrenceId?: string;
  kind: "completed" | "failed" | "waiting" | "invalidated";
  message: string;
  read: boolean;
  createdAt: string;
}

export interface CreateScheduleInput {
  projectDir: string;
  agentName: string;
  workflowName: string;
  input: JsonValue;
  cron: string;
  timezone: string;
  packageFingerprint: string;
  now?: Date;
}
