use std::collections::BTreeMap;
use std::path::Path;

use flowmation_domain::config::{
    ModelConfig, ModelsConfig, PartialModelsConfig, ProviderConfig, ProviderKind,
    merge_agent_instructions,
};

fn sample_models() -> ModelsConfig {
    ModelsConfig {
        default_provider: "local".to_owned(),
        default_model: "qwen3:8b".to_owned(),
        providers: BTreeMap::from([(
            "local".to_owned(),
            ProviderConfig {
                kind: ProviderKind::Ollama,
                base_url: "http://localhost:11434".to_owned(),
                token_source: None,
                models: vec![ModelConfig {
                    name: "qwen3:8b".to_owned(),
                    context_window: 16_384,
                }],
            },
        )]),
        model_aliases: BTreeMap::new(),
    }
}

#[test]
fn default_models_config_has_unconfigured_ollama_default() {
    let config = ModelsConfig::default();

    assert_eq!(config.default_provider, "ollama");
    assert!(!config.has_configured_default_model());
}

#[test]
fn provider_kind_is_required() {
    let result = serde_json::from_str::<ProviderConfig>(
        r#"{
            "baseUrl": "http://localhost:11434",
            "models": []
        }"#,
    );

    assert!(result.is_err());
}

#[test]
fn provider_kinds_use_stable_configuration_names() -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(
        serde_json::to_string(&ProviderKind::OpenAiSubscription)?,
        "\"openai-subscription\""
    );
    assert_eq!(
        serde_json::to_string(&ProviderKind::OpenAiCompatible)?,
        "\"openai-compatible\""
    );
    Ok(())
}

#[test]
fn merges_project_model_aliases_and_validates_targets() -> Result<(), Box<dyn std::error::Error>> {
    let global = sample_models();
    let project = PartialModelsConfig {
        model_aliases: Some(BTreeMap::from([(
            "reviewer".to_owned(),
            "local/qwen3:8b".to_owned(),
        )])),
        ..PartialModelsConfig::default()
    };
    let merged = global.merge_project(Some(&project));

    merged.validate(Path::new("/global"), Path::new("/project"))?;
    assert_eq!(
        merged.resolve_model("reviewer")?.model_name,
        "qwen3:8b".to_owned()
    );
    Ok(())
}

#[test]
fn rejects_ambiguous_unqualified_models() {
    let mut config = sample_models();
    config.providers.insert(
        "remote".to_owned(),
        ProviderConfig {
            kind: ProviderKind::Ollama,
            base_url: "https://example.test".to_owned(),
            token_source: None,
            models: vec![ModelConfig {
                name: "qwen3:8b".to_owned(),
                context_window: 8_192,
            }],
        },
    );

    assert!(config.resolve_model("qwen3:8b").is_err());
}

#[test]
fn merges_global_and_project_agent_instructions() {
    assert_eq!(
        merge_agent_instructions(Some("Global"), Some("Project")),
        "## Global Instructions\n\nGlobal\n\n---\n\n## Project Instructions\n\nProject"
    );
}
