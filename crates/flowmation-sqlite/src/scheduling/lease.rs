use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::{PersistenceError, Result};

pub struct WorkerLeaseRepository<'connection> {
    connection: &'connection mut Connection,
}

#[allow(clippy::missing_errors_doc)]
impl<'connection> WorkerLeaseRepository<'connection> {
    #[allow(clippy::redundant_pub_crate)]
    pub(crate) const fn new(connection: &'connection mut Connection) -> Self {
        Self { connection }
    }

    pub fn acquire(
        &mut self,
        lease_key: &str,
        owner_id: &str,
        at: DateTime<Utc>,
        lease_milliseconds: i64,
    ) -> Result<bool> {
        let expires_at = at
            .checked_add_signed(Duration::milliseconds(lease_milliseconds))
            .ok_or_else(|| PersistenceError::InvalidValue {
                field: "lease_milliseconds",
                value: lease_milliseconds.to_string(),
            })?
            .to_rfc3339_opts(SecondsFormat::Millis, true);
        let now = at.to_rfc3339_opts(SecondsFormat::Millis, true);
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = transaction
            .query_row(
                "SELECT owner_id, expires_at FROM schedule_worker_leases
                 WHERE lease_key = ?",
                [lease_key],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        if let Some((existing_owner, existing_expiry)) = existing
            && existing_owner != owner_id
            && existing_expiry > now
        {
            return Ok(false);
        }
        transaction.execute(
            "INSERT INTO schedule_worker_leases (
               lease_key, owner_id, expires_at, updated_at
             ) VALUES (?, ?, ?, ?)
             ON CONFLICT(lease_key) DO UPDATE SET
               owner_id = excluded.owner_id,
               expires_at = excluded.expires_at,
               updated_at = excluded.updated_at",
            params![lease_key, owner_id, expires_at, now],
        )?;
        transaction.commit()?;
        Ok(true)
    }
}
