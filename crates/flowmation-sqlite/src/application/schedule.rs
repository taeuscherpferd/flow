use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use flowmation_application::scheduling::{
    ScheduleRepository as ApplicationScheduleRepository, ScheduleWorkerRepository,
    WorkerExecutionResult,
};
use flowmation_domain::ids::{ScheduleId, ScheduleOccurrenceId, WorkflowRunId};
use flowmation_domain::schedule::{
    CreateScheduleInput, ScheduleOccurrence as ApplicationOccurrence,
    ScheduleOccurrenceStatus as ApplicationOccurrenceStatus, ScheduleRecord as ApplicationSchedule,
    ScheduleStatus as ApplicationScheduleStatus,
};

use super::SqliteApplicationRepository;
use crate::{
    CreateSchedule, OccurrenceUpdate, ScheduleOccurrence, ScheduleOccurrenceStatus, ScheduleRecord,
    ScheduleStatus,
};

impl ApplicationScheduleRepository for SqliteApplicationRepository {
    fn create(
        &self,
        input: &CreateScheduleInput,
        next_run_at: DateTime<Utc>,
    ) -> Result<ApplicationSchedule, String> {
        let create = CreateSchedule {
            id: None,
            project_dir: path_text(&input.project_dir)?,
            agent_name: input.agent_name.clone(),
            workflow_name: input.workflow_name.clone(),
            input: input.input.clone(),
            cron: input.cron.clone(),
            timezone: input.timezone.clone(),
            package_fingerprint: input.package_fingerprint.clone(),
            next_run_at: timestamp(next_run_at),
            now: input.now.clone(),
        };
        self.database()?
            .schedules()
            .create(&create)
            .map_err(|error| error.to_string())
            .and_then(to_application_schedule)
    }

    fn list(&self, project_dir: &Path) -> Result<Vec<ApplicationSchedule>, String> {
        self.database()?
            .schedules()
            .list(&path_text(project_dir)?)
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(to_application_schedule)
            .collect()
    }

    fn get(&self, id: &ScheduleId) -> Result<Option<ApplicationSchedule>, String> {
        self.database()?
            .schedules()
            .get(id.as_str())
            .map_err(|error| error.to_string())?
            .map(to_application_schedule)
            .transpose()
    }

    fn occurrences(&self, id: &ScheduleId) -> Result<Vec<ApplicationOccurrence>, String> {
        self.database()?
            .occurrences()
            .list(id.as_str())
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(to_application_occurrence)
            .collect()
    }

    fn set_status(
        &self,
        id: &ScheduleId,
        status: ApplicationScheduleStatus,
    ) -> Result<bool, String> {
        self.database()?
            .schedules()
            .set_status(id.as_str(), to_persistence_schedule_status(status))
            .map_err(|error| error.to_string())
    }

    fn delete(&self, id: &ScheduleId) -> Result<bool, String> {
        self.database()?
            .schedules()
            .delete(id.as_str())
            .map_err(|error| error.to_string())
    }

    fn reauthorize(
        &self,
        id: &ScheduleId,
        fingerprint: &str,
        next_run_at: DateTime<Utc>,
        expected_updated_at: &str,
    ) -> Result<bool, String> {
        self.database()?
            .schedules()
            .reauthorize(
                id.as_str(),
                fingerprint,
                &timestamp(next_run_at),
                Some(expected_updated_at),
            )
            .map_err(|error| error.to_string())
    }
}

impl ScheduleWorkerRepository for SqliteApplicationRepository {
    fn acquire_lease(
        &self,
        key: &str,
        owner_id: &str,
        now: DateTime<Utc>,
        duration: Duration,
    ) -> Result<bool, String> {
        let milliseconds = i64::try_from(duration.as_millis())
            .map_err(|_| "Schedule worker lease duration is too large.".to_owned())?;
        self.database()?
            .worker_leases()
            .acquire(key, owner_id, now, milliseconds)
            .map_err(|error| error.to_string())
    }

    fn recoverable_occurrences(&self) -> Result<Vec<ApplicationOccurrence>, String> {
        self.database()?
            .occurrences()
            .list_recoverable()
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(to_application_occurrence)
            .collect()
    }

    fn due(&self, now: DateTime<Utc>) -> Result<Vec<ApplicationSchedule>, String> {
        self.database()?
            .schedules()
            .list_due(&timestamp(now))
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(to_application_schedule)
            .collect()
    }

    fn schedule(&self, id: &ScheduleId) -> Result<Option<ApplicationSchedule>, String> {
        ApplicationScheduleRepository::get(self, id)
    }

    fn claim(
        &self,
        schedule: &ApplicationSchedule,
        scheduled_for: DateTime<Utc>,
        next_run_at: DateTime<Utc>,
    ) -> Result<Option<ApplicationOccurrence>, String> {
        self.database()?
            .occurrences()
            .claim_due(
                schedule.id.as_str(),
                &timestamp(scheduled_for),
                &timestamp(next_run_at),
            )
            .map_err(|error| error.to_string())?
            .map(to_application_occurrence)
            .transpose()
    }

    fn update_occurrence(
        &self,
        occurrence: &ApplicationOccurrence,
        result: &WorkerExecutionResult,
    ) -> Result<(), String> {
        self.database()?
            .occurrences()
            .update(
                occurrence.id.as_str(),
                to_persistence_occurrence_status(result.status),
                &OccurrenceUpdate {
                    run_id: result.run_id.clone(),
                    result: result.result.clone(),
                    error: result.error.clone(),
                },
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn invalidate(
        &self,
        schedule: &ApplicationSchedule,
        occurrence: &ApplicationOccurrence,
        error: &str,
    ) -> Result<(), String> {
        let changed = self
            .database()?
            .occurrences()
            .invalidate_schedule(schedule.id.as_str(), occurrence.id.as_str(), error)
            .map_err(|database_error| database_error.to_string())?;
        if changed {
            Ok(())
        } else {
            Err("Schedule occurrence could not be invalidated.".to_owned())
        }
    }
}

fn to_application_schedule(record: ScheduleRecord) -> Result<ApplicationSchedule, String> {
    Ok(ApplicationSchedule {
        id: ScheduleId::new(record.id).map_err(|error| error.to_string())?,
        project_dir: PathBuf::from(record.project_dir),
        agent_name: record.agent_name,
        workflow_name: record.workflow_name,
        input: record.input,
        cron: record.cron,
        timezone: record.timezone,
        package_fingerprint: record.package_fingerprint,
        status: match record.status {
            ScheduleStatus::Active => ApplicationScheduleStatus::Active,
            ScheduleStatus::Paused => ApplicationScheduleStatus::Paused,
            ScheduleStatus::NeedsReauthorization => ApplicationScheduleStatus::NeedsReauthorization,
        },
        next_run_at: record.next_run_at,
        created_at: record.created_at,
        updated_at: record.updated_at,
    })
}

fn to_application_occurrence(record: ScheduleOccurrence) -> Result<ApplicationOccurrence, String> {
    Ok(ApplicationOccurrence {
        id: ScheduleOccurrenceId::new(record.id).map_err(|error| error.to_string())?,
        schedule_id: ScheduleId::new(record.schedule_id).map_err(|error| error.to_string())?,
        scheduled_for: record.scheduled_for,
        status: match record.status {
            ScheduleOccurrenceStatus::Pending => ApplicationOccurrenceStatus::Pending,
            ScheduleOccurrenceStatus::Running => ApplicationOccurrenceStatus::Running,
            ScheduleOccurrenceStatus::Completed => ApplicationOccurrenceStatus::Completed,
            ScheduleOccurrenceStatus::Failed => ApplicationOccurrenceStatus::Failed,
            ScheduleOccurrenceStatus::Waiting => ApplicationOccurrenceStatus::Waiting,
            ScheduleOccurrenceStatus::Skipped => ApplicationOccurrenceStatus::Skipped,
            ScheduleOccurrenceStatus::Invalidated => ApplicationOccurrenceStatus::Invalidated,
        },
        run_id: record
            .run_id
            .map(WorkflowRunId::new)
            .transpose()
            .map_err(|error| error.to_string())?,
        result: record.result,
        error: record.error,
        created_at: record.created_at,
        updated_at: record.updated_at,
    })
}

const fn to_persistence_schedule_status(status: ApplicationScheduleStatus) -> ScheduleStatus {
    match status {
        ApplicationScheduleStatus::Active => ScheduleStatus::Active,
        ApplicationScheduleStatus::Paused => ScheduleStatus::Paused,
        ApplicationScheduleStatus::NeedsReauthorization => ScheduleStatus::NeedsReauthorization,
    }
}

const fn to_persistence_occurrence_status(
    status: ApplicationOccurrenceStatus,
) -> ScheduleOccurrenceStatus {
    match status {
        ApplicationOccurrenceStatus::Pending => ScheduleOccurrenceStatus::Pending,
        ApplicationOccurrenceStatus::Running => ScheduleOccurrenceStatus::Running,
        ApplicationOccurrenceStatus::Completed => ScheduleOccurrenceStatus::Completed,
        ApplicationOccurrenceStatus::Failed => ScheduleOccurrenceStatus::Failed,
        ApplicationOccurrenceStatus::Waiting => ScheduleOccurrenceStatus::Waiting,
        ApplicationOccurrenceStatus::Skipped => ScheduleOccurrenceStatus::Skipped,
        ApplicationOccurrenceStatus::Invalidated => ScheduleOccurrenceStatus::Invalidated,
    }
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn path_text(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("Path is not valid UTF-8: {}", path.display()))
}
