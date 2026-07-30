use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use flowmation_domain::cron::{CronExpression, validate_timezone};
use flowmation_domain::ids::ScheduleId;
use flowmation_domain::schedule::{
    CreateScheduleInput, ScheduleOccurrence, ScheduleOccurrenceStatus, ScheduleRecord,
    ScheduleStatus,
};
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::workflow::WorkflowRecord;

const LEASE_KEY: &str = "schedule-worker";
const LEASE_DURATION: Duration = Duration::from_secs(45);
const LEASE_HEARTBEAT: Duration = Duration::from_secs(15);

#[derive(Clone, Debug)]
pub struct ScheduleRequest {
    pub agent_name: String,
    pub workflow_name: String,
    pub input: Value,
    pub cron: String,
    pub timezone: Option<String>,
    pub now: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug)]
pub struct PreparedScheduleReauthorization {
    pub id: ScheduleId,
    pub expected_updated_at: String,
    pub package_fingerprint: String,
    pub next_run_at: DateTime<Utc>,
    pub confirmation: ScheduleRecord,
}

pub trait ScheduledWorkflowCatalog: Send + Sync {
    fn resolve(&self, agent_name: &str, requested: &str) -> Result<WorkflowRecord, String>;
    fn validate_input(&self, workflow: &WorkflowRecord, input: &Value) -> Result<(), String>;
    fn package_fingerprint(&self, agent_name: &str, workflow: &WorkflowRecord) -> String;
}

pub trait ScheduleRepository: Send + Sync {
    fn create(
        &self,
        input: &CreateScheduleInput,
        next_run_at: DateTime<Utc>,
    ) -> Result<ScheduleRecord, String>;
    fn list(&self, project_dir: &Path) -> Result<Vec<ScheduleRecord>, String>;
    fn get(&self, id: &ScheduleId) -> Result<Option<ScheduleRecord>, String>;
    fn occurrences(&self, id: &ScheduleId) -> Result<Vec<ScheduleOccurrence>, String>;
    fn set_status(&self, id: &ScheduleId, status: ScheduleStatus) -> Result<bool, String>;
    fn delete(&self, id: &ScheduleId) -> Result<bool, String>;
    fn reauthorize(
        &self,
        id: &ScheduleId,
        fingerprint: &str,
        next_run_at: DateTime<Utc>,
        expected_updated_at: &str,
    ) -> Result<bool, String>;
}

pub struct ScheduleService {
    project_dir: PathBuf,
    catalog: Arc<dyn ScheduledWorkflowCatalog>,
    repository: Arc<dyn ScheduleRepository>,
}

impl ScheduleService {
    #[must_use]
    pub fn new(
        project_dir: impl Into<PathBuf>,
        catalog: Arc<dyn ScheduledWorkflowCatalog>,
        repository: Arc<dyn ScheduleRepository>,
    ) -> Self {
        Self {
            project_dir: project_dir.into(),
            catalog,
            repository,
        }
    }

    pub fn create(&self, request: &ScheduleRequest) -> Result<ScheduleRecord, String> {
        let (input, next_run_at) = self.prepare(request)?;
        self.repository.create(&input, next_run_at)
    }

    pub fn preview_confirmation(&self, request: &ScheduleRequest) -> Result<String, String> {
        let (input, _next_run_at) = self.prepare(request)?;
        Ok(confirmation_fields(
            &input.agent_name,
            &input.workflow_name,
            &input.input,
            &input.project_dir,
            &input.timezone,
            &input.cron,
            &input.package_fingerprint,
        ))
    }

    pub fn list(&self) -> Result<Vec<ScheduleRecord>, String> {
        self.repository.list(&self.project_dir)
    }

    pub fn get(&self, id: &ScheduleId) -> Result<Option<ScheduleRecord>, String> {
        Ok(self
            .repository
            .get(id)?
            .filter(|schedule| schedule.project_dir == self.project_dir))
    }

    pub fn occurrences(&self, id: &ScheduleId) -> Result<Vec<ScheduleOccurrence>, String> {
        self.require_project_schedule(id)?;
        self.repository.occurrences(id)
    }

    pub fn pause(&self, id: &ScheduleId) -> Result<(), String> {
        self.require_project_schedule(id)?;
        self.repository
            .set_status(id, ScheduleStatus::Paused)
            .and_then(require_changed)
    }

    pub fn resume(&self, id: &ScheduleId) -> Result<(), String> {
        let schedule = self.require_project_schedule(id)?;
        if schedule.status == ScheduleStatus::NeedsReauthorization {
            return Err(format!(
                "Schedule \"{id}\" needs reauthorization because its agent package changed."
            ));
        }
        self.repository
            .set_status(id, ScheduleStatus::Active)
            .and_then(require_changed)
    }

    pub fn delete(&self, id: &ScheduleId) -> Result<(), String> {
        self.require_project_schedule(id)?;
        self.repository.delete(id).and_then(require_changed)
    }

    pub fn prepare_reauthorization(
        &self,
        id: &ScheduleId,
        now: DateTime<Utc>,
    ) -> Result<PreparedScheduleReauthorization, String> {
        let schedule = self.require_project_schedule(id)?;
        let workflow = self
            .catalog
            .resolve(&schedule.agent_name, &schedule.workflow_name)?;
        self.catalog.validate_input(&workflow, &schedule.input)?;
        let next_run_at = CronExpression::parse(&schedule.cron)
            .map_err(|error| error.to_string())?
            .next(now, &schedule.timezone)
            .map_err(|error| error.to_string())?;
        let package_fingerprint = self
            .catalog
            .package_fingerprint(&schedule.agent_name, &workflow);
        let mut confirmation = schedule.clone();
        confirmation.package_fingerprint = package_fingerprint.clone();
        confirmation.status = ScheduleStatus::Active;
        confirmation.next_run_at = next_run_at.to_rfc3339_opts(SecondsFormat::Millis, true);
        Ok(PreparedScheduleReauthorization {
            id: id.clone(),
            expected_updated_at: schedule.updated_at,
            package_fingerprint,
            next_run_at,
            confirmation,
        })
    }

    pub fn reauthorize(
        &self,
        prepared: &PreparedScheduleReauthorization,
    ) -> Result<ScheduleRecord, String> {
        self.require_project_schedule(&prepared.id)?;
        let updated = self.repository.reauthorize(
            &prepared.id,
            &prepared.package_fingerprint,
            prepared.next_run_at,
            &prepared.expected_updated_at,
        )?;
        if !updated {
            return Err(format!(
                "Schedule \"{}\" changed while reauthorization was awaiting approval. Review it \
                 again.",
                prepared.id
            ));
        }
        self.repository
            .get(&prepared.id)?
            .ok_or_else(|| format!("Unknown schedule \"{}\".", prepared.id))
    }

    #[must_use]
    pub fn confirmation(schedule: &ScheduleRecord) -> String {
        confirmation_fields(
            &schedule.agent_name,
            &schedule.workflow_name,
            &schedule.input,
            &schedule.project_dir,
            &schedule.timezone,
            &schedule.cron,
            &schedule.package_fingerprint,
        )
    }

    fn prepare(
        &self,
        request: &ScheduleRequest,
    ) -> Result<(CreateScheduleInput, DateTime<Utc>), String> {
        let workflow = self
            .catalog
            .resolve(&request.agent_name, &request.workflow_name)?;
        self.catalog.validate_input(&workflow, &request.input)?;
        let timezone = request
            .timezone
            .clone()
            .unwrap_or_else(|| iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".to_owned()));
        validate_timezone(&timezone).map_err(|error| error.to_string())?;
        let cron = CronExpression::parse(&request.cron).map_err(|error| error.to_string())?;
        let now = request.now.unwrap_or_else(Utc::now);
        let next_run_at = cron
            .next(now, &timezone)
            .map_err(|error| error.to_string())?;
        Ok((
            CreateScheduleInput {
                project_dir: self.project_dir.clone(),
                agent_name: request.agent_name.clone(),
                workflow_name: workflow.metadata.name.clone(),
                input: request.input.clone(),
                cron: cron.source().to_owned(),
                timezone,
                package_fingerprint: self
                    .catalog
                    .package_fingerprint(&request.agent_name, &workflow),
                now: Some(now.to_rfc3339_opts(SecondsFormat::Millis, true)),
            },
            next_run_at,
        ))
    }

    fn require_project_schedule(&self, id: &ScheduleId) -> Result<ScheduleRecord, String> {
        self.get(id)?
            .ok_or_else(|| format!("Unknown schedule \"{id}\"."))
    }
}

fn require_changed(changed: bool) -> Result<(), String> {
    if changed {
        Ok(())
    } else {
        Err("Schedule was not updated.".to_owned())
    }
}

fn confirmation_fields(
    agent_name: &str,
    workflow_name: &str,
    input: &Value,
    project_dir: &Path,
    timezone: &str,
    cron: &str,
    package_fingerprint: &str,
) -> String {
    format!(
        "Agent: {agent_name}\nWorkflow: {agent_name}/{workflow_name}\nInput: {input}\nWorking \
         directory: {}\nTimezone: {timezone}\nCadence: {cron}\nPackage fingerprint: \
         {package_fingerprint}",
        project_dir.display()
    )
}

#[derive(Clone, Debug)]
pub struct WorkerExecutionResult {
    pub run_id: Option<String>,
    pub status: ScheduleOccurrenceStatus,
    pub result: Option<Value>,
    pub error: Option<String>,
}

#[async_trait]
pub trait ScheduleExecution: Send + Sync {
    async fn source_matches(&self, schedule: &ScheduleRecord) -> Result<bool, String>;

    async fn execute(
        &self,
        schedule: &ScheduleRecord,
        occurrence: &ScheduleOccurrence,
    ) -> WorkerExecutionResult;
}

pub trait ScheduleWorkerRepository: Send + Sync {
    fn acquire_lease(
        &self,
        key: &str,
        owner_id: &str,
        now: DateTime<Utc>,
        duration: Duration,
    ) -> Result<bool, String>;
    fn recoverable_occurrences(&self) -> Result<Vec<ScheduleOccurrence>, String>;
    fn due(&self, now: DateTime<Utc>) -> Result<Vec<ScheduleRecord>, String>;
    fn schedule(&self, id: &ScheduleId) -> Result<Option<ScheduleRecord>, String>;
    fn claim(
        &self,
        schedule: &ScheduleRecord,
        scheduled_for: DateTime<Utc>,
        next_run_at: DateTime<Utc>,
    ) -> Result<Option<ScheduleOccurrence>, String>;
    fn update_occurrence(
        &self,
        occurrence: &ScheduleOccurrence,
        result: &WorkerExecutionResult,
    ) -> Result<(), String>;
    fn invalidate(
        &self,
        schedule: &ScheduleRecord,
        occurrence: &ScheduleOccurrence,
        error: &str,
    ) -> Result<(), String>;
}

pub struct ScheduleWorker {
    owner_id: String,
    repository: Arc<dyn ScheduleWorkerRepository>,
    execution: Arc<dyn ScheduleExecution>,
}

impl ScheduleWorker {
    #[must_use]
    pub fn new(
        repository: Arc<dyn ScheduleWorkerRepository>,
        execution: Arc<dyn ScheduleExecution>,
    ) -> Self {
        Self {
            owner_id: Uuid::new_v4().to_string(),
            repository,
            execution,
        }
    }

    pub async fn tick(&self, now: DateTime<Utc>) -> Result<bool, String> {
        if !self
            .repository
            .acquire_lease(LEASE_KEY, &self.owner_id, now, LEASE_DURATION)?
        {
            return Ok(false);
        }
        let heartbeat_cancellation = CancellationToken::new();
        let heartbeat = tokio::spawn({
            let cancellation = heartbeat_cancellation.clone();
            let owner_id = self.owner_id.clone();
            let repository = self.repository.clone();
            async move {
                let mut interval = tokio::time::interval(LEASE_HEARTBEAT);
                interval.tick().await;
                loop {
                    tokio::select! {
                        () = cancellation.cancelled() => break,
                        _ = interval.tick() => {
                            let _renewal = repository.acquire_lease(
                                LEASE_KEY,
                                &owner_id,
                                Utc::now(),
                                LEASE_DURATION,
                            );
                        }
                    }
                }
            }
        });
        let result = self.process(now).await;
        heartbeat_cancellation.cancel();
        let _heartbeat_result = heartbeat.await;
        result.map(|()| true)
    }

    async fn process(&self, now: DateTime<Utc>) -> Result<(), String> {
        for occurrence in self.repository.recoverable_occurrences()? {
            let Some(schedule) = self.repository.schedule(&occurrence.schedule_id)? else {
                continue;
            };
            self.execute_occurrence(&schedule, &occurrence).await?;
        }
        for schedule in self.repository.due(now)? {
            let scheduled_for = schedule
                .next_run_at
                .parse::<DateTime<Utc>>()
                .map_err(|error| error.to_string())?;
            let next_run_at = CronExpression::parse(&schedule.cron)
                .map_err(|error| error.to_string())?
                .next(now, &schedule.timezone)
                .map_err(|error| error.to_string())?;
            if let Some(occurrence) =
                self.repository
                    .claim(&schedule, scheduled_for, next_run_at)?
            {
                self.execute_occurrence(&schedule, &occurrence).await?;
            }
        }
        Ok(())
    }

    async fn execute_occurrence(
        &self,
        schedule: &ScheduleRecord,
        occurrence: &ScheduleOccurrence,
    ) -> Result<(), String> {
        if !self.execution.source_matches(schedule).await? {
            return self.repository.invalidate(
                schedule,
                occurrence,
                "The agent package changed and the schedule needs reauthorization.",
            );
        }
        let result = self.execution.execute(schedule, occurrence).await;
        self.repository.update_occurrence(occurrence, &result)
    }
}
