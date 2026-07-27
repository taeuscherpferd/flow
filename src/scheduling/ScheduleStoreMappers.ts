import type { SQLOutputValue } from "node:sqlite";
import type {
  ScheduleNotification,
  ScheduleOccurrence,
  ScheduleOccurrenceStatus,
  ScheduleRecord,
  ScheduleStatus,
} from "#src/scheduling/types.js";
import type { JsonValue } from "#src/workflows/types.js";

export function scheduleText(
  row: Record<string, SQLOutputValue>,
  key: string,
): string {
  const value = row[key];
  if (typeof value !== "string") {
    throw new Error(`Invalid schedule database value for "${key}".`);
  }
  return value;
}

export function optionalScheduleText(
  row: Record<string, SQLOutputValue>,
  key: string,
): string | undefined {
  const value = row[key];
  if (value === null) return undefined;
  if (typeof value !== "string") {
    throw new Error(`Invalid schedule database value for "${key}".`);
  }
  return value;
}

export function mapSchedule(
  row: Record<string, SQLOutputValue>,
): ScheduleRecord {
  return {
    id: scheduleText(row, "id"),
    projectDir: scheduleText(row, "project_dir"),
    agentName: scheduleText(row, "agent_name"),
    workflowName: scheduleText(row, "workflow_name"),
    input: JSON.parse(scheduleText(row, "input_json")) as JsonValue,
    cron: scheduleText(row, "cron"),
    timezone: scheduleText(row, "timezone"),
    packageFingerprint: scheduleText(row, "package_fingerprint"),
    status: scheduleText(row, "status") as ScheduleStatus,
    nextRunAt: scheduleText(row, "next_run_at"),
    createdAt: scheduleText(row, "created_at"),
    updatedAt: scheduleText(row, "updated_at"),
  };
}

export function mapOccurrence(
  row: Record<string, SQLOutputValue>,
): ScheduleOccurrence {
  const runId = optionalScheduleText(row, "run_id");
  const result = optionalScheduleText(row, "result_json");
  const error = optionalScheduleText(row, "error");
  return {
    id: scheduleText(row, "id"),
    scheduleId: scheduleText(row, "schedule_id"),
    scheduledFor: scheduleText(row, "scheduled_for"),
    status: scheduleText(row, "status") as ScheduleOccurrenceStatus,
    ...(runId === undefined ? {} : { runId }),
    ...(result === undefined
      ? {}
      : { result: JSON.parse(result) as JsonValue }),
    ...(error === undefined ? {} : { error }),
    createdAt: scheduleText(row, "created_at"),
    updatedAt: scheduleText(row, "updated_at"),
  };
}

export function mapNotification(
  row: Record<string, SQLOutputValue>,
): ScheduleNotification {
  const scheduleId = optionalScheduleText(row, "schedule_id");
  const occurrenceId = optionalScheduleText(row, "occurrence_id");
  return {
    id: scheduleText(row, "id"),
    projectDir: scheduleText(row, "project_dir"),
    agentName: scheduleText(row, "agent_name"),
    ...(scheduleId === undefined ? {} : { scheduleId }),
    ...(occurrenceId === undefined ? {} : { occurrenceId }),
    kind: scheduleText(row, "kind") as ScheduleNotification["kind"],
    message: scheduleText(row, "message"),
    read: row["is_read"] === 1,
    createdAt: scheduleText(row, "created_at"),
  };
}
