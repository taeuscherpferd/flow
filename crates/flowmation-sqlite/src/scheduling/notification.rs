use rusqlite::{Connection, params};
use uuid::Uuid;

use super::mapping::{map_notification, now, raw_notification};
use crate::{
    PersistenceError, Result, ScheduleNotification, ScheduleNotificationKind, ScheduleRecord,
};

pub struct NotificationRepository<'connection> {
    connection: &'connection mut Connection,
}

#[allow(clippy::missing_errors_doc)]
impl<'connection> NotificationRepository<'connection> {
    #[allow(clippy::redundant_pub_crate)]
    pub(crate) const fn new(connection: &'connection mut Connection) -> Self {
        Self { connection }
    }

    pub fn create(
        &mut self,
        schedule: &ScheduleRecord,
        kind: ScheduleNotificationKind,
        message: &str,
        occurrence_id: Option<&str>,
    ) -> Result<()> {
        self.create_at(schedule, kind, message, occurrence_id, &now())
    }

    pub fn create_at(
        &mut self,
        schedule: &ScheduleRecord,
        kind: ScheduleNotificationKind,
        message: &str,
        occurrence_id: Option<&str>,
        created_at: &str,
    ) -> Result<()> {
        self.connection.execute(
            "INSERT INTO schedule_notifications (
               id, project_dir, agent_name, schedule_id, occurrence_id,
               kind, message, created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                Uuid::new_v4().to_string(),
                schedule.project_dir,
                schedule.agent_name,
                schedule.id,
                occurrence_id,
                kind.as_str(),
                message,
                created_at
            ],
        )?;
        Ok(())
    }

    pub fn unread(&self, project_dir: &str) -> Result<Vec<ScheduleNotification>> {
        let mut statement = self.connection.prepare(
            "SELECT * FROM schedule_notifications
             WHERE project_dir = ? AND is_read = 0 ORDER BY created_at",
        )?;
        let rows = statement.query_map([project_dir], raw_notification)?;
        rows.map(|row| {
            row.map_err(PersistenceError::from)
                .and_then(map_notification)
        })
        .collect()
    }

    pub fn mark_read(&mut self, project_dir: &str) -> Result<usize> {
        self.connection
            .execute(
                "UPDATE schedule_notifications SET is_read = 1
                 WHERE project_dir = ? AND is_read = 0",
                [project_dir],
            )
            .map_err(PersistenceError::from)
    }
}
