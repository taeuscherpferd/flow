use chrono::{SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

use crate::{
    EffectRecord, HumanResponseRecord, NewWorkflowStep, PersistenceError, Result, WorkflowStep,
    WorkflowStepKind, WorkflowStepState,
};

pub struct WorkflowStepRepository<'connection> {
    connection: &'connection mut Connection,
}

#[allow(clippy::missing_errors_doc)]
impl<'connection> WorkflowStepRepository<'connection> {
    #[allow(clippy::redundant_pub_crate)]
    pub(crate) const fn new(connection: &'connection mut Connection) -> Self {
        Self { connection }
    }

    pub fn get(&self, run_id: &str, key: &str) -> Result<Option<WorkflowStep>> {
        get_step(self.connection, run_id, key)
    }

    pub fn start(&mut self, step: &NewWorkflowStep) -> Result<()> {
        start_step(self.connection, step, &now())
    }

    pub fn start_at(&mut self, step: &NewWorkflowStep, created_at: &str) -> Result<()> {
        start_step(self.connection, step, created_at)
    }

    pub fn complete(&mut self, run_id: &str, key: &str, output: &Value) -> Result<bool> {
        complete_step(self.connection, run_id, key, output, &now())
    }

    pub fn complete_at(
        &mut self,
        run_id: &str,
        key: &str,
        output: &Value,
        updated_at: &str,
    ) -> Result<bool> {
        complete_step(self.connection, run_id, key, output, updated_at)
    }
}

pub struct EffectRepository<'connection> {
    connection: &'connection mut Connection,
}

#[allow(clippy::missing_errors_doc)]
impl<'connection> EffectRepository<'connection> {
    #[allow(clippy::redundant_pub_crate)]
    pub(crate) const fn new(connection: &'connection mut Connection) -> Self {
        Self { connection }
    }

    pub fn get(&self, run_id: &str, key: &str) -> Result<Option<EffectRecord>> {
        let Some(step) = get_step(self.connection, run_id, key)? else {
            return Ok(None);
        };
        if step.kind != WorkflowStepKind::Effect {
            return Ok(None);
        }
        let idempotency_key = step
            .input
            .as_ref()
            .and_then(|input| input.get("idempotencyKey"))
            .and_then(Value::as_str)
            .ok_or_else(|| PersistenceError::InvalidValue {
                field: "workflow_steps.input_json.idempotencyKey",
                value: step
                    .input
                    .as_ref()
                    .map_or_else(String::new, Value::to_string),
            })?;
        Ok(Some(EffectRecord {
            key: step.key,
            idempotency_key: idempotency_key.to_owned(),
            state: step.state,
            output: step.output,
        }))
    }

    pub fn start(&mut self, run_id: &str, key: &str, idempotency_key: &str) -> Result<()> {
        start_step(
            self.connection,
            &NewWorkflowStep {
                run_id: run_id.to_owned(),
                key: key.to_owned(),
                kind: WorkflowStepKind::Effect,
                input: Some(json!({ "idempotencyKey": idempotency_key })),
            },
            &now(),
        )
    }

    pub fn complete(&mut self, run_id: &str, key: &str, output: &Value) -> Result<bool> {
        complete_step(self.connection, run_id, key, output, &now())
    }
}

pub struct HumanResponseRepository<'connection> {
    connection: &'connection mut Connection,
}

#[allow(clippy::missing_errors_doc)]
impl<'connection> HumanResponseRepository<'connection> {
    #[allow(clippy::redundant_pub_crate)]
    pub(crate) const fn new(connection: &'connection mut Connection) -> Self {
        Self { connection }
    }

    pub fn get(&self, run_id: &str, key: &str) -> Result<Option<HumanResponseRecord>> {
        let Some(step) = get_step(self.connection, run_id, key)? else {
            return Ok(None);
        };
        if step.kind != WorkflowStepKind::Human {
            return Ok(None);
        }
        let prompt = step.input.ok_or_else(|| PersistenceError::InvalidValue {
            field: "workflow_steps.input_json",
            value: "NULL".to_owned(),
        })?;
        Ok(Some(HumanResponseRecord {
            key: step.key,
            prompt,
            state: step.state,
            response: step.output,
        }))
    }

    pub fn request(&mut self, run_id: &str, key: &str, prompt: &Value) -> Result<()> {
        start_step(
            self.connection,
            &NewWorkflowStep {
                run_id: run_id.to_owned(),
                key: key.to_owned(),
                kind: WorkflowStepKind::Human,
                input: Some(prompt.clone()),
            },
            &now(),
        )
    }

    pub fn respond(&mut self, run_id: &str, key: &str, response: &Value) -> Result<bool> {
        complete_step(self.connection, run_id, key, response, &now())
    }
}

fn get_step(connection: &Connection, run_id: &str, key: &str) -> Result<Option<WorkflowStep>> {
    let raw = connection
        .query_row(
            "SELECT run_id, key, kind, state, input_json, output_json, created_at, updated_at
             FROM workflow_steps
             WHERE run_id = ? AND key = ?",
            params![run_id, key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .optional()?;
    raw.map(|row| {
        Ok(WorkflowStep {
            run_id: row.0,
            key: row.1,
            kind: WorkflowStepKind::parse(&row.2)?,
            state: WorkflowStepState::parse(&row.3)?,
            input: row
                .4
                .map(|value| serde_json::from_str(&value))
                .transpose()?,
            output: row
                .5
                .map(|value| serde_json::from_str(&value))
                .transpose()?,
            created_at: row.6,
            updated_at: row.7,
        })
    })
    .transpose()
}

fn start_step(connection: &Connection, step: &NewWorkflowStep, created_at: &str) -> Result<()> {
    connection.execute(
        "INSERT INTO workflow_steps (
           run_id, key, kind, state, input_json, created_at, updated_at
         ) VALUES (?, ?, ?, 'started', ?, ?, ?)",
        params![
            step.run_id,
            step.key,
            step.kind.as_str(),
            step.input.as_ref().map(serde_json::to_string).transpose()?,
            created_at,
            created_at,
        ],
    )?;
    Ok(())
}

fn complete_step(
    connection: &Connection,
    run_id: &str,
    key: &str,
    output: &Value,
    updated_at: &str,
) -> Result<bool> {
    let changed = connection.execute(
        "UPDATE workflow_steps
         SET state = 'completed', output_json = ?, updated_at = ?
         WHERE run_id = ? AND key = ?",
        params![serde_json::to_string(output)?, updated_at, run_id, key],
    )?;
    Ok(changed == 1)
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}
