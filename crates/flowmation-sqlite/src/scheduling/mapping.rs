use rusqlite::Row;

use crate::{
    Result, ScheduleNotification, ScheduleNotificationKind, ScheduleOccurrence,
    ScheduleOccurrenceStatus, ScheduleRecord, ScheduleStatus,
};

pub(super) struct RawSchedule {
    pub id: String,
    pub project_dir: String,
    pub agent_name: String,
    pub workflow_name: String,
    pub input_json: String,
    pub cron: String,
    pub timezone: String,
    pub package_fingerprint: String,
    pub status: String,
    pub next_run_at: String,
    pub created_at: String,
    pub updated_at: String,
}

pub(super) fn raw_schedule(row: &Row<'_>) -> rusqlite::Result<RawSchedule> {
    Ok(RawSchedule {
        id: row.get("id")?,
        project_dir: row.get("project_dir")?,
        agent_name: row.get("agent_name")?,
        workflow_name: row.get("workflow_name")?,
        input_json: row.get("input_json")?,
        cron: row.get("cron")?,
        timezone: row.get("timezone")?,
        package_fingerprint: row.get("package_fingerprint")?,
        status: row.get("status")?,
        next_run_at: row.get("next_run_at")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

pub(super) fn map_schedule(raw: RawSchedule) -> Result<ScheduleRecord> {
    Ok(ScheduleRecord {
        id: raw.id,
        project_dir: raw.project_dir,
        agent_name: raw.agent_name,
        workflow_name: raw.workflow_name,
        input: serde_json::from_str(&raw.input_json)?,
        cron: raw.cron,
        timezone: raw.timezone,
        package_fingerprint: raw.package_fingerprint,
        status: ScheduleStatus::parse(&raw.status)?,
        next_run_at: raw.next_run_at,
        created_at: raw.created_at,
        updated_at: raw.updated_at,
    })
}

pub(super) struct RawOccurrence {
    pub id: String,
    pub schedule_id: String,
    pub scheduled_for: String,
    pub status: String,
    pub run_id: Option<String>,
    pub result_json: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub(super) fn raw_occurrence(row: &Row<'_>) -> rusqlite::Result<RawOccurrence> {
    Ok(RawOccurrence {
        id: row.get("id")?,
        schedule_id: row.get("schedule_id")?,
        scheduled_for: row.get("scheduled_for")?,
        status: row.get("status")?,
        run_id: row.get("run_id")?,
        result_json: row.get("result_json")?,
        error: row.get("error")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

pub(super) fn map_occurrence(raw: RawOccurrence) -> Result<ScheduleOccurrence> {
    Ok(ScheduleOccurrence {
        id: raw.id,
        schedule_id: raw.schedule_id,
        scheduled_for: raw.scheduled_for,
        status: ScheduleOccurrenceStatus::parse(&raw.status)?,
        run_id: raw.run_id,
        result: raw
            .result_json
            .as_ref()
            .map(|value| serde_json::from_str(value))
            .transpose()?,
        error: raw.error,
        created_at: raw.created_at,
        updated_at: raw.updated_at,
    })
}

pub(super) struct RawNotification {
    pub id: String,
    pub project_dir: String,
    pub agent_name: String,
    pub schedule_id: Option<String>,
    pub occurrence_id: Option<String>,
    pub kind: String,
    pub message: String,
    pub is_read: i64,
    pub created_at: String,
}

pub(super) fn raw_notification(row: &Row<'_>) -> rusqlite::Result<RawNotification> {
    Ok(RawNotification {
        id: row.get("id")?,
        project_dir: row.get("project_dir")?,
        agent_name: row.get("agent_name")?,
        schedule_id: row.get("schedule_id")?,
        occurrence_id: row.get("occurrence_id")?,
        kind: row.get("kind")?,
        message: row.get("message")?,
        is_read: row.get("is_read")?,
        created_at: row.get("created_at")?,
    })
}

pub(super) fn map_notification(raw: RawNotification) -> Result<ScheduleNotification> {
    Ok(ScheduleNotification {
        id: raw.id,
        project_dir: raw.project_dir,
        agent_name: raw.agent_name,
        schedule_id: raw.schedule_id,
        occurrence_id: raw.occurrence_id,
        kind: ScheduleNotificationKind::parse(&raw.kind)?,
        message: raw.message,
        read: raw.is_read == 1,
        created_at: raw.created_at,
    })
}

pub(super) fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
