use std::collections::BTreeMap;
use std::sync::Arc;

use flowmation_application::ModelProvider;
use flowmation_codex::{CodexProvider, OPENAI_SUBSCRIPTION_PROVIDER_NAME};
use flowmation_domain::config::ModelsConfig;
use flowmation_ollama::OllamaProvider;

pub fn create_model_providers(models: &ModelsConfig) -> BTreeMap<String, Arc<dyn ModelProvider>> {
    models
        .providers
        .iter()
        .map(|(name, config)| {
            let provider: Arc<dyn ModelProvider> = if name == OPENAI_SUBSCRIPTION_PROVIDER_NAME {
                Arc::new(CodexProvider::default())
            } else {
                Arc::new(OllamaProvider::new(&config.base_url))
            };
            (name.clone(), provider)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use flowmation_domain::config::{ModelConfig, ModelsConfig, ProviderConfig};

    use super::create_model_providers;

    #[test]
    fn creates_subscription_and_http_providers_from_the_same_factory() {
        let models = ModelsConfig {
            default_provider: "openai".to_owned(),
            default_model: "gpt-5.6".to_owned(),
            providers: BTreeMap::from([
                (
                    "openai".to_owned(),
                    ProviderConfig {
                        base_url: "codex://app-server".to_owned(),
                        models: vec![ModelConfig {
                            name: "gpt-5.6".to_owned(),
                            context_window: 1_050_000,
                        }],
                    },
                ),
                (
                    "ollama".to_owned(),
                    ProviderConfig {
                        base_url: "http://localhost:11434".to_owned(),
                        models: Vec::new(),
                    },
                ),
            ]),
            model_aliases: BTreeMap::new(),
        };

        let providers = create_model_providers(&models);

        assert_eq!(providers["openai"].id(), "openai");
        assert_eq!(providers["ollama"].id(), "ollama");
    }
}
