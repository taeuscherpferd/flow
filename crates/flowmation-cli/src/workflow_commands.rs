use std::path::{Path, PathBuf};

use flowmation_sqlite::{
    SqliteDatabase, WorkflowRunDetails, WorkflowRunStatus, WorkflowRunSummary,
};

#[derive(Clone, Debug)]
pub struct WorkflowRunScope {
    global_dir: PathBuf,
    project_dir: String,
    agent_name: String,
}

impl WorkflowRunScope {
    pub fn new(
        global_dir: impl Into<PathBuf>,
        project_dir: &Path,
        agent_name: impl Into<String>,
    ) -> Result<Self, String> {
        Ok(Self {
            global_dir: global_dir.into(),
            project_dir: path_text(project_dir)?,
            agent_name: agent_name.into(),
        })
    }

    pub fn list(&self, limit: u32) -> Result<Vec<WorkflowRunSummary>, String> {
        let mut database =
            SqliteDatabase::open_global_dir(&self.global_dir).map_err(|error| error.to_string())?;
        database
            .workflow_runs()
            .list(&self.project_dir, limit, Some(&self.agent_name))
            .map_err(|error| error.to_string())
    }

    pub fn get(&self, id: &str) -> Result<Option<WorkflowRunDetails>, String> {
        let mut database =
            SqliteDatabase::open_global_dir(&self.global_dir).map_err(|error| error.to_string())?;
        let run = database
            .workflow_runs()
            .get(id)
            .map_err(|error| error.to_string())?;
        Ok(run.filter(|run| self.contains(run)))
    }

    pub fn cancel(&self, id: &str) -> Result<Option<WorkflowRunDetails>, String> {
        let mut database =
            SqliteDatabase::open_global_dir(&self.global_dir).map_err(|error| error.to_string())?;
        let Some(run) = database
            .workflow_runs()
            .get(id)
            .map_err(|error| error.to_string())?
            .filter(|run| self.contains(run))
        else {
            return Ok(None);
        };
        if is_terminal(run.summary.status) {
            return Err(format!(
                "Workflow \"{}\" is already {}.",
                run.summary.workflow_name,
                run.summary.status.as_str()
            ));
        }
        let changed = database
            .workflow_runs()
            .update_status(id, WorkflowRunStatus::Cancelled, None)
            .map_err(|error| error.to_string())?;
        if !changed {
            return Ok(None);
        }
        database
            .workflow_runs()
            .get(id)
            .map_err(|error| error.to_string())
    }

    fn contains(&self, run: &WorkflowRunDetails) -> bool {
        run.summary.project_dir == self.project_dir && run.summary.agent_name == self.agent_name
    }
}

const fn is_terminal(status: WorkflowRunStatus) -> bool {
    matches!(
        status,
        WorkflowRunStatus::Completed
            | WorkflowRunStatus::Failed
            | WorkflowRunStatus::Cancelled
            | WorkflowRunStatus::VersionMismatch
    )
}

fn path_text(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("Path is not valid UTF-8: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use flowmation_sqlite::{
        CreateWorkflowRun, SqliteDatabase, WorkflowPresentation, WorkflowRunStatus,
    };
    use serde_json::json;

    use super::WorkflowRunScope;

    #[test]
    fn scopes_listing_and_lookup_to_project_and_agent() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        create_run(directory.path(), "visible", "/project", "main")?;
        create_run(directory.path(), "other-agent", "/project", "reviewer")?;
        create_run(directory.path(), "other-project", "/elsewhere", "main")?;
        let scope =
            WorkflowRunScope::new(directory.path(), std::path::Path::new("/project"), "main")?;

        let listed = scope.list(20)?;

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "visible");
        assert!(scope.get("visible")?.is_some());
        assert!(scope.get("other-agent")?.is_none());
        assert!(scope.get("other-project")?.is_none());
        Ok(())
    }

    #[test]
    fn cancels_nonterminal_scoped_runs_and_rejects_terminal_runs() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        create_run(directory.path(), "waiting", "/project", "main")?;
        create_run(directory.path(), "completed", "/project", "main")?;
        let mut database = SqliteDatabase::open_global_dir(directory.path())?;
        database
            .workflow_runs()
            .update_status("completed", WorkflowRunStatus::Completed, None)?;
        drop(database);
        let scope =
            WorkflowRunScope::new(directory.path(), std::path::Path::new("/project"), "main")?;

        let cancelled = scope.cancel("waiting")?.ok_or("missing cancelled run")?;

        assert_eq!(cancelled.summary.status, WorkflowRunStatus::Cancelled);
        assert!(scope.cancel("completed").is_err());
        assert!(scope.cancel("missing")?.is_none());
        Ok(())
    }

    fn create_run(
        global_dir: &std::path::Path,
        id: &str,
        project_dir: &str,
        agent_name: &str,
    ) -> Result<(), Box<dyn Error>> {
        let mut database = SqliteDatabase::open_global_dir(global_dir)?;
        database.workflow_runs().create(&CreateWorkflowRun {
            id: id.to_owned(),
            workflow_name: "demo".to_owned(),
            project_dir: project_dir.to_owned(),
            agent_name: Some(agent_name.to_owned()),
            trigger: None,
            source_entry_path: "/workflow/WORKFLOW.ts".to_owned(),
            source_fingerprint: "fingerprint".to_owned(),
            presentation: WorkflowPresentation::Direct,
            input: json!("input"),
        })?;
        Ok(())
    }
}
