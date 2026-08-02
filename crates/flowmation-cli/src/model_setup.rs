use std::path::PathBuf;

use async_trait::async_trait;
use flowmation_application::{ConfigService, ModelSetup};
use flowmation_codex::{CodexModel, OPENAI_SUBSCRIPTION_PROVIDER_NAME};
use flowmation_domain::config::{CredentialSource, ProviderKind};

const DEFAULT_PROVIDER: &str = "ollama";
const DEFAULT_BASE_URL: &str = "http://localhost:11434";
const DEFAULT_CONTEXT_WINDOW: u64 = 8_192;
const DEFAULT_OPENAI_CONTEXT_WINDOW: u64 = 1_050_000;
const OPENAI_APP_SERVER_URL: &str = "codex://app-server";
const DEFAULT_API_PROVIDER: &str = "openai-api";
const DEFAULT_API_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_API_KEY_ENVIRONMENT: &str = "OPENAI_API_KEY";
const DEFAULT_API_CONTEXT_WINDOW: u64 = 128_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SetupProvider {
    Ollama,
    OpenAiSubscription,
    OpenAiCompatible,
}

#[must_use]
pub fn format_openai_model(model: &CodexModel) -> String {
    let default = if model.is_default { " (default)" } else { "" };
    format!(
        "  {OPENAI_SUBSCRIPTION_PROVIDER_NAME}/{} — {}{default}",
        model.id, model.display_name
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelSetupResult {
    Completed {
        config_path: PathBuf,
        provider: String,
        model: String,
    },
    Cancelled,
}

#[async_trait]
pub trait ModelSetupIo: Send + Sync {
    async fn prompt(&self, prompt: &str) -> Result<Option<String>, String>;
    async fn authenticate_openai(&self) -> Result<(), String>;
    async fn discover_openai_models(&self) -> Result<Vec<CodexModel>, String>;
    fn output(&self, message: &str);
}

pub struct ModelSetupService<'a> {
    config: &'a ConfigService,
    io: &'a dyn ModelSetupIo,
}

impl<'a> ModelSetupService<'a> {
    #[must_use]
    pub const fn new(config: &'a ConfigService, io: &'a dyn ModelSetupIo) -> Self {
        Self { config, io }
    }

    pub async fn run(&self) -> Result<ModelSetupResult, String> {
        self.io
            .output("Let's set up your first provider and model.");
        let Some(provider) = self.ask_provider().await? else {
            return Ok(ModelSetupResult::Cancelled);
        };
        self.run_for_provider(provider).await
    }

    pub async fn run_openai(
        &self,
        requested_model: Option<&str>,
    ) -> Result<ModelSetupResult, String> {
        let set_as_default = !self
            .config
            .load()
            .await
            .map_err(|error| error.to_string())?
            .models
            .has_configured_default_model();
        self.run_openai_provider(requested_model, set_as_default)
            .await
    }

    pub async fn run_openai_compatible(&self) -> Result<ModelSetupResult, String> {
        let set_as_default = !self
            .config
            .load()
            .await
            .map_err(|error| error.to_string())?
            .models
            .has_configured_default_model();
        self.run_openai_compatible_provider(set_as_default).await
    }

    async fn run_for_provider(&self, provider: SetupProvider) -> Result<ModelSetupResult, String> {
        match provider {
            SetupProvider::OpenAiSubscription => self.run_openai_provider(None, true).await,
            SetupProvider::OpenAiCompatible => self.run_openai_compatible_provider(true).await,
            SetupProvider::Ollama => {
                let Some(base_url) = self.ask_base_url().await? else {
                    return Ok(ModelSetupResult::Cancelled);
                };
                let Some(model) = self.ask_model(DEFAULT_PROVIDER, &[], None).await? else {
                    return Ok(ModelSetupResult::Cancelled);
                };
                let Some(context_window) = self.ask_context_window(DEFAULT_CONTEXT_WINDOW).await?
                else {
                    return Ok(ModelSetupResult::Cancelled);
                };
                self.save_model(
                    ModelSetup {
                        provider: DEFAULT_PROVIDER.to_owned(),
                        provider_kind: ProviderKind::Ollama,
                        base_url,
                        token_source: None,
                        model,
                        context_window,
                    },
                    true,
                )
                .await
            }
        }
    }

    async fn run_openai_provider(
        &self,
        requested_model: Option<&str>,
        set_as_default: bool,
    ) -> Result<ModelSetupResult, String> {
        self.io.authenticate_openai().await?;
        let openai_models = self.io.discover_openai_models().await?;
        if openai_models.is_empty() {
            return Err("Codex did not return any available OpenAI models.".to_owned());
        }
        if requested_model.is_none() {
            self.output_openai_models(&openai_models);
        }
        let Some(model) = self
            .ask_model(
                OPENAI_SUBSCRIPTION_PROVIDER_NAME,
                &openai_models,
                requested_model,
            )
            .await?
        else {
            return Ok(ModelSetupResult::Cancelled);
        };
        self.save_model(
            ModelSetup {
                provider: OPENAI_SUBSCRIPTION_PROVIDER_NAME.to_owned(),
                provider_kind: ProviderKind::OpenAiSubscription,
                base_url: OPENAI_APP_SERVER_URL.to_owned(),
                token_source: None,
                model,
                context_window: DEFAULT_OPENAI_CONTEXT_WINDOW,
            },
            set_as_default,
        )
        .await
    }

    async fn run_openai_compatible_provider(
        &self,
        set_as_default: bool,
    ) -> Result<ModelSetupResult, String> {
        self.io.output(
            "OpenAI-compatible APIs use provider API billing.",
        );
        let Some(provider) = self.ask_api_provider_name().await? else {
            return Ok(ModelSetupResult::Cancelled);
        };
        let Some(base_url) = self.ask_api_base_url().await? else {
            return Ok(ModelSetupResult::Cancelled);
        };
        let Some(token_source) = self.ask_token_source().await? else {
            return Ok(ModelSetupResult::Cancelled);
        };
        let Some(model) = self.ask_model(&provider, &[], None).await? else {
            return Ok(ModelSetupResult::Cancelled);
        };
        let Some(context_window) = self.ask_context_window(DEFAULT_API_CONTEXT_WINDOW).await?
        else {
            return Ok(ModelSetupResult::Cancelled);
        };
        self.save_model(
            ModelSetup {
                provider,
                provider_kind: ProviderKind::OpenAiCompatible,
                base_url,
                token_source,
                model,
                context_window,
            },
            set_as_default,
        )
        .await
    }

    async fn save_model(
        &self,
        setup: ModelSetup,
        set_as_default: bool,
    ) -> Result<ModelSetupResult, String> {
        let config_path = if set_as_default {
            self.config.save_model_setup(&setup).await
        } else {
            self.config.add_model(&setup).await
        }
        .map_err(|error| error.to_string())?;
        Ok(ModelSetupResult::Completed {
            config_path,
            provider: setup.provider,
            model: setup.model,
        })
    }

    fn output_openai_models(&self, models: &[CodexModel]) {
        self.io.output("OpenAI models available through Codex:");
        for model in models {
            self.io.output(&format_openai_model(model));
        }
    }

    async fn ask_provider(&self) -> Result<Option<SetupProvider>, String> {
        self.io.output("Available providers:");
        self.io.output("  ollama — local Ollama-compatible models");
        self.io
            .output("  openai — OpenAI models through a ChatGPT subscription");
        self.io
            .output("  openai-api — OpenAI Platform or another OpenAI-compatible API endpoint");
        loop {
            let Some(answer) = self
                .io
                .prompt(&format!("Provider name [{DEFAULT_PROVIDER}]: "))
                .await?
            else {
                return Ok(None);
            };
            match defaulted(&answer, DEFAULT_PROVIDER).as_str() {
                "1" | "ollama" => return Ok(Some(SetupProvider::Ollama)),
                "2" | "openai" => return Ok(Some(SetupProvider::OpenAiSubscription)),
                "3" | "openai-api" | "openai-compatible" => {
                    return Ok(Some(SetupProvider::OpenAiCompatible));
                }
                _ => {
                    self.io.output("Choose ollama, openai, or openai-api.");
                }
            }
        }
    }

    async fn ask_base_url(&self) -> Result<Option<String>, String> {
        self.ask_http_url(
            &format!("Ollama-compatible base URL [{DEFAULT_BASE_URL}]: "),
            DEFAULT_BASE_URL,
        )
        .await
    }

    async fn ask_api_base_url(&self) -> Result<Option<String>, String> {
        self.ask_http_url(
            &format!("OpenAI-compatible base URL [{DEFAULT_API_BASE_URL}]: "),
            DEFAULT_API_BASE_URL,
        )
        .await
    }

    async fn ask_http_url(&self, prompt: &str, default: &str) -> Result<Option<String>, String> {
        loop {
            let Some(answer) = self.io.prompt(prompt).await? else {
                return Ok(None);
            };
            let base_url = defaulted(&answer, default);
            if valid_http_url(&base_url) {
                return Ok(Some(base_url.trim_end_matches('/').to_owned()));
            }
            self.io.output("Enter a valid http:// or https:// URL.");
        }
    }

    async fn ask_api_provider_name(&self) -> Result<Option<String>, String> {
        loop {
            let Some(answer) = self
                .io
                .prompt(&format!("Provider name [{DEFAULT_API_PROVIDER}]: "))
                .await?
            else {
                return Ok(None);
            };
            let provider = defaulted(&answer, DEFAULT_API_PROVIDER);
            if matches!(provider.as_str(), "openai" | "ollama") {
                self.io.output(
                    "The provider names openai and ollama are reserved for their built-in adapters.",
                );
            } else if !provider.contains('/') && !provider.chars().any(char::is_whitespace) {
                return Ok(Some(provider));
            } else {
                self.io
                    .output("Provider names cannot contain spaces or slashes.");
            }
        }
    }

    async fn ask_token_source(&self) -> Result<Option<Option<CredentialSource>>, String> {
        loop {
            let Some(answer) = self
                .io
                .prompt(&format!(
                    "API key environment variable [{DEFAULT_API_KEY_ENVIRONMENT}; type none for no authentication]: "
                ))
                .await?
            else {
                return Ok(None);
            };
            let name = defaulted(&answer, DEFAULT_API_KEY_ENVIRONMENT);
            if name.eq_ignore_ascii_case("none") {
                return Ok(Some(None));
            }
            if valid_environment_name(&name) {
                return Ok(Some(Some(CredentialSource::Environment { name })));
            }
            self.io.output(
                "Environment variable names must start with a letter or underscore and contain only letters, numbers, or underscores.",
            );
        }
    }

    async fn ask_model(
        &self,
        provider: &str,
        openai_models: &[CodexModel],
        requested_model: Option<&str>,
    ) -> Result<Option<String>, String> {
        if provider == OPENAI_SUBSCRIPTION_PROVIDER_NAME
            && let Some(requested_model) = requested_model
        {
            return if openai_models
                .iter()
                .any(|model| model.id == requested_model)
            {
                Ok(Some(requested_model.to_owned()))
            } else {
                Err(format!(
                    "Codex did not report an available model named \"{requested_model}\"."
                ))
            };
        }
        let default_openai_model = openai_models
            .iter()
            .find(|model| model.is_default)
            .or_else(|| openai_models.first())
            .map(|model| model.id.as_str());
        loop {
            let prompt = if let Some(default_openai_model) = default_openai_model {
                format!(
                    "OpenAI model [{OPENAI_SUBSCRIPTION_PROVIDER_NAME}/{default_openai_model}]: "
                )
            } else {
                "Model name: ".to_owned()
            };
            let Some(answer) = self.io.prompt(&prompt).await? else {
                return Ok(None);
            };
            let model = if provider == OPENAI_SUBSCRIPTION_PROVIDER_NAME {
                if answer.trim().is_empty() {
                    default_openai_model.unwrap_or_default()
                } else {
                    answer
                        .trim()
                        .strip_prefix("openai/")
                        .unwrap_or(answer.trim())
                }
            } else {
                answer.trim()
            };
            if provider == OPENAI_SUBSCRIPTION_PROVIDER_NAME
                && !openai_models.iter().any(|entry| entry.id == model)
            {
                self.io
                    .output("Choose one of the OpenAI models listed above.");
            } else if !model.is_empty() {
                return Ok(Some(model.to_owned()));
            } else {
                self.io.output("Model name is required.");
            }
        }
    }

    async fn ask_context_window(&self, default: u64) -> Result<Option<u64>, String> {
        loop {
            let Some(answer) = self
                .io
                .prompt(&format!("Context window [{default}]: "))
                .await?
            else {
                return Ok(None);
            };
            let value = answer.trim();
            if value.is_empty() {
                return Ok(Some(default));
            }
            if let Ok(context_window) = value.parse::<u64>()
                && context_window > 0
            {
                return Ok(Some(context_window));
            }
            self.io
                .output("Context window must be a positive whole number.");
        }
    }
}

fn defaulted(value: &str, default: &str) -> String {
    if value.trim().is_empty() {
        default.to_owned()
    } else {
        value.trim().to_owned()
    }
}

fn valid_http_url(value: &str) -> bool {
    let authority = value
        .strip_prefix("http://")
        .or_else(|| value.strip_prefix("https://"));
    authority.is_some_and(|authority| {
        !authority.is_empty()
            && !authority.starts_with('/')
            && !authority.chars().any(char::is_whitespace)
    })
}

fn valid_environment_name(value: &str) -> bool {
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use tempfile::tempdir;

    use super::{ModelSetupIo, ModelSetupResult, ModelSetupService};
    use flowmation_application::{ConfigService, ModelSetup};
    use flowmation_codex::CodexModel;
    use flowmation_domain::config::{CredentialSource, ProviderKind};

    struct ScriptedSetupIo {
        answers: Mutex<VecDeque<Option<String>>>,
        output: Mutex<Vec<String>>,
        prompts: Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl ModelSetupIo for ScriptedSetupIo {
        async fn prompt(&self, prompt: &str) -> Result<Option<String>, String> {
            self.prompts
                .lock()
                .map_err(|error| error.to_string())?
                .push(prompt.to_owned());
            let answer = self
                .answers
                .lock()
                .map_err(|error| error.to_string())?
                .pop_front()
                .unwrap_or(None);
            Ok(answer)
        }

        async fn authenticate_openai(&self) -> Result<(), String> {
            Ok(())
        }

        async fn discover_openai_models(&self) -> Result<Vec<CodexModel>, String> {
            Ok(vec![
                CodexModel {
                    id: "gpt-5.6".to_owned(),
                    display_name: "GPT-5.6".to_owned(),
                    is_default: true,
                },
                CodexModel {
                    id: "gpt-5.4-mini".to_owned(),
                    display_name: "GPT-5.4 mini".to_owned(),
                    is_default: false,
                },
            ])
        }

        fn output(&self, message: &str) {
            if let Ok(mut output) = self.output.lock() {
                output.push(message.to_owned());
            }
        }
    }

    #[tokio::test]
    async fn creates_first_model_from_validated_answers() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = tempdir()?;
        let config = ConfigService::new(root.path().join("global"), root.path().join("project"));
        let io = ScriptedSetupIo {
            answers: Mutex::new(VecDeque::from([
                Some(String::new()),
                Some("not-a-url".to_owned()),
                Some("http://localhost:11434/".to_owned()),
                Some(String::new()),
                Some("llama3.2".to_owned()),
                Some("0".to_owned()),
                Some("8192".to_owned()),
            ])),
            output: Mutex::new(Vec::new()),
            prompts: Mutex::new(Vec::new()),
        };

        let result = ModelSetupService::new(&config, &io).run().await?;
        assert_eq!(
            result,
            ModelSetupResult::Completed {
                config_path: root.path().join("global/models.json"),
                provider: "ollama".to_owned(),
                model: "llama3.2".to_owned(),
            }
        );
        let resolved = config.load().await?;
        assert_eq!(resolved.models.default_provider, "ollama");
        assert_eq!(resolved.models.default_model, "llama3.2");
        assert_eq!(
            resolved.models.providers["ollama"].base_url,
            "http://localhost:11434"
        );
        assert_eq!(
            resolved.models.providers["ollama"].models[0].context_window,
            8_192
        );
        let output = io.output.lock().map_err(|error| error.to_string())?;
        assert!(output.iter().any(|line| line.contains("valid http")));
        assert!(output.iter().any(|line| line.contains("Model name")));
        assert!(output.iter().any(|line| line.contains("positive whole")));
        assert!(output.iter().any(|line| line.contains("openai-api")));
        Ok(())
    }

    #[tokio::test]
    async fn cancels_when_input_closes() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let config = ConfigService::new(root.path().join("global"), root.path().join("project"));
        let io = ScriptedSetupIo {
            answers: Mutex::new(VecDeque::from([None])),
            output: Mutex::new(Vec::new()),
            prompts: Mutex::new(Vec::new()),
        };

        assert_eq!(
            ModelSetupService::new(&config, &io).run().await?,
            ModelSetupResult::Cancelled
        );
        Ok(())
    }

    #[tokio::test]
    async fn configures_openai_with_subscription_defaults() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = tempdir()?;
        let config = ConfigService::new(root.path().join("global"), root.path().join("project"));
        let io = ScriptedSetupIo {
            answers: Mutex::new(VecDeque::from([
                Some("openai".to_owned()),
                Some(String::new()),
            ])),
            output: Mutex::new(Vec::new()),
            prompts: Mutex::new(Vec::new()),
        };

        let result = ModelSetupService::new(&config, &io).run().await?;
        assert_eq!(
            result,
            ModelSetupResult::Completed {
                config_path: root.path().join("global/models.json"),
                provider: "openai".to_owned(),
                model: "gpt-5.6".to_owned(),
            }
        );
        let resolved = config.load().await?;
        assert_eq!(
            resolved.models.providers["openai"].base_url,
            "codex://app-server"
        );
        assert_eq!(
            resolved.models.providers["openai"].models[0].context_window,
            1_050_000
        );
        let output = io.output.lock().map_err(|error| error.to_string())?;
        assert!(output.iter().any(|line| line.contains("openai/gpt-5.6")));
        let prompts = io.prompts.lock().map_err(|error| error.to_string())?;
        assert!(prompts.iter().any(|line| line.contains("OpenAI model")));
        assert!(!prompts.iter().any(|line| line.contains("Context window")));
        Ok(())
    }

    #[tokio::test]
    async fn configures_a_requested_discovered_openai_model()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let config = ConfigService::new(root.path().join("global"), root.path().join("project"));
        let io = ScriptedSetupIo {
            answers: Mutex::new(VecDeque::new()),
            output: Mutex::new(Vec::new()),
            prompts: Mutex::new(Vec::new()),
        };

        let result = ModelSetupService::new(&config, &io)
            .run_openai(Some("gpt-5.4-mini"))
            .await?;
        assert_eq!(
            result,
            ModelSetupResult::Completed {
                config_path: root.path().join("global/models.json"),
                provider: "openai".to_owned(),
                model: "gpt-5.4-mini".to_owned(),
            }
        );
        assert!(
            io.prompts
                .lock()
                .map_err(|error| error.to_string())?
                .is_empty()
        );
        Ok(())
    }

    #[tokio::test]
    async fn adding_openai_model_preserves_the_existing_default()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let config = ConfigService::new(root.path().join("global"), root.path().join("project"));
        config
            .save_model_setup(&ModelSetup {
                provider: "ollama".to_owned(),
                provider_kind: ProviderKind::Ollama,
                base_url: "http://localhost:11434".to_owned(),
                token_source: None,
                model: "llama3.2".to_owned(),
                context_window: 8_192,
            })
            .await?;
        let io = ScriptedSetupIo {
            answers: Mutex::new(VecDeque::new()),
            output: Mutex::new(Vec::new()),
            prompts: Mutex::new(Vec::new()),
        };

        ModelSetupService::new(&config, &io)
            .run_openai(Some("gpt-5.4-mini"))
            .await?;

        let resolved = config.load().await?;
        assert_eq!(resolved.models.default_provider, "ollama");
        assert_eq!(resolved.models.default_model, "llama3.2");
        assert!(
            resolved.models.providers["openai"]
                .models
                .iter()
                .any(|model| model.name == "gpt-5.4-mini")
        );
        Ok(())
    }

    #[tokio::test]
    async fn configures_an_openai_compatible_api_without_storing_the_key()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let config = ConfigService::new(root.path().join("global"), root.path().join("project"));
        let io = ScriptedSetupIo {
            answers: Mutex::new(VecDeque::from([
                Some("openai-api".to_owned()),
                Some("openai".to_owned()),
                Some("openrouter".to_owned()),
                Some("https://openrouter.ai/api/v1/".to_owned()),
                Some("OPENROUTER_API_KEY".to_owned()),
                Some("example/model".to_owned()),
                Some("131072".to_owned()),
            ])),
            output: Mutex::new(Vec::new()),
            prompts: Mutex::new(Vec::new()),
        };

        let result = ModelSetupService::new(&config, &io).run().await?;

        assert_eq!(
            result,
            ModelSetupResult::Completed {
                config_path: root.path().join("global/models.json"),
                provider: "openrouter".to_owned(),
                model: "example/model".to_owned(),
            }
        );
        let resolved = config.load().await?;
        let provider = &resolved.models.providers["openrouter"];
        assert_eq!(provider.kind, ProviderKind::OpenAiCompatible);
        assert_eq!(provider.base_url, "https://openrouter.ai/api/v1");
        assert_eq!(
            provider.token_source,
            Some(CredentialSource::Environment {
                name: "OPENROUTER_API_KEY".to_owned()
            })
        );
        let stored = tokio::fs::read_to_string(root.path().join("global/models.json")).await?;
        assert!(!stored.contains("test-secret"));
        assert!(stored.contains("OPENROUTER_API_KEY"));
        let output = io.output.lock().map_err(|error| error.to_string())?;
        assert!(output.iter().any(|line| line.contains("API billing")));
        assert!(output.iter().any(|line| line.contains("reserved")));
        Ok(())
    }
}
