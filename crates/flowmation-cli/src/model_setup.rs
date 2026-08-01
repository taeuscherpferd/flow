use std::path::PathBuf;

use async_trait::async_trait;
use flowmation_application::{ConfigService, ModelSetup};
use flowmation_codex::{CodexModel, OPENAI_SUBSCRIPTION_PROVIDER_NAME};

const DEFAULT_PROVIDER: &str = "ollama";
const DEFAULT_BASE_URL: &str = "http://localhost:11434";
const DEFAULT_CONTEXT_WINDOW: u64 = 8_192;
const DEFAULT_OPENAI_CONTEXT_WINDOW: u64 = 1_050_000;
const OPENAI_APP_SERVER_URL: &str = "codex://app-server";

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

    async fn run_for_provider(&self, provider: String) -> Result<ModelSetupResult, String> {
        if provider == OPENAI_SUBSCRIPTION_PROVIDER_NAME {
            return self.run_openai_provider(None, true).await;
        }
        let Some(base_url) = self.ask_base_url().await? else {
            return Ok(ModelSetupResult::Cancelled);
        };
        let Some(model) = self.ask_model(&provider, &[], None).await? else {
            return Ok(ModelSetupResult::Cancelled);
        };
        let Some(context_window) = self.ask_context_window().await? else {
            return Ok(ModelSetupResult::Cancelled);
        };
        self.save_model(provider, base_url, model, context_window, true)
            .await
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
            OPENAI_SUBSCRIPTION_PROVIDER_NAME.to_owned(),
            OPENAI_APP_SERVER_URL.to_owned(),
            model,
            DEFAULT_OPENAI_CONTEXT_WINDOW,
            set_as_default,
        )
        .await
    }

    async fn save_model(
        &self,
        provider: String,
        base_url: String,
        model: String,
        context_window: u64,
        set_as_default: bool,
    ) -> Result<ModelSetupResult, String> {
        let setup = ModelSetup {
            provider: provider.clone(),
            base_url,
            model: model.clone(),
            context_window,
        };
        let config_path = if set_as_default {
            self.config.save_model_setup(&setup).await
        } else {
            self.config.add_model(&setup).await
        }
        .map_err(|error| error.to_string())?;
        Ok(ModelSetupResult::Completed {
            config_path,
            provider,
            model,
        })
    }

    fn output_openai_models(&self, models: &[CodexModel]) {
        self.io.output("OpenAI models available through Codex:");
        for model in models {
            self.io.output(&format_openai_model(model));
        }
    }

    async fn ask_provider(&self) -> Result<Option<String>, String> {
        loop {
            let Some(answer) = self
                .io
                .prompt(&format!("Provider name [{DEFAULT_PROVIDER}]: "))
                .await?
            else {
                return Ok(None);
            };
            let provider = defaulted(&answer, DEFAULT_PROVIDER);
            if !provider.contains('/') && !provider.chars().any(char::is_whitespace) {
                return Ok(Some(provider));
            }
            self.io
                .output("Provider names cannot contain spaces or slashes.");
        }
    }

    async fn ask_base_url(&self) -> Result<Option<String>, String> {
        loop {
            let Some(answer) = self
                .io
                .prompt(&format!(
                    "Ollama-compatible base URL [{DEFAULT_BASE_URL}]: "
                ))
                .await?
            else {
                return Ok(None);
            };
            let base_url = defaulted(&answer, DEFAULT_BASE_URL);
            if valid_http_url(&base_url) {
                return Ok(Some(base_url.trim_end_matches('/').to_owned()));
            }
            self.io.output("Enter a valid http:// or https:// URL.");
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

    async fn ask_context_window(&self) -> Result<Option<u64>, String> {
        loop {
            let Some(answer) = self
                .io
                .prompt(&format!("Context window [{DEFAULT_CONTEXT_WINDOW}]: "))
                .await?
            else {
                return Ok(None);
            };
            let value = answer.trim();
            if value.is_empty() {
                return Ok(Some(DEFAULT_CONTEXT_WINDOW));
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

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use tempfile::tempdir;

    use super::{ModelSetupIo, ModelSetupResult, ModelSetupService};
    use flowmation_application::{ConfigService, ModelSetup};
    use flowmation_codex::CodexModel;

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
                base_url: "http://localhost:11434".to_owned(),
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
}
