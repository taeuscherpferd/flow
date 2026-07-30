use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::chat::ThinkingMode;
use crate::config::ConfigScalar;
use crate::ids::AgentSessionId;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentExecutionMode {
    Direct,
    Delegated,
    Workflow,
    Scheduled,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentToolName {
    ReadFile,
    WriteFile,
    RunCommand,
    LoadSkill,
    RunWorkflow,
    ListAgents,
    DelegateAgent,
    CreateSchedule,
    ListSchedules,
    PauseSchedule,
    ResumeSchedule,
    DeleteSchedule,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentDefinition {
    pub version: u8,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingMode>,
    pub tools: Vec<AgentToolName>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentPackageFingerprint {
    pub algorithm: FingerprintAlgorithm,
    pub value: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FingerprintAlgorithm {
    Sha256,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PackageSource {
    Global,
    Project,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRecord {
    pub frontmatter: SkillFrontmatter,
    pub body: String,
    pub rendered_body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_config_vars: Option<FlowmationSkillMetadata>,
    pub dir: PathBuf,
    pub source: PackageSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SkillFrontmatter {
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FlowmationSkillMetadata {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub config: BTreeMap<String, FlowmationConfigVariable>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FlowmationConfigVariable {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<ConfigScalar>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRecord {
    pub definition: AgentDefinition,
    pub directory: PathBuf,
    pub source: PackageSource,
    pub soul: String,
    pub instructions: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_index: Option<String>,
    pub context_files: Vec<String>,
    pub skills: Vec<SkillRecord>,
    pub fingerprint: AgentPackageFingerprint,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionRecord {
    pub id: AgentSessionId,
    pub project_dir: PathBuf,
    pub agent_name: String,
    pub mode: AgentExecutionMode,
    pub provider: String,
    pub model: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfile {
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingMode>,
    pub tools: Vec<AgentToolName>,
    pub soul: String,
    pub instructions: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_index: Option<String>,
    pub context_files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_directory: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<AgentPackageFingerprint>,
}

impl From<&AgentRecord> for AgentProfile {
    fn from(record: &AgentRecord) -> Self {
        Self {
            name: record.definition.name.clone(),
            description: record.definition.description.clone(),
            model: record.definition.model.clone(),
            thinking: record.definition.thinking,
            tools: record.definition.tools.clone(),
            soul: record.soul.clone(),
            instructions: record.instructions.clone(),
            context_index: record.context_index.clone(),
            context_files: record.context_files.clone(),
            package_directory: Some(record.directory.clone()),
            fingerprint: Some(record.fingerprint.clone()),
        }
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum AgentDefinitionError {
    #[error("\"version\" must be 1")]
    UnsupportedVersion,
    #[error("\"name\" must use lowercase kebab-case")]
    InvalidName,
    #[error("\"description\" must be a non-empty string")]
    EmptyDescription,
}

impl AgentDefinition {
    /// Validates the persisted agent manifest fields owned by the domain.
    ///
    /// # Errors
    ///
    /// Returns an [`AgentDefinitionError`] for an unsupported version, invalid
    /// name, or empty description.
    pub fn validate(&self) -> Result<(), AgentDefinitionError> {
        if self.version != 1 {
            return Err(AgentDefinitionError::UnsupportedVersion);
        }
        if !is_kebab_case_name(&self.name) {
            return Err(AgentDefinitionError::InvalidName);
        }
        if self.description.trim().is_empty() {
            return Err(AgentDefinitionError::EmptyDescription);
        }
        Ok(())
    }
}

#[must_use]
pub fn is_kebab_case_name(name: &str) -> bool {
    !name.is_empty()
        && name.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

#[cfg(test)]
mod tests {
    use super::is_kebab_case_name;

    #[test]
    fn agent_name_pattern_matches_legacy_lowercase_kebab_case_rule() {
        assert!(is_kebab_case_name("finance"));
        assert!(is_kebab_case_name("finance-2"));
        assert!(!is_kebab_case_name("Bad_Name"));
        assert!(!is_kebab_case_name("-finance"));
        assert!(!is_kebab_case_name("finance-"));
    }
}
