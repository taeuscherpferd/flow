use flowmation_domain::config::{ConfigError, ModelsConfig};

pub use flowmation_domain::config::{ModelReference, ResolvedModel};

#[must_use]
pub fn list_model_references(models: &ModelsConfig) -> Vec<ModelReference> {
    models.list_model_references()
}

pub fn resolve_model(
    models: &ModelsConfig,
    requested_spec: &str,
) -> Result<ResolvedModel, ConfigError> {
    models.resolve_model(requested_spec)
}
