import type { DatabaseSync } from "node:sqlite";

export function ensureScheduleRunTriggers(database: DatabaseSync): void {
  const hasRuns = database
    .prepare(
      `SELECT 1 FROM sqlite_master
       WHERE type = 'table' AND name = 'workflow_runs'`,
    )
    .get();
  const hasOccurrences = database
    .prepare(
      `SELECT 1 FROM sqlite_master
       WHERE type = 'table' AND name = 'schedule_occurrences'`,
    )
    .get();
  if (!hasRuns || !hasOccurrences) return;

  database.exec(`
    CREATE TRIGGER IF NOT EXISTS schedule_occurrence_run_status
    AFTER UPDATE OF status, output_json, error ON workflow_runs
    WHEN json_extract(NEW.trigger_json, '$.type') = 'schedule'
    BEGIN
      UPDATE schedule_occurrences
      SET status = CASE
            WHEN NEW.status = 'completed' THEN 'completed'
            WHEN NEW.status = 'waiting' THEN 'waiting'
            WHEN NEW.status IN ('failed', 'cancelled', 'version-mismatch')
              THEN 'failed'
            ELSE 'running'
          END,
          result_json = NEW.output_json,
          error = NEW.error,
          updated_at = NEW.updated_at
      WHERE run_id = NEW.id;

      INSERT INTO schedule_notifications (
        id, project_dir, agent_name, schedule_id, occurrence_id,
        kind, message, is_read, created_at
      )
      SELECT
        lower(hex(randomblob(16))),
        NEW.project_dir,
        NEW.agent_name,
        json_extract(NEW.trigger_json, '$.scheduleId'),
        occurrence.id,
        CASE
          WHEN NEW.status = 'completed' THEN 'completed'
          WHEN NEW.status = 'waiting' THEN 'waiting'
          ELSE 'failed'
        END,
        'Scheduled workflow ' || NEW.agent_name || '/' ||
          NEW.workflow_name || ' ' || NEW.status || ' (' || NEW.id || ').',
        0,
        NEW.updated_at
      FROM schedule_occurrences AS occurrence
      WHERE occurrence.run_id = NEW.id
        AND NEW.status IN (
          'completed', 'waiting', 'failed', 'cancelled', 'version-mismatch'
        )
        AND OLD.status <> NEW.status;
    END;
  `);
}
