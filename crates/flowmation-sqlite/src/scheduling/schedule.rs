use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use super::mapping::{RawSchedule, map_schedule, now, raw_schedule};
use crate::{CreateSchedule, PersistenceError, Result, ScheduleRecord, ScheduleStatus};

pub struct ScheduleRepository<'connection> {
    connection: &'connection mut Connection,
}

#[allow(clippy::missing_errors_doc)]
impl<'connection> ScheduleRepository<'connection> {
    #[allow(clippy::redundant_pub_crate)]
    pub(crate) const fn new(connection: &'connection mut Connection) -> Self {
        Self { connection }
    }

    pub fn create(&mut self, input: &CreateSchedule) -> Result<ScheduleRecord> {
        let created_at = input.now.clone().unwrap_or_else(now);
        let id = input
            .id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        self.connection.execute(
            "INSERT INTO schedules (
               id, project_dir, agent_name, workflow_name, input_json, cron,
               timezone, package_fingerprint, status, next_run_at, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'active', ?, ?, ?)",
            params![
                id,
                input.project_dir,
                input.agent_name,
                input.workflow_name,
                serde_json::to_string(&input.input)?,
                input.cron,
                input.timezone,
                input.package_fingerprint,
                input.next_run_at,
                created_at,
                created_at,
            ],
        )?;
        self.get(&id)?.ok_or(PersistenceError::InvalidValue {
            field: "schedules.id",
            value: id,
        })
    }

    pub fn get(&self, id: &str) -> Result<Option<ScheduleRecord>> {
        self.connection
            .query_row("SELECT * FROM schedules WHERE id = ?", [id], raw_schedule)
            .optional()?
            .map(map_schedule)
            .transpose()
    }

    pub fn list(&self, project_dir: &str) -> Result<Vec<ScheduleRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT * FROM schedules
             WHERE project_dir = ?
             ORDER BY created_at DESC",
        )?;
        map_rows(statement.query_map([project_dir], raw_schedule)?)
    }

    pub fn list_due(&self, at: &str) -> Result<Vec<ScheduleRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT * FROM schedules
             WHERE status = 'active' AND next_run_at <= ?
             ORDER BY next_run_at",
        )?;
        map_rows(statement.query_map([at], raw_schedule)?)
    }

    pub fn set_status(&mut self, id: &str, status: ScheduleStatus) -> Result<bool> {
        self.set_status_at(id, status, &now())
    }

    pub fn set_status_at(
        &mut self,
        id: &str,
        status: ScheduleStatus,
        updated_at: &str,
    ) -> Result<bool> {
        let changed = self.connection.execute(
            "UPDATE schedules SET status = ?, updated_at = ? WHERE id = ?",
            params![status.as_str(), updated_at, id],
        )?;
        Ok(changed == 1)
    }

    pub fn reauthorize(
        &mut self,
        id: &str,
        package_fingerprint: &str,
        next_run_at: &str,
        expected_updated_at: Option<&str>,
    ) -> Result<bool> {
        self.reauthorize_at(
            id,
            package_fingerprint,
            next_run_at,
            expected_updated_at,
            &now(),
        )
    }

    pub fn reauthorize_at(
        &mut self,
        id: &str,
        package_fingerprint: &str,
        next_run_at: &str,
        expected_updated_at: Option<&str>,
        updated_at: &str,
    ) -> Result<bool> {
        let changed = match expected_updated_at {
            Some(expected) => self.connection.execute(
                "UPDATE schedules
                 SET package_fingerprint = ?, status = 'active',
                     next_run_at = ?, updated_at = ?
                 WHERE id = ? AND updated_at = ?",
                params![package_fingerprint, next_run_at, updated_at, id, expected],
            )?,
            None => self.connection.execute(
                "UPDATE schedules
                 SET package_fingerprint = ?, status = 'active',
                     next_run_at = ?, updated_at = ?
                 WHERE id = ?",
                params![package_fingerprint, next_run_at, updated_at, id],
            )?,
        };
        Ok(changed == 1)
    }

    pub fn update_next_run(&mut self, id: &str, next_run_at: &str) -> Result<bool> {
        self.update_next_run_at(id, next_run_at, &now())
    }

    pub fn update_next_run_at(
        &mut self,
        id: &str,
        next_run_at: &str,
        updated_at: &str,
    ) -> Result<bool> {
        let changed = self.connection.execute(
            "UPDATE schedules SET next_run_at = ?, updated_at = ? WHERE id = ?",
            params![next_run_at, updated_at, id],
        )?;
        Ok(changed == 1)
    }

    pub fn delete(&mut self, id: &str) -> Result<bool> {
        Ok(self
            .connection
            .execute("DELETE FROM schedules WHERE id = ?", [id])?
            == 1)
    }
}

fn map_rows<F>(rows: rusqlite::MappedRows<'_, F>) -> Result<Vec<ScheduleRecord>>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<RawSchedule>,
{
    rows.map(|row| row.map_err(PersistenceError::from).and_then(map_schedule))
        .collect()
}
