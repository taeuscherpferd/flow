use chrono::{SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::Value;

use crate::{
    CreateWorkflowRun, PersistenceError, Result, WorkflowPresentation, WorkflowRunDetails,
    WorkflowRunStatus, WorkflowRunSummary, WorkflowTrigger,
};

struct RawWorkflowRun {
    id: String,
    workflow_name: String,
    project_dir: String,
    agent_name: String,
    trigger_json: String,
    source_entry_path: String,
    source_fingerprint: String,
    status: String,
    presentation: String,
    input_json: String,
    output_json: Option<String>,
    parent_run_id: Option<String>,
    depth: i64,
    error: Option<String>,
    created_at: String,
    updated_at: String,
}

pub struct WorkflowRunRepository<'connection> {
    connection: &'connection mut Connection,
}

#[allow(clippy::missing_errors_doc)]
impl<'connection> WorkflowRunRepository<'connection> {
    #[allow(clippy::redundant_pub_crate)]
    pub(crate) const fn new(connection: &'connection mut Connection) -> Self {
        Self { connection }
    }

    pub fn create(&mut self, input: &CreateWorkflowRun) -> Result<WorkflowRunDetails> {
        self.create_at(input, &now())
    }

    pub fn create_at(
        &mut self,
        input: &CreateWorkflowRun,
        created_at: &str,
    ) -> Result<WorkflowRunDetails> {
        let trigger =
            serde_json::to_string(input.trigger.as_ref().unwrap_or(&WorkflowTrigger::Manual))?;
        self.connection.execute(
            "INSERT INTO workflow_runs (
               id, workflow_name, project_dir, agent_name, trigger_json, source_entry_path,
               source_fingerprint, status, presentation, input_json,
               parent_run_id, depth, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, 'queued', ?, ?, ?, ?, ?, ?)",
            params![
                input.id,
                input.workflow_name,
                input.project_dir,
                input.agent_name.as_deref().unwrap_or("main"),
                trigger,
                input.source_entry_path,
                input.source_fingerprint,
                input.presentation.as_str(),
                serde_json::to_string(&input.input)?,
                Option::<String>::None,
                0_i64,
                created_at,
                created_at,
            ],
        )?;
        self.get(&input.id)?
            .ok_or_else(|| PersistenceError::InvalidValue {
                field: "workflow_runs.id",
                value: input.id.clone(),
            })
    }

    pub fn get(&self, id: &str) -> Result<Option<WorkflowRunDetails>> {
        self.connection
            .query_row(
                "SELECT * FROM workflow_runs WHERE id = ?",
                [id],
                raw_workflow_run,
            )
            .optional()?
            .map(map_workflow_run)
            .transpose()
    }

    pub fn list(
        &self,
        project_dir: &str,
        limit: u32,
        agent_name: Option<&str>,
    ) -> Result<Vec<WorkflowRunSummary>> {
        let rows = if let Some(agent_name) = agent_name {
            let mut statement = self.connection.prepare(
                "SELECT * FROM workflow_runs
                 WHERE project_dir = ? AND agent_name = ?
                 ORDER BY updated_at DESC
                 LIMIT ?",
            )?;
            collect_runs(
                statement.query_map(params![project_dir, agent_name, limit], raw_workflow_run)?,
            )
        } else {
            let mut statement = self.connection.prepare(
                "SELECT * FROM workflow_runs
                 WHERE project_dir = ?
                 ORDER BY updated_at DESC
                 LIMIT ?",
            )?;
            collect_runs(statement.query_map(params![project_dir, limit], raw_workflow_run)?)
        }?;
        rows.into_iter().map(map_workflow_summary).collect()
    }

    pub fn update_status(
        &mut self,
        id: &str,
        status: WorkflowRunStatus,
        error: Option<&str>,
    ) -> Result<bool> {
        self.update_status_at(id, status, error, &now())
    }

    pub fn update_status_at(
        &mut self,
        id: &str,
        status: WorkflowRunStatus,
        error: Option<&str>,
        updated_at: &str,
    ) -> Result<bool> {
        let changed = self.connection.execute(
            "UPDATE workflow_runs
             SET status = ?, error = ?, updated_at = ?
             WHERE id = ?",
            params![status.as_str(), error, updated_at, id],
        )?;
        Ok(changed == 1)
    }

    pub fn complete(
        &mut self,
        id: &str,
        output: &Value,
        presentation: WorkflowPresentation,
    ) -> Result<bool> {
        self.complete_at(id, output, presentation, &now())
    }

    pub fn complete_at(
        &mut self,
        id: &str,
        output: &Value,
        presentation: WorkflowPresentation,
        updated_at: &str,
    ) -> Result<bool> {
        let changed = self.connection.execute(
            "UPDATE workflow_runs
             SET status = 'completed', output_json = ?, presentation = ?,
                 error = NULL, updated_at = ?
             WHERE id = ? AND status = 'running'",
            params![
                serde_json::to_string(output)?,
                presentation.as_str(),
                updated_at,
                id
            ],
        )?;
        Ok(changed == 1)
    }

    pub fn transition_to_running(&mut self, id: &str) -> Result<bool> {
        self.transition_to_running_at(id, &now())
    }

    pub fn transition_to_running_at(&mut self, id: &str, updated_at: &str) -> Result<bool> {
        let changed = self.connection.execute(
            "UPDATE workflow_runs
             SET status = 'running', error = NULL, updated_at = ?
             WHERE id = ? AND status IN ('queued', 'waiting', 'interrupted')",
            params![updated_at, id],
        )?;
        Ok(changed == 1)
    }

    pub fn transition_running_status(
        &mut self,
        id: &str,
        status: WorkflowRunStatus,
        error: Option<&str>,
    ) -> Result<bool> {
        self.transition_running_status_at(id, status, error, &now())
    }

    pub fn transition_running_status_at(
        &mut self,
        id: &str,
        status: WorkflowRunStatus,
        error: Option<&str>,
        updated_at: &str,
    ) -> Result<bool> {
        let changed = self.connection.execute(
            "UPDATE workflow_runs
             SET status = ?, error = ?, updated_at = ?
             WHERE id = ? AND status = 'running'",
            params![status.as_str(), error, updated_at, id],
        )?;
        Ok(changed == 1)
    }
}

fn raw_workflow_run(row: &Row<'_>) -> rusqlite::Result<RawWorkflowRun> {
    Ok(RawWorkflowRun {
        id: row.get("id")?,
        workflow_name: row.get("workflow_name")?,
        project_dir: row.get("project_dir")?,
        agent_name: row.get("agent_name")?,
        trigger_json: row.get("trigger_json")?,
        source_entry_path: row.get("source_entry_path")?,
        source_fingerprint: row.get("source_fingerprint")?,
        status: row.get("status")?,
        presentation: row.get("presentation")?,
        input_json: row.get("input_json")?,
        output_json: row.get("output_json")?,
        parent_run_id: row.get("parent_run_id")?,
        depth: row.get("depth")?,
        error: row.get("error")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

fn map_workflow_summary(raw: RawWorkflowRun) -> Result<WorkflowRunSummary> {
    Ok(WorkflowRunSummary {
        id: raw.id,
        workflow_name: raw.workflow_name,
        project_dir: raw.project_dir,
        agent_name: raw.agent_name,
        trigger: serde_json::from_str(&raw.trigger_json)?,
        status: WorkflowRunStatus::parse(&raw.status)?,
        presentation: WorkflowPresentation::parse(&raw.presentation)?,
        created_at: raw.created_at,
        updated_at: raw.updated_at,
        error: raw.error,
    })
}

fn map_workflow_run(raw: RawWorkflowRun) -> Result<WorkflowRunDetails> {
    let input = serde_json::from_str(&raw.input_json)?;
    let output = raw
        .output_json
        .as_ref()
        .map(|value| serde_json::from_str(value))
        .transpose()?;
    let source_entry_path = raw.source_entry_path.clone();
    let source_fingerprint = raw.source_fingerprint.clone();
    let parent_run_id = raw.parent_run_id.clone();
    let depth = raw.depth;
    Ok(WorkflowRunDetails {
        summary: map_workflow_summary(raw)?,
        input,
        output,
        source_entry_path,
        source_fingerprint,
        parent_run_id,
        depth,
    })
}

fn collect_runs<F>(rows: rusqlite::MappedRows<'_, F>) -> Result<Vec<RawWorkflowRun>>
where
    F: FnMut(&Row<'_>) -> rusqlite::Result<RawWorkflowRun>,
{
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(PersistenceError::from)
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}
