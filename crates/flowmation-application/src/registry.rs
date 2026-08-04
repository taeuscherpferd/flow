use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use flowmation_domain::agent::{
    AgentDefinition, AgentPackageFingerprint, AgentRecord, AgentToolName, FingerprintAlgorithm,
    FlowmationSkillMetadata, PackageSource, SkillFrontmatter, SkillRecord, is_kebab_case_name,
};
use flowmation_domain::chat::ThinkingMode;
use flowmation_domain::config::{ConfigScalar, ModelsConfig, SkillsConfig};
use flowmation_domain::fingerprint::{fingerprint_directory, list_regular_files};
use serde::Deserialize;
use thiserror::Error;

use crate::builtin_skills::BUILTIN_SKILLS;

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Manifest(#[from] serde_yaml::Error),
    #[error("{0}")]
    Fingerprint(#[from] flowmation_domain::fingerprint::FingerprintError),
    #[error("{0}")]
    Invalid(String),
}

#[derive(Clone, Debug)]
pub struct SkillScanRoot {
    pub directory: PathBuf,
    pub source: PackageSource,
}

#[derive(Clone, Debug, Default)]
pub struct SkillsService {
    records: BTreeMap<String, SkillRecord>,
    builtin_records: BTreeMap<String, SkillRecord>,
    config: SkillsConfig,
}

impl SkillsService {
    #[must_use]
    pub fn new(config: SkillsConfig) -> Self {
        Self {
            records: BTreeMap::new(),
            builtin_records: BTreeMap::new(),
            config,
        }
    }

    pub fn with_builtin_skills(config: SkillsConfig) -> Result<Self, RegistryError> {
        let mut service = Self::new(config);
        for source in BUILTIN_SKILLS {
            let (frontmatter, body) = parse_skill(source.raw)?;
            if frontmatter.name != source.name {
                return Err(RegistryError::Invalid(format!(
                    "built-in skill name \"{}\" does not match \"{}\"",
                    frontmatter.name, source.name
                )));
            }
            let record = service.build_record(
                frontmatter,
                body,
                PathBuf::from("builtin-skills").join(source.name),
                PackageSource::Global,
            );
            service.records.insert(source.name.to_owned(), record);
        }
        service.builtin_records.clone_from(&service.records);
        Ok(service)
    }

    #[must_use]
    pub fn from_records(records: Vec<SkillRecord>) -> Self {
        Self {
            records: records
                .into_iter()
                .map(|record| (record.frontmatter.name.clone(), record))
                .collect(),
            builtin_records: BTreeMap::new(),
            config: BTreeMap::new(),
        }
    }

    pub async fn load(&mut self, roots: &[SkillScanRoot]) -> Vec<String> {
        self.records.clone_from(&self.builtin_records);
        let mut warnings = Vec::new();
        for root in roots {
            if let Err(error) = self.scan(root).await {
                warnings.push(format!(
                    "Skipping skills in {} — {error}.",
                    root.directory.display()
                ));
            }
        }
        warnings
    }

    async fn scan(&mut self, root: &SkillScanRoot) -> Result<(), RegistryError> {
        let mut entries = match tokio::fs::read_dir(&root.directory).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        while let Some(entry) = entries.next_entry().await? {
            if !entry.file_type().await?.is_dir() {
                continue;
            }
            let skill_file = entry.path().join("SKILL.md");
            let raw = match tokio::fs::read_to_string(&skill_file).await {
                Ok(raw) => raw,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            let (frontmatter, body) = parse_skill(&raw)?;
            let name = frontmatter.name.clone();
            let record = self.build_record(frontmatter, body, entry.path(), root.source);
            self.records.insert(name, record);
        }
        Ok(())
    }

    fn build_record(
        &self,
        frontmatter: SkillFrontmatter,
        body: String,
        dir: PathBuf,
        source: PackageSource,
    ) -> SkillRecord {
        let expected_config_vars = frontmatter
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("flowmation"))
            .cloned()
            .and_then(|value| serde_json::from_value::<FlowmationSkillMetadata>(value).ok());
        let body = body.trim().to_owned();
        let rendered_body = render_skill_body(
            &body,
            self.config.get(&frontmatter.name),
            expected_config_vars.as_ref(),
        );
        SkillRecord {
            frontmatter,
            body,
            rendered_body,
            expected_config_vars,
            dir,
            source,
            agent_name: None,
            resource_id: None,
        }
    }

    #[must_use]
    pub fn list(&self) -> Vec<SkillFrontmatter> {
        self.records
            .values()
            .map(|record| record.frontmatter.clone())
            .collect()
    }

    #[must_use]
    pub fn records(&self, agent_name: Option<&str>) -> Vec<SkillRecord> {
        self.records
            .values()
            .cloned()
            .map(|mut record| {
                if let Some(agent_name) = agent_name {
                    record.agent_name = Some(agent_name.to_owned());
                    record.resource_id = Some(format!("{agent_name}/{}", record.frontmatter.name));
                }
                record
            })
            .collect()
    }

    #[must_use]
    pub fn body(&self, name: &str) -> Option<&str> {
        self.records
            .get(name)
            .map(|record| record.rendered_body.as_str())
    }

    #[must_use]
    pub fn record(&self, name: &str) -> Option<&SkillRecord> {
        self.records.get(name)
    }
}

#[derive(Debug, Deserialize)]
struct AgentManifest {
    version: u8,
    name: String,
    description: String,
    model: Option<String>,
    thinking: Option<ThinkingMode>,
    tools: Option<Vec<AgentToolName>>,
}

impl AgentManifest {
    fn into_definition(self) -> AgentDefinition {
        AgentDefinition {
            version: self.version,
            name: self.name,
            description: self.description,
            model: self.model,
            thinking: self.thinking,
            tools: self
                .tools
                .unwrap_or_else(|| vec![AgentToolName::ReadFile, AgentToolName::LoadSkill]),
        }
    }
}

#[derive(Debug)]
pub struct AgentPackageRegistry {
    global_dir: PathBuf,
    project_dir: PathBuf,
    models: ModelsConfig,
    skills_config: SkillsConfig,
    records: BTreeMap<String, AgentRecord>,
    warnings: Vec<String>,
}

impl AgentPackageRegistry {
    #[must_use]
    pub fn new(
        global_dir: impl Into<PathBuf>,
        project_dir: impl Into<PathBuf>,
        models: ModelsConfig,
        skills_config: SkillsConfig,
    ) -> Self {
        Self {
            global_dir: global_dir.into(),
            project_dir: project_dir.into(),
            models,
            skills_config,
            records: BTreeMap::new(),
            warnings: Vec::new(),
        }
    }

    pub async fn load(&mut self) -> Result<(), RegistryError> {
        self.records.clear();
        self.warnings.clear();
        self.scan(PackageSource::Global).await?;
        self.scan(PackageSource::Project).await
    }

    #[must_use]
    pub fn list(&self) -> Vec<&AgentRecord> {
        self.records.values().collect()
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&AgentRecord> {
        self.records.get(name)
    }

    #[must_use]
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    async fn scan(&mut self, source: PackageSource) -> Result<(), RegistryError> {
        let root = match source {
            PackageSource::Global => self.global_dir.join("agents"),
            PackageSource::Project => self.project_dir.join("agents"),
        };
        let mut entries = match tokio::fs::read_dir(root).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        while let Some(entry) = entries.next_entry().await? {
            if !entry.file_type().await?.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if source == PackageSource::Project {
                self.records.remove(&name);
            }
            match self.load_directory(&entry.path(), &name, source).await {
                Ok(record) => {
                    self.records.insert(name, record);
                }
                Err(error) => self
                    .warnings
                    .push(format!("Skipping agent \"{name}\" — {error}.")),
            }
        }
        Ok(())
    }

    async fn load_directory(
        &self,
        directory: &Path,
        directory_name: &str,
        source: PackageSource,
    ) -> Result<AgentRecord, RegistryError> {
        if !is_kebab_case_name(directory_name) {
            return Err(RegistryError::Invalid(
                "directory names must use lowercase kebab-case".to_owned(),
            ));
        }
        let manifest_raw = required_file(directory, "AGENT.yaml").await?;
        let manifest: AgentManifest = serde_yaml::from_str(&manifest_raw)?;
        let definition = manifest.into_definition();
        definition
            .validate()
            .map_err(|error| RegistryError::Invalid(error.to_string()))?;
        if definition.name != directory_name {
            return Err(RegistryError::Invalid(format!(
                "\"name\" must match directory \"{directory_name}\" and use lowercase kebab-case"
            )));
        }
        if let Some(model) = &definition.model {
            self.models.resolve_model(model).map_err(|_| {
                RegistryError::Invalid(format!(
                    "\"model\" resolves to unknown or ambiguous model \"{model}\""
                ))
            })?;
        }
        let soul = required_file(directory, "SOUL.md").await?;
        let instructions = required_file(directory, "AGENTS.md").await?;
        let context_index = optional_file(directory, "CONTEXT.md").await?;
        let fingerprint = fingerprint_directory(directory)?;
        let context_directory = directory.join("context");
        let context_files = if tokio::fs::try_exists(&context_directory).await? {
            list_regular_files(&context_directory)?
                .into_iter()
                .map(|file| format!("context/{}", file.to_string_lossy().replace('\\', "/")))
                .collect()
        } else {
            Vec::new()
        };
        let mut skills = SkillsService::new(self.skills_config.clone());
        let skill_warnings = skills
            .load(&[SkillScanRoot {
                directory: directory.join("skills"),
                source,
            }])
            .await;
        if let Some(warning) = skill_warnings.first() {
            return Err(RegistryError::Invalid(warning.clone()));
        }
        let skill_records = skills.records(Some(directory_name));
        if let Some(invalid) = skill_records.iter().find(|skill| {
            skill.frontmatter.name != skill.dir.file_name().unwrap_or_default().to_string_lossy()
                || !is_kebab_case_name(&skill.frontmatter.name)
        }) {
            return Err(RegistryError::Invalid(format!(
                "skill \"{}\" must match its lowercase kebab-case directory",
                invalid.frontmatter.name
            )));
        }
        Ok(AgentRecord {
            definition,
            directory: directory.to_path_buf(),
            source,
            soul,
            instructions,
            context_index,
            context_files,
            skills: skill_records,
            fingerprint: AgentPackageFingerprint {
                algorithm: FingerprintAlgorithm::Sha256,
                value: fingerprint,
            },
        })
    }
}

#[derive(Clone, Debug)]
pub struct AgentProfile {
    pub name: String,
    pub description: String,
    pub model: Option<String>,
    pub thinking: Option<ThinkingMode>,
    pub tools: Vec<AgentToolName>,
    pub soul: String,
    pub instructions: String,
    pub context_index: Option<String>,
    pub context_files: Vec<String>,
    pub package_directory: Option<PathBuf>,
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
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentDirectoryListing {
    pub name: String,
    pub description: String,
}

#[must_use]
pub fn build_system_prompt(
    profile: &AgentProfile,
    skills: &[SkillFrontmatter],
    agents: &[AgentDirectoryListing],
) -> String {
    let mut sections = vec![profile.soul.trim().to_owned()];
    if !profile.instructions.trim().is_empty() {
        sections.push(profile.instructions.trim().to_owned());
    }
    if let Some(context_index) = profile
        .context_index
        .as_deref()
        .filter(|context| !context.trim().is_empty())
    {
        sections.push(format!("## Context Index\n\n{}", context_index.trim()));
    }
    if !profile.context_files.is_empty() {
        let files = profile
            .context_files
            .iter()
            .map(|file| {
                profile.package_directory.as_ref().map_or_else(
                    || file.clone(),
                    |directory| directory.join(file).display().to_string(),
                )
            })
            .map(|file| format!("- {file}"))
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(format!("## On-demand Context\n\n{files}"));
    }
    if !agents.is_empty() {
        let listings = agents
            .iter()
            .map(|agent| format!("- **{}**: {}", agent.name, agent.description))
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(format!(
            "## Configured Agents\n\n{listings}\n\nUse list_agents for discovery and \
             delegate_agent with an explicit task when specialist isolation is useful."
        ));
    }
    if !skills.is_empty() {
        let loading = if profile.tools.contains(&AgentToolName::LoadSkill) {
            "Call load_skill with a listed name to lazily load its full instructions.\n\n"
        } else {
            ""
        };
        let listings = skills
            .iter()
            .map(|skill| format!("- **{}**: {}", skill.name, skill.description))
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(format!("## Available Skills\n\n{loading}{listings}"));
    }
    let tool_names = if profile.tools.is_empty() {
        "(none)".to_owned()
    } else {
        profile
            .tools
            .iter()
            .map(agent_tool_name)
            .collect::<Vec<_>>()
            .join(", ")
    };
    sections.push(format!("## Tools\n\nAvailable tools: {tool_names}."));
    sections.join("\n\n---\n\n")
}

fn parse_skill(raw: &str) -> Result<(SkillFrontmatter, String), RegistryError> {
    let Some(frontmatter) = raw.strip_prefix("---\n") else {
        return Err(RegistryError::Invalid(
            "SKILL.md is missing YAML frontmatter".to_owned(),
        ));
    };
    let Some((yaml, body)) = frontmatter.split_once("\n---") else {
        return Err(RegistryError::Invalid(
            "SKILL.md has unterminated YAML frontmatter".to_owned(),
        ));
    };
    let frontmatter: SkillFrontmatter = serde_yaml::from_str(yaml)?;
    if frontmatter.name.trim().is_empty() || frontmatter.description.trim().is_empty() {
        return Err(RegistryError::Invalid(
            "SKILL.md is missing required \"name\"/\"description\" frontmatter".to_owned(),
        ));
    }
    Ok((
        frontmatter,
        body.strip_prefix('\n').unwrap_or(body).to_owned(),
    ))
}

fn render_skill_body(
    body: &str,
    configured: Option<&BTreeMap<String, ConfigScalar>>,
    metadata: Option<&FlowmationSkillMetadata>,
) -> String {
    let mut values: BTreeMap<String, ConfigScalar> = metadata
        .into_iter()
        .flat_map(|metadata| &metadata.config)
        .filter_map(|(name, variable)| {
            variable
                .default
                .as_ref()
                .map(|value| (name.clone(), value.clone()))
        })
        .collect();
    if let Some(configured) = configured {
        values.extend(configured.clone());
    }
    values
        .into_iter()
        .fold(body.to_owned(), |rendered, (name, value)| {
            rendered.replace(&format!("${{{name}}}"), &config_scalar(&value))
        })
}

fn config_scalar(value: &ConfigScalar) -> String {
    match value {
        ConfigScalar::String(value) => value.clone(),
        ConfigScalar::Number(value) => value.to_string(),
        ConfigScalar::Boolean(value) => value.to_string(),
    }
}

async fn required_file(directory: &Path, name: &str) -> Result<String, RegistryError> {
    optional_file(directory, name)
        .await?
        .ok_or_else(|| RegistryError::Invalid(format!("required file {name} is missing")))
}

async fn optional_file(directory: &Path, name: &str) -> Result<Option<String>, RegistryError> {
    match tokio::fs::read_to_string(directory.join(name)).await {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn agent_tool_name(name: &AgentToolName) -> &'static str {
    match name {
        AgentToolName::ReadFile => "read_file",
        AgentToolName::WriteFile => "write_file",
        AgentToolName::RunCommand => "run_command",
        AgentToolName::LoadSkill => "load_skill",
        AgentToolName::RunWorkflow => "run_workflow",
        AgentToolName::ListAgents => "list_agents",
        AgentToolName::DelegateAgent => "delegate_agent",
        AgentToolName::CreateSchedule => "create_schedule",
        AgentToolName::ListSchedules => "list_schedules",
        AgentToolName::PauseSchedule => "pause_schedule",
        AgentToolName::ResumeSchedule => "resume_schedule",
        AgentToolName::DeleteSchedule => "delete_schedule",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use flowmation_domain::agent::PackageSource;
    use tempfile::tempdir;

    use super::{AgentPackageRegistry, SkillScanRoot, SkillsService, build_system_prompt};
    use flowmation_domain::config::{ModelConfig, ModelsConfig, ProviderConfig, ProviderKind};

    #[tokio::test]
    async fn built_in_skills_are_available_and_follow_override_precedence()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let global = root.path().join("global/create-skill");
        let project = root.path().join("project/create-skill");
        tokio::fs::create_dir_all(&global).await?;
        tokio::fs::create_dir_all(&project).await?;
        tokio::fs::write(
            global.join("SKILL.md"),
            "---\nname: create-skill\ndescription: Global override\n---\n\nglobal body",
        )
        .await?;
        tokio::fs::write(
            project.join("SKILL.md"),
            "---\nname: create-skill\ndescription: Project override\n---\n\nproject body",
        )
        .await?;

        let mut skills = SkillsService::with_builtin_skills(BTreeMap::new())?;
        assert!(skills.body("create-schedule").is_some());
        assert!(skills.body("create-skill").is_some());
        assert!(skills.body("create-workflow").is_some());

        let warnings = skills
            .load(&[
                SkillScanRoot {
                    directory: root.path().join("global"),
                    source: PackageSource::Global,
                },
                SkillScanRoot {
                    directory: root.path().join("project"),
                    source: PackageSource::Project,
                },
            ])
            .await;

        assert!(warnings.is_empty());
        assert_eq!(skills.body("create-skill"), Some("project body"));
        assert!(skills.body("create-workflow").is_some());
        assert_eq!(skills.list().len(), 3);
        Ok(())
    }

    #[tokio::test]
    async fn project_agent_packages_replace_global_packages_atomically()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        write_agent(root.path(), "global", "Global persona").await?;
        write_agent(root.path(), "project", "Project persona").await?;
        let mut registry = AgentPackageRegistry::new(
            root.path().join("global"),
            root.path().join("project"),
            models(),
            BTreeMap::new(),
        );
        registry.load().await?;
        let agent = registry.get("finance").ok_or("finance agent missing")?;
        assert_eq!(agent.soul, "Project persona");
        assert_eq!(agent.context_files, vec!["context/policy.md"]);
        assert_eq!(
            agent.skills[0].resource_id.as_deref(),
            Some("finance/reconcile-transactions")
        );
        assert!(registry.warnings().is_empty());
        let prompt = build_system_prompt(&agent.into(), &[], &[]);
        assert!(prompt.contains("Project persona"));
        Ok(())
    }

    #[tokio::test]
    async fn invalid_project_package_does_not_fall_back_to_global()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        write_agent(root.path(), "global", "Global persona").await?;
        let project = root.path().join("project/agents/finance");
        tokio::fs::create_dir_all(&project).await?;
        tokio::fs::write(
            project.join("AGENT.yaml"),
            "version: 2\nname: finance\ndescription: invalid\n",
        )
        .await?;
        let mut registry = AgentPackageRegistry::new(
            root.path().join("global"),
            root.path().join("project"),
            models(),
            BTreeMap::new(),
        );
        registry.load().await?;
        assert!(registry.get("finance").is_none());
        Ok(())
    }

    #[tokio::test]
    async fn fingerprint_changes_when_package_context_changes()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        write_agent(root.path(), "global", "Persona").await?;
        let mut registry = AgentPackageRegistry::new(
            root.path().join("global"),
            root.path().join("project"),
            models(),
            BTreeMap::new(),
        );
        registry.load().await?;
        let before = registry
            .get("finance")
            .ok_or("finance agent missing")?
            .fingerprint
            .value
            .clone();

        tokio::fs::write(
            root.path().join("global/agents/finance/context/policy.md"),
            "changed policy",
        )
        .await?;
        registry.load().await?;

        assert_ne!(
            registry
                .get("finance")
                .ok_or("finance agent missing after reload")?
                .fingerprint
                .value,
            before
        );
        Ok(())
    }

    #[tokio::test]
    async fn invalid_names_and_missing_required_files_are_rejected()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let invalid_name = root.path().join("global/agents/Bad_Name");
        tokio::fs::create_dir_all(&invalid_name).await?;
        tokio::fs::write(
            invalid_name.join("AGENT.yaml"),
            "version: 1\nname: Bad_Name\ndescription: invalid\n",
        )
        .await?;

        let missing_instructions = root.path().join("global/agents/missing-instructions");
        tokio::fs::create_dir_all(&missing_instructions).await?;
        tokio::fs::write(
            missing_instructions.join("AGENT.yaml"),
            "version: 1\nname: missing-instructions\ndescription: incomplete\n",
        )
        .await?;
        tokio::fs::write(missing_instructions.join("SOUL.md"), "Persona").await?;

        let mut registry = AgentPackageRegistry::new(
            root.path().join("global"),
            root.path().join("project"),
            models(),
            BTreeMap::new(),
        );
        registry.load().await?;

        assert!(registry.list().is_empty());
        assert!(
            registry
                .warnings()
                .iter()
                .any(|warning| warning.contains("lowercase kebab-case"))
        );
        assert!(
            registry
                .warnings()
                .iter()
                .any(|warning| warning.contains("required file AGENTS.md is missing"))
        );
        Ok(())
    }

    fn models() -> ModelsConfig {
        ModelsConfig {
            default_provider: "local".to_owned(),
            default_model: "default".to_owned(),
            providers: BTreeMap::from([(
                "local".to_owned(),
                ProviderConfig {
                    kind: ProviderKind::Ollama,
                    base_url: "http://localhost:11434".to_owned(),
                    token_source: None,
                    models: vec![
                        ModelConfig {
                            name: "default".to_owned(),
                            context_window: 8_192,
                        },
                        ModelConfig {
                            name: "specialist".to_owned(),
                            context_window: 16_384,
                        },
                    ],
                },
            )]),
            model_aliases: BTreeMap::from([("finance".to_owned(), "local/specialist".to_owned())]),
        }
    }

    async fn write_agent(
        root: &Path,
        source: &str,
        soul: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let directory = root.join(source).join("agents/finance");
        tokio::fs::create_dir_all(directory.join("skills/reconcile-transactions")).await?;
        tokio::fs::create_dir_all(directory.join("context")).await?;
        tokio::fs::write(
            directory.join("AGENT.yaml"),
            "version: 1\nname: finance\ndescription: Manages finance operations\nmodel: \
             finance\nthinking: medium\ntools:\n  - read_file\n  - load_skill\n",
        )
        .await?;
        tokio::fs::write(directory.join("SOUL.md"), soul).await?;
        tokio::fs::write(directory.join("AGENTS.md"), "Reconcile precisely.").await?;
        tokio::fs::write(directory.join("CONTEXT.md"), "Use policy on demand.").await?;
        tokio::fs::write(
            directory.join("context/policy.md"),
            format!("{source} policy"),
        )
        .await?;
        tokio::fs::write(
            directory.join("skills/reconcile-transactions/SKILL.md"),
            format!(
                "---\nname: reconcile-transactions\ndescription: Reconciles transactions\n---\n\n\
                 {source} instructions"
            ),
        )
        .await?;
        Ok(())
    }
}
