use std::collections::BTreeMap;
use std::sync::Arc;

use flowmation_application::ModelProvider;
use flowmation_codex::{CodexProvider, OPENAI_SUBSCRIPTION_PROVIDER_NAME};
use flowmation_domain::config::{ModelsConfig, ProviderKind};
use flowmation_ollama::OllamaProvider;
use flowmation_openai_compatible::OpenAiCompatibleProvider;

pub fn create_model_providers(models: &ModelsConfig) -> BTreeMap<String, Arc<dyn ModelProvider>> {
    models
        .providers
        .iter()
        .map(|(name, config)| {
            let provider: Arc<dyn ModelProvider> = if name == OPENAI_SUBSCRIPTION_PROVIDER_NAME {
                Arc::new(CodexProvider::default())
            } else {
                match config.kind {
                    ProviderKind::OpenAiSubscription => Arc::new(CodexProvider::default()),
                    ProviderKind::OpenAiCompatible => Arc::new(OpenAiCompatibleProvider::new(
                        name,
                        &config.base_url,
                        config.token_source.clone(),
                    )),
                    ProviderKind::Ollama => Arc::new(OllamaProvider::new(&config.base_url)),
                }
            };
            (name.clone(), provider)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use flowmation_domain::config::{
        CredentialSource, ModelConfig, ModelsConfig, ProviderConfig, ProviderKind,
    };

    use super::create_model_providers;

    #[test]
    fn creates_each_provider_kind_from_the_same_factory() {
        let models = ModelsConfig {
            default_provider: "openai".to_owned(),
            default_model: "gpt-5.6".to_owned(),
            providers: BTreeMap::from([
                (
                    "openai".to_owned(),
                    ProviderConfig {
                        kind: ProviderKind::OpenAiCompatible,
                        base_url: "https://api.openai.com/v1".to_owned(),
                        token_source: Some(CredentialSource::Environment {
                            name: "OPENAI_API_KEY".to_owned(),
                        }),
                        models: vec![ModelConfig {
                            name: "gpt-5.6".to_owned(),
                            context_window: 1_050_000,
                        }],
                    },
                ),
                (
                    "ollama".to_owned(),
                    ProviderConfig {
                        kind: ProviderKind::Ollama,
                        base_url: "http://localhost:11434".to_owned(),
                        token_source: None,
                        models: Vec::new(),
                    },
                ),
                (
                    "openrouter".to_owned(),
                    ProviderConfig {
                        kind: ProviderKind::OpenAiCompatible,
                        base_url: "https://openrouter.ai/api/v1".to_owned(),
                        token_source: Some(CredentialSource::Environment {
                            name: "OPENROUTER_API_KEY".to_owned(),
                        }),
                        models: Vec::new(),
                    },
                ),
            ]),
            model_aliases: BTreeMap::new(),
        };

        let providers = create_model_providers(&models);

        assert_eq!(providers["openai"].id(), "openai");
        assert!(format!("{:?}", providers["openai"]).contains("CodexProvider"));
        assert_eq!(providers["ollama"].id(), "ollama");
        assert_eq!(providers["openrouter"].id(), "openrouter");
    }
}
