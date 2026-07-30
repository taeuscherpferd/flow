use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, SecondsFormat, Utc};
use flowmation_application::scheduling::{
    ScheduleRepository, ScheduleRequest, ScheduleService, ScheduledWorkflowCatalog,
};
use flowmation_application::workflow::WorkflowRecord;
use flowmation_domain::agent::PackageSource;
use flowmation_domain::ids::ScheduleId;
use flowmation_domain::schedule::{
    CreateScheduleInput, ScheduleOccurrence, ScheduleRecord, ScheduleStatus,
};
use flowmation_workflow_host::protocol::{
    AgentInvocationPolicy, WorkflowMetadata, WorkflowPresentation,
};
use serde_json::{Value, json};

const CREATED_AT: &str = "2026-07-25T12:00:00.000Z";

struct Catalog {
    fingerprint: Mutex<String>,
}

impl ScheduledWorkflowCatalog for Catalog {
    fn resolve(&self, _agent_name: &str, _requested: &str) -> Result<WorkflowRecord, String> {
        Ok(workflow())
    }

    fn validate_input(&self, _workflow: &WorkflowRecord, _input: &Value) -> Result<(), String> {
        Ok(())
    }

    fn package_fingerprint(&self, _agent_name: &str, _workflow: &WorkflowRecord) -> String {
        self.fingerprint
            .lock()
            .map_or_else(|_| "poisoned".to_owned(), |value| value.clone())
    }
}

#[derive(Default)]
struct ReauthorizationRepository {
    schedule: Mutex<Option<ScheduleRecord>>,
}

impl ScheduleRepository for ReauthorizationRepository {
    fn create(
        &self,
        input: &CreateScheduleInput,
        next_run_at: DateTime<Utc>,
    ) -> Result<ScheduleRecord, String> {
        let record = ScheduleRecord {
            id: ScheduleId::new("schedule-1").map_err(|error| error.to_string())?,
            project_dir: input.project_dir.clone(),
            agent_name: input.agent_name.clone(),
            workflow_name: input.workflow_name.clone(),
            input: input.input.clone(),
            cron: input.cron.clone(),
            timezone: input.timezone.clone(),
            package_fingerprint: input.package_fingerprint.clone(),
            status: ScheduleStatus::Active,
            next_run_at: timestamp(next_run_at),
            created_at: CREATED_AT.to_owned(),
            updated_at: CREATED_AT.to_owned(),
        };
        *self.schedule.lock().map_err(|error| error.to_string())? = Some(record.clone());
        Ok(record)
    }

    fn list(&self, _project_dir: &Path) -> Result<Vec<ScheduleRecord>, String> {
        Ok(self
            .schedule
            .lock()
            .map_err(|error| error.to_string())?
            .iter()
            .cloned()
            .collect())
    }

    fn get(&self, _id: &ScheduleId) -> Result<Option<ScheduleRecord>, String> {
        Ok(self
            .schedule
            .lock()
            .map_err(|error| error.to_string())?
            .clone())
    }

    fn occurrences(&self, _id: &ScheduleId) -> Result<Vec<ScheduleOccurrence>, String> {
        Ok(Vec::new())
    }

    fn set_status(&self, _id: &ScheduleId, _status: ScheduleStatus) -> Result<bool, String> {
        Ok(false)
    }

    fn delete(&self, _id: &ScheduleId) -> Result<bool, String> {
        Ok(false)
    }

    fn reauthorize(
        &self,
        _id: &ScheduleId,
        fingerprint: &str,
        next_run_at: DateTime<Utc>,
        expected_updated_at: &str,
    ) -> Result<bool, String> {
        let mut stored = self.schedule.lock().map_err(|error| error.to_string())?;
        let Some(schedule) = stored.as_mut() else {
            return Ok(false);
        };
        if schedule.updated_at != expected_updated_at {
            return Ok(false);
        }
        schedule.package_fingerprint = fingerprint.to_owned();
        schedule.next_run_at = timestamp(next_run_at);
        schedule.status = ScheduleStatus::Active;
        schedule.updated_at = "2026-07-25T12:05:01.000Z".to_owned();
        Ok(true)
    }
}

#[test]
fn reauthorizes_the_exact_prospective_fingerprint_shown_for_approval()
-> Result<(), Box<dyn std::error::Error>> {
    let catalog = Arc::new(Catalog {
        fingerprint: Mutex::new("original".to_owned()),
    });
    let repository = Arc::new(ReauthorizationRepository::default());
    let service = ScheduleService::new("/project", catalog.clone(), repository.clone());
    let schedule = service.create(&ScheduleRequest {
        agent_name: "main".to_owned(),
        workflow_name: "scheduled-report".to_owned(),
        input: json!(""),
        cron: "* * * * *".to_owned(),
        timezone: Some("UTC".to_owned()),
        now: Some("2026-07-25T12:00:00Z".parse()?),
    })?;
    *catalog
        .fingerprint
        .lock()
        .map_err(|error| error.to_string())? = "approved".to_owned();

    let prepared =
        service.prepare_reauthorization(&schedule.id, "2026-07-25T12:05:00Z".parse()?)?;
    assert_eq!(prepared.confirmation.package_fingerprint, "approved");
    assert_eq!(
        service
            .get(&schedule.id)?
            .ok_or("schedule disappeared")?
            .package_fingerprint,
        "original"
    );
    *catalog
        .fingerprint
        .lock()
        .map_err(|error| error.to_string())? = "changed-after-approval".to_owned();

    let updated = service.reauthorize(&prepared)?;
    assert_eq!(updated.package_fingerprint, "approved");
    assert_eq!(updated.next_run_at, prepared.confirmation.next_run_at);
    Ok(())
}

fn workflow() -> WorkflowRecord {
    WorkflowRecord {
        metadata: WorkflowMetadata {
            name: "scheduled-report".to_owned(),
            description: "Produces a report".to_owned(),
            input_schema: None,
            agent_invocation: AgentInvocationPolicy::Disabled,
            presentation: WorkflowPresentation::Direct,
        },
        directory: PathBuf::from("/workflows/scheduled-report"),
        entry_path: PathBuf::from("/workflows/scheduled-report/WORKFLOW.js"),
        fingerprint: "workflow".to_owned(),
        source: PackageSource::Project,
        agent_name: Some("main".to_owned()),
        resource_id: Some("main/scheduled-report".to_owned()),
    }
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}
