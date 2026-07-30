use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use uuid::Uuid;

use super::mapping::{RawOccurrence, map_occurrence, now, raw_occurrence};
use crate::{
    OccurrenceUpdate, PersistenceError, Result, ScheduleOccurrence, ScheduleOccurrenceStatus,
};

pub struct OccurrenceRepository<'connection> {
    connection: &'connection mut Connection,
}

#[allow(clippy::missing_errors_doc)]
impl<'connection> OccurrenceRepository<'connection> {
    #[allow(clippy::redundant_pub_crate)]
    pub(crate) const fn new(connection: &'connection mut Connection) -> Self {
        Self { connection }
    }

    pub fn claim_due(
        &mut self,
        schedule_id: &str,
        scheduled_for: &str,
        next_run_at: &str,
    ) -> Result<Option<ScheduleOccurrence>> {
        self.claim_due_at(schedule_id, scheduled_for, next_run_at, &now())
    }

    pub fn claim_due_at(
        &mut self,
        schedule_id: &str,
        scheduled_for: &str,
        next_run_at: &str,
        updated_at: &str,
    ) -> Result<Option<ScheduleOccurrence>> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let blocked = has_non_terminal(&transaction, schedule_id)?;
        let status = if blocked {
            ScheduleOccurrenceStatus::Skipped
        } else {
            ScheduleOccurrenceStatus::Pending
        };
        let error = blocked.then_some("An earlier occurrence is still non-terminal.");
        let occurrence_id = create_occurrence(
            &transaction,
            schedule_id,
            scheduled_for,
            status,
            error,
            updated_at,
        )?;
        transaction.execute(
            "UPDATE schedules SET next_run_at = ?, updated_at = ? WHERE id = ?",
            params![next_run_at, updated_at, schedule_id],
        )?;
        transaction.commit()?;
        if status != ScheduleOccurrenceStatus::Pending {
            return Ok(None);
        }
        occurrence_id
            .map(|id| self.get(&id))
            .transpose()
            .map(Option::flatten)
    }

    pub fn create(
        &mut self,
        schedule_id: &str,
        scheduled_for: &str,
        status: ScheduleOccurrenceStatus,
        error: Option<&str>,
    ) -> Result<Option<ScheduleOccurrence>> {
        self.create_at(schedule_id, scheduled_for, status, error, &now())
    }

    pub fn create_at(
        &mut self,
        schedule_id: &str,
        scheduled_for: &str,
        status: ScheduleOccurrenceStatus,
        error: Option<&str>,
        created_at: &str,
    ) -> Result<Option<ScheduleOccurrence>> {
        let id = create_occurrence(
            self.connection,
            schedule_id,
            scheduled_for,
            status,
            error,
            created_at,
        )?;
        id.map(|id| self.get(&id)).transpose().map(Option::flatten)
    }

    pub fn get(&self, id: &str) -> Result<Option<ScheduleOccurrence>> {
        self.connection
            .query_row(
                "SELECT * FROM schedule_occurrences WHERE id = ?",
                [id],
                raw_occurrence,
            )
            .optional()?
            .map(map_occurrence)
            .transpose()
    }

    pub fn list(&self, schedule_id: &str) -> Result<Vec<ScheduleOccurrence>> {
        let mut statement = self.connection.prepare(
            "SELECT * FROM schedule_occurrences
             WHERE schedule_id = ? ORDER BY scheduled_for DESC",
        )?;
        map_rows(statement.query_map([schedule_id], raw_occurrence)?)
    }

    pub fn list_recoverable(&self) -> Result<Vec<ScheduleOccurrence>> {
        let mut statement = self.connection.prepare(
            "SELECT * FROM schedule_occurrences
             WHERE status IN ('pending', 'running')
             ORDER BY created_at",
        )?;
        map_rows(statement.query_map([], raw_occurrence)?)
    }

    pub fn has_non_terminal(&self, schedule_id: &str) -> Result<bool> {
        has_non_terminal(self.connection, schedule_id)
    }

    pub fn update(
        &mut self,
        id: &str,
        status: ScheduleOccurrenceStatus,
        update: &OccurrenceUpdate,
    ) -> Result<bool> {
        self.update_at(id, status, update, &now())
    }

    pub fn update_at(
        &mut self,
        id: &str,
        status: ScheduleOccurrenceStatus,
        update: &OccurrenceUpdate,
        updated_at: &str,
    ) -> Result<bool> {
        let changed = self.connection.execute(
            "UPDATE schedule_occurrences
             SET status = ?, run_id = COALESCE(?, run_id),
                 result_json = ?, error = ?, updated_at = ?
             WHERE id = ?",
            params![
                status.as_str(),
                update.run_id,
                update
                    .result
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()?,
                update.error,
                updated_at,
                id,
            ],
        )?;
        Ok(changed == 1)
    }

    pub fn invalidate_schedule(
        &mut self,
        schedule_id: &str,
        occurrence_id: &str,
        error: &str,
    ) -> Result<bool> {
        let updated_at = now();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let schedule_changed = transaction.execute(
            "UPDATE schedules
             SET status = 'needs-reauthorization', updated_at = ?
             WHERE id = ?",
            params![updated_at, schedule_id],
        )?;
        let occurrence_changed = transaction.execute(
            "UPDATE schedule_occurrences
             SET status = 'invalidated', error = ?, updated_at = ?
             WHERE id = ?",
            params![error, updated_at, occurrence_id],
        )?;
        transaction.execute(
            "INSERT INTO schedule_notifications (
               id, project_dir, agent_name, schedule_id, occurrence_id,
               kind, message, is_read, created_at
             )
             SELECT lower(hex(randomblob(16))), project_dir, agent_name, id, ?,
                    'invalidated', 'Schedule ' || id || ' was invalidated: ' || ?,
                    0, ?
             FROM schedules
             WHERE id = ?",
            params![occurrence_id, error, updated_at, schedule_id],
        )?;
        transaction.commit()?;
        Ok(schedule_changed == 1 && occurrence_changed == 1)
    }
}

fn create_occurrence(
    connection: &Connection,
    schedule_id: &str,
    scheduled_for: &str,
    status: ScheduleOccurrenceStatus,
    error: Option<&str>,
    created_at: &str,
) -> Result<Option<String>> {
    let id = Uuid::new_v4().to_string();
    let changed = connection.execute(
        "INSERT OR IGNORE INTO schedule_occurrences (
           id, schedule_id, scheduled_for, status, error, created_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?)",
        params![
            id,
            schedule_id,
            scheduled_for,
            status.as_str(),
            error,
            created_at,
            created_at
        ],
    )?;
    Ok((changed == 1).then_some(id))
}

fn has_non_terminal(connection: &Connection, schedule_id: &str) -> Result<bool> {
    let found = connection
        .query_row(
            "SELECT 1 FROM schedule_occurrences
             WHERE schedule_id = ? AND status IN ('pending', 'running', 'waiting')
             LIMIT 1",
            [schedule_id],
            |_| Ok(()),
        )
        .optional()?;
    Ok(found.is_some())
}

fn map_rows<F>(rows: rusqlite::MappedRows<'_, F>) -> Result<Vec<ScheduleOccurrence>>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<RawOccurrence>,
{
    rows.map(|row| row.map_err(PersistenceError::from).and_then(map_occurrence))
        .collect()
}
