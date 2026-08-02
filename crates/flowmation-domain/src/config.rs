use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const DEFAULT_SOUL: &str = "You are a helpful, terse coding assistant.\n";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfig {
    pub name: String,
    pub context_window: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    pub kind: ProviderKind,
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_source: Option<CredentialSource>,
    pub models: Vec<ModelConfig>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    Ollama,
    #[serde(rename = "openai-subscription")]
    OpenAiSubscription,
    #[serde(rename = "openai-compatible")]
    OpenAiCompatible,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum CredentialSource {
    Environment { name: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelsConfig {
    pub default_provider: String,
    pub default_model: String,
    pub providers: BTreeMap<String, ProviderConfig>,
    #[serde(default)]
    pub model_aliases: BTreeMap<String, String>,
}

impl Default for ModelsConfig {
    fn default() -> Self {
        Self {
            default_provider: "ollama".to_owned(),
            default_model: "llama3.1".to_owned(),
            providers: BTreeMap::from([(
                "ollama".to_owned(),
                ProviderConfig {
                    kind: ProviderKind::Ollama,
                    base_url: "http://localhost:11434".to_owned(),
                    token_source: None,
                    models: Vec::new(),
                },
            )]),
            model_aliases: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartialModelsConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub providers: Option<BTreeMap<String, ProviderConfig>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_aliases: Option<BTreeMap<String, String>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ConfigScalar {
    String(String),
    Number(serde_json::Number),
    Boolean(bool),
}

pub type SkillsConfig = BTreeMap<String, BTreeMap<String, ConfigScalar>>;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct AppConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<SkillsConfig>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedConfig {
    pub models: ModelsConfig,
    pub skills_config: SkillsConfig,
    pub soul: String,
    pub agents_instructions: String,
    pub global_dir: PathBuf,
    pub project_dir: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelReference {
    pub provider: String,
    pub model: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedModel {
    pub provider_name: String,
    pub model_name: String,
    pub context_window: u64,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error(
        "No provider named \"{provider}\" configured. Check models.json in {global_dir} or {project_dir}."
    )]
    MissingDefaultProvider {
        provider: String,
        global_dir: PathBuf,
        project_dir: PathBuf,
    },
    #[error(
        "No model configured for \"{provider}\". Use /model to configure one, or update defaultModel in {models_path}."
    )]
    MissingDefaultModel {
        provider: String,
        models_path: PathBuf,
    },
    #[error("Model alias \"{alias}\" must be a non-empty unqualified name.")]
    InvalidAlias { alias: String },
    #[error("Model alias \"{alias}\" points to unknown model \"{target}\".")]
    InvalidAliasTarget { alias: String, target: String },
    #[error("Unknown provider \"{0}\".")]
    UnknownProvider(String),
    #[error("Provider \"{provider}\" has no model \"{model}\".")]
    ProviderHasNoModel { provider: String, model: String },
    #[error("Unknown model \"{0}\".")]
    UnknownModel(String),
    #[error("Model \"{model}\" exists in multiple providers — qualify it: {choices}.")]
    AmbiguousModel { model: String, choices: String },
}

impl ModelsConfig {
    #[must_use]
    pub fn has_configured_default_model(&self) -> bool {
        self.providers
            .get(&self.default_provider)
            .is_some_and(|provider| {
                provider
                    .models
                    .iter()
                    .any(|model| model.name == self.default_model)
            })
    }

    #[must_use]
    pub fn merge_project(&self, project: Option<&PartialModelsConfig>) -> Self {
        let Some(project) = project else {
            return self.clone();
        };
        let mut providers = self.providers.clone();
        if let Some(project_providers) = &project.providers {
            providers.extend(project_providers.clone());
        }
        let mut model_aliases = self.model_aliases.clone();
        if let Some(project_aliases) = &project.model_aliases {
            model_aliases.extend(project_aliases.clone());
        }
        Self {
            default_provider: project
                .default_provider
                .clone()
                .unwrap_or_else(|| self.default_provider.clone()),
            default_model: project
                .default_model
                .clone()
                .unwrap_or_else(|| self.default_model.clone()),
            providers,
            model_aliases,
        }
    }

    /// Validates the default model and every configured model alias.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when the default or an alias does not resolve.
    pub fn validate(&self, global_dir: &Path, project_dir: &Path) -> Result<(), ConfigError> {
        let Some(provider) = self.providers.get(&self.default_provider) else {
            return Err(ConfigError::MissingDefaultProvider {
                provider: self.default_provider.clone(),
                global_dir: global_dir.to_path_buf(),
                project_dir: project_dir.to_path_buf(),
            });
        };
        if !provider
            .models
            .iter()
            .any(|model| model.name == self.default_model)
        {
            return Err(ConfigError::MissingDefaultModel {
                provider: self.default_provider.clone(),
                models_path: global_dir.join("models.json"),
            });
        }
        for (alias, target) in &self.model_aliases {
            if alias.contains('/') || alias.trim().is_empty() {
                return Err(ConfigError::InvalidAlias {
                    alias: alias.clone(),
                });
            }
            if !self.contains_qualified_model(target) {
                return Err(ConfigError::InvalidAliasTarget {
                    alias: alias.clone(),
                    target: target.clone(),
                });
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn list_model_references(&self) -> Vec<ModelReference> {
        self.providers
            .iter()
            .flat_map(|(provider, config)| {
                config.models.iter().map(|model| ModelReference {
                    provider: provider.clone(),
                    model: model.name.clone(),
                })
            })
            .collect()
    }

    /// Resolves an alias, qualified model, or unique unqualified model.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when the requested model is unknown or ambiguous.
    pub fn resolve_model(&self, requested_spec: &str) -> Result<ResolvedModel, ConfigError> {
        let spec = self
            .model_aliases
            .get(requested_spec)
            .map_or(requested_spec, String::as_str);
        let (provider_name, model_name) =
            if let Some((provider_name, model_name)) = spec.split_once('/') {
                let provider_name = provider_name.trim();
                let model_name = model_name.trim();
                let Some(provider) = self.providers.get(provider_name) else {
                    return Err(ConfigError::UnknownProvider(provider_name.to_owned()));
                };
                if !provider.models.iter().any(|model| model.name == model_name) {
                    return Err(ConfigError::ProviderHasNoModel {
                        provider: provider_name.to_owned(),
                        model: model_name.to_owned(),
                    });
                }
                (provider_name.to_owned(), model_name.to_owned())
            } else {
                let matches: Vec<ModelReference> = self
                    .list_model_references()
                    .into_iter()
                    .filter(|reference| reference.model == spec)
                    .collect();
                match matches.as_slice() {
                    [] => return Err(ConfigError::UnknownModel(requested_spec.to_owned())),
                    [model] => (model.provider.clone(), model.model.clone()),
                    _ => {
                        let choices = matches
                            .iter()
                            .map(|model| format!("{}/{}", model.provider, model.model))
                            .collect::<Vec<_>>()
                            .join(", ");
                        return Err(ConfigError::AmbiguousModel {
                            model: requested_spec.to_owned(),
                            choices,
                        });
                    }
                }
            };
        let context_window = self
            .providers
            .get(&provider_name)
            .and_then(|provider| {
                provider
                    .models
                    .iter()
                    .find(|model| model.name == model_name)
            })
            .map(|model| model.context_window)
            .ok_or_else(|| ConfigError::ProviderHasNoModel {
                provider: provider_name.clone(),
                model: model_name.clone(),
            })?;
        Ok(ResolvedModel {
            provider_name,
            model_name,
            context_window,
        })
    }

    fn contains_qualified_model(&self, target: &str) -> bool {
        let Some((provider, model)) = target.split_once('/') else {
            return false;
        };
        !provider.is_empty()
            && self
                .providers
                .get(provider)
                .is_some_and(|config| config.models.iter().any(|entry| entry.name == model))
    }
}

#[must_use]
pub fn merge_skills_config(
    global: Option<&SkillsConfig>,
    project: Option<&SkillsConfig>,
) -> SkillsConfig {
    let names: BTreeSet<&String> = global
        .into_iter()
        .flat_map(BTreeMap::keys)
        .chain(project.into_iter().flat_map(BTreeMap::keys))
        .collect();
    names
        .into_iter()
        .map(|name| {
            let mut values = global
                .and_then(|config| config.get(name))
                .cloned()
                .unwrap_or_default();
            if let Some(project_values) = project.and_then(|config| config.get(name)) {
                values.extend(project_values.clone());
            }
            (name.clone(), values)
        })
        .collect()
}

#[must_use]
pub fn resolve_soul(project: Option<&str>, global: Option<&str>) -> String {
    project.or(global).unwrap_or(DEFAULT_SOUL).to_owned()
}

#[must_use]
pub fn merge_agent_instructions(global: Option<&str>, project: Option<&str>) -> String {
    let mut sections = Vec::new();
    if let Some(instructions) = global.filter(|value| !value.trim().is_empty()) {
        sections.push(format!("## Global Instructions\n\n{}", instructions.trim()));
    }
    if let Some(instructions) = project.filter(|value| !value.trim().is_empty()) {
        sections.push(format!(
            "## Project Instructions\n\n{}",
            instructions.trim()
        ));
    }
    sections.join("\n\n---\n\n")
}
