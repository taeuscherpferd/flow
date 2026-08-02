use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use flowmation_domain::config::{
    AppConfig, CredentialSource, ModelConfig, ModelsConfig, PartialModelsConfig, ProviderConfig,
    ProviderKind, ResolvedConfig, merge_agent_instructions, merge_skills_config, resolve_soul,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelSetup {
    pub provider: String,
    pub provider_kind: ProviderKind,
    pub base_url: String,
    pub token_source: Option<CredentialSource>,
    pub model: String,
    pub context_window: u64,
}

#[derive(Debug, Error)]
pub enum ConfigServiceError {
    #[error("Failed to parse {path}: {source}")]
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("Could not read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Could not write {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Could not serialize {path}: {source}")]
    Serialize {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error(
        "Project model configuration at {path} cannot define a credential source for provider \"{provider}\". Configure credentials in the global models.json file."
    )]
    ProjectCredentialSource { path: PathBuf, provider: String },
}

#[derive(Clone, Debug)]
pub struct ConfigService {
    global_dir: PathBuf,
    project_dir: PathBuf,
}

impl Default for ConfigService {
    fn default() -> Self {
        let global_dir = user_home()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".work-agent");
        let project_dir = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".work-agent");
        Self::new(global_dir, project_dir)
    }
}

impl ConfigService {
    #[must_use]
    pub fn new(global_dir: impl Into<PathBuf>, project_dir: impl Into<PathBuf>) -> Self {
        Self {
            global_dir: global_dir.into(),
            project_dir: project_dir.into(),
        }
    }

    #[must_use]
    pub fn global_dir(&self) -> &Path {
        &self.global_dir
    }

    #[must_use]
    pub fn project_dir(&self) -> &Path {
        &self.project_dir
    }

    pub async fn load(&self) -> Result<ResolvedConfig, ConfigServiceError> {
        self.ensure_global_scaffold().await?;
        let global_models =
            read_json_if_exists::<ModelsConfig>(&self.global_dir.join("models.json"))
                .await?
                .unwrap_or_default();
        let project_models_path = self.project_dir.join("models.json");
        let project_models =
            read_json_if_exists::<PartialModelsConfig>(&project_models_path).await?;
        reject_project_credential_sources(project_models.as_ref(), &project_models_path)?;
        let models = global_models.merge_project(project_models.as_ref());
        let global_app = read_json_if_exists::<AppConfig>(&self.global_dir.join("config.json"))
            .await?
            .unwrap_or_default();
        let project_app = read_json_if_exists::<AppConfig>(&self.project_dir.join("config.json"))
            .await?
            .unwrap_or_default();
        let soul = resolve_soul(
            read_text_if_exists(&self.project_dir.join("SOUL.md"))
                .await?
                .as_deref(),
            read_text_if_exists(&self.global_dir.join("SOUL.md"))
                .await?
                .as_deref(),
        );
        let agents_instructions = merge_agent_instructions(
            read_text_if_exists(&self.global_dir.join("AGENTS.md"))
                .await?
                .as_deref(),
            read_text_if_exists(&self.project_dir.join("AGENTS.md"))
                .await?
                .as_deref(),
        );
        Ok(ResolvedConfig {
            models,
            skills_config: merge_skills_config(
                global_app.skills.as_ref(),
                project_app.skills.as_ref(),
            ),
            soul,
            agents_instructions,
            global_dir: self.global_dir.clone(),
            project_dir: self.project_dir.clone(),
        })
    }

    pub async fn save_model_setup(
        &self,
        setup: &ModelSetup,
    ) -> Result<PathBuf, ConfigServiceError> {
        self.save_model(setup, true).await
    }

    pub async fn add_model(&self, setup: &ModelSetup) -> Result<PathBuf, ConfigServiceError> {
        self.save_model(setup, false).await
    }

    async fn save_model(
        &self,
        setup: &ModelSetup,
        set_as_default: bool,
    ) -> Result<PathBuf, ConfigServiceError> {
        self.ensure_global_scaffold().await?;
        let path = self.global_dir.join("models.json");
        let mut current = read_json_if_exists::<ModelsConfig>(&path)
            .await?
            .unwrap_or_default();
        let models = current
            .providers
            .get(&setup.provider)
            .map_or_else(Vec::new, |provider| {
                provider
                    .models
                    .iter()
                    .filter(|model| model.name != setup.model)
                    .cloned()
                    .collect()
            });
        let mut models = models;
        models.push(ModelConfig {
            name: setup.model.clone(),
            context_window: setup.context_window,
        });
        if set_as_default {
            current.default_provider.clone_from(&setup.provider);
            current.default_model.clone_from(&setup.model);
        }
        current.providers.insert(
            setup.provider.clone(),
            ProviderConfig {
                kind: setup.provider_kind,
                base_url: setup.base_url.clone(),
                token_source: setup.token_source.clone(),
                models,
            },
        );
        write_json(&path, &current).await?;
        Ok(path)
    }

    async fn ensure_global_scaffold(&self) -> Result<(), ConfigServiceError> {
        let existed = tokio::fs::try_exists(&self.global_dir)
            .await
            .map_err(|source| ConfigServiceError::Read {
                path: self.global_dir.clone(),
                source,
            })?;
        for directory in [
            self.global_dir.join("skills"),
            self.global_dir.join("workflows"),
            self.global_dir.join("agents"),
        ] {
            tokio::fs::create_dir_all(&directory)
                .await
                .map_err(|source| ConfigServiceError::Write {
                    path: directory,
                    source,
                })?;
        }
        if existed {
            return Ok(());
        }
        write_json(
            &self.global_dir.join("models.json"),
            &ModelsConfig::default(),
        )
        .await?;
        write_json(
            &self.global_dir.join("config.json"),
            &AppConfig {
                skills: Some(BTreeMap::new()),
            },
        )
        .await?;
        write_json(
            &self.global_dir.join("package.json"),
            &serde_json::json!({"private": true, "type": "module"}),
        )
        .await?;
        write_text(
            &self.global_dir.join("SOUL.md"),
            flowmation_domain::config::DEFAULT_SOUL,
        )
        .await?;
        write_text(&self.global_dir.join("AGENTS.md"), "").await
    }
}

fn reject_project_credential_sources(
    config: Option<&PartialModelsConfig>,
    path: &Path,
) -> Result<(), ConfigServiceError> {
    let credential_provider = config
        .and_then(|config| config.providers.as_ref())
        .and_then(|providers| {
            providers
                .iter()
                .find(|(_, provider)| provider.token_source.is_some())
        });
    if let Some((provider, _)) = credential_provider {
        return Err(ConfigServiceError::ProjectCredentialSource {
            path: path.to_path_buf(),
            provider: provider.clone(),
        });
    }
    Ok(())
}

async fn read_text_if_exists(path: &Path) -> Result<Option<String>, ConfigServiceError> {
    match tokio::fs::read_to_string(path).await {
        Ok(content) => Ok(Some(content)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(ConfigServiceError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

async fn read_json_if_exists<T: DeserializeOwned>(
    path: &Path,
) -> Result<Option<T>, ConfigServiceError> {
    let Some(raw) = read_text_if_exists(path).await? else {
        return Ok(None);
    };
    serde_json::from_str(&raw)
        .map(Some)
        .map_err(|source| ConfigServiceError::Parse {
            path: path.to_path_buf(),
            source,
        })
}

async fn write_json(path: &Path, value: &impl Serialize) -> Result<(), ConfigServiceError> {
    let content =
        serde_json::to_string_pretty(value).map_err(|source| ConfigServiceError::Serialize {
            path: path.to_path_buf(),
            source,
        })?;
    write_text(path, &content).await
}

async fn write_text(path: &Path, content: &str) -> Result<(), ConfigServiceError> {
    tokio::fs::write(path, content)
        .await
        .map_err(|source| ConfigServiceError::Write {
            path: path.to_path_buf(),
            source,
        })
}

fn user_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use flowmation_domain::config::ProviderKind;
    use tempfile::tempdir;

    use super::{ConfigService, ConfigServiceError, ModelSetup};

    #[tokio::test]
    async fn loads_first_run_scaffold_without_a_model() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let service = ConfigService::new(root.path().join("global"), root.path().join("project"));
        let config = service.load().await?;
        assert!(!config.models.has_configured_default_model());
        assert_eq!(config.models.default_provider, "ollama");
        Ok(())
    }

    #[tokio::test]
    async fn saves_model_setup_as_active_global_model() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let service = ConfigService::new(root.path().join("global"), root.path().join("project"));
        let path = service
            .save_model_setup(&ModelSetup {
                provider: "local".to_owned(),
                provider_kind: ProviderKind::Ollama,
                base_url: "http://localhost:11434".to_owned(),
                token_source: None,
                model: "qwen3:8b".to_owned(),
                context_window: 16_384,
            })
            .await?;
        let config = service.load().await?;
        assert!(config.models.has_configured_default_model());
        assert_eq!(config.models.default_provider, "local");
        assert!(path.ends_with("models.json"));
        Ok(())
    }

    #[tokio::test]
    async fn adds_model_without_changing_the_active_global_model()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let service = ConfigService::new(root.path().join("global"), root.path().join("project"));
        service
            .save_model_setup(&ModelSetup {
                provider: "local".to_owned(),
                provider_kind: ProviderKind::Ollama,
                base_url: "http://localhost:11434".to_owned(),
                token_source: None,
                model: "qwen3:8b".to_owned(),
                context_window: 16_384,
            })
            .await?;

        service
            .add_model(&ModelSetup {
                provider: "openai".to_owned(),
                provider_kind: ProviderKind::OpenAiSubscription,
                base_url: "codex://app-server".to_owned(),
                token_source: None,
                model: "gpt-5.6".to_owned(),
                context_window: 1_050_000,
            })
            .await?;

        let config = service.load().await?;
        assert_eq!(config.models.default_provider, "local");
        assert_eq!(config.models.default_model, "qwen3:8b");
        assert_eq!(config.models.providers["openai"].models[0].name, "gpt-5.6");
        Ok(())
    }

    #[tokio::test]
    async fn rejects_project_defined_environment_credentials()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let project_dir = root.path().join("project");
        tokio::fs::create_dir_all(&project_dir).await?;
        tokio::fs::write(
            project_dir.join("models.json"),
            r#"{
                "providers": {
                    "remote": {
                        "kind": "openai-compatible",
                        "baseUrl": "https://example.test/v1",
                        "tokenSource": {
                            "type": "environment",
                            "name": "OPENAI_API_KEY"
                        },
                        "models": []
                    }
                }
            }"#,
        )
        .await?;
        let service = ConfigService::new(root.path().join("global"), &project_dir);

        let Err(error) = service.load().await else {
            return Err("project credentials accepted".into());
        };

        assert!(matches!(
            error,
            ConfigServiceError::ProjectCredentialSource { provider, .. }
                if provider == "remote"
        ));
        Ok(())
    }
}
