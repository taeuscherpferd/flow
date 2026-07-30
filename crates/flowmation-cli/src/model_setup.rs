use std::path::PathBuf;

use async_trait::async_trait;
use flowmation_application::{ConfigService, ModelSetup};

const DEFAULT_PROVIDER: &str = "ollama";
const DEFAULT_BASE_URL: &str = "http://localhost:11434";
const DEFAULT_CONTEXT_WINDOW: u64 = 8_192;

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
        let Some(base_url) = self.ask_base_url().await? else {
            return Ok(ModelSetupResult::Cancelled);
        };
        let Some(model) = self.ask_model().await? else {
            return Ok(ModelSetupResult::Cancelled);
        };
        let Some(context_window) = self.ask_context_window().await? else {
            return Ok(ModelSetupResult::Cancelled);
        };
        let config_path = self
            .config
            .save_model_setup(&ModelSetup {
                provider: provider.clone(),
                base_url,
                model: model.clone(),
                context_window,
            })
            .await
            .map_err(|error| error.to_string())?;
        Ok(ModelSetupResult::Completed {
            config_path,
            provider,
            model,
        })
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

    async fn ask_model(&self) -> Result<Option<String>, String> {
        loop {
            let Some(answer) = self.io.prompt("Model name: ").await? else {
                return Ok(None);
            };
            let model = answer.trim();
            if !model.is_empty() {
                return Ok(Some(model.to_owned()));
            }
            self.io.output("Model name is required.");
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
    use flowmation_application::ConfigService;

    struct ScriptedSetupIo {
        answers: Mutex<VecDeque<Option<String>>>,
        output: Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl ModelSetupIo for ScriptedSetupIo {
        async fn prompt(&self, _prompt: &str) -> Result<Option<String>, String> {
            let answer = self
                .answers
                .lock()
                .map_err(|error| error.to_string())?
                .pop_front()
                .unwrap_or(None);
            Ok(answer)
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
        };

        assert_eq!(
            ModelSetupService::new(&config, &io).run().await?,
            ModelSetupResult::Cancelled
        );
        Ok(())
    }
}
