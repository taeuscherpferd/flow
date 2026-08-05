use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolEffect {
    Read,
    Write,
    Command,
    External,
    Schedule,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolPermissionMode {
    #[default]
    Effect,
    SelfManaged,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolResult {
    pub ok: bool,
    pub content: String,
}

impl ToolResult {
    #[must_use]
    pub fn success(content: impl Into<String>) -> Self {
        Self {
            ok: true,
            content: content.into(),
        }
    }

    #[must_use]
    pub fn failure(content: impl Into<String>) -> Self {
        Self {
            ok: false,
            content: content.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ToolEffect, ToolPermissionMode};

    #[test]
    fn tool_effects_and_permission_modes_preserve_legacy_values()
    -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(
            serde_json::to_string(&ToolEffect::Schedule)?,
            "\"schedule\""
        );
        assert_eq!(
            serde_json::to_string(&ToolPermissionMode::SelfManaged)?,
            "\"self-managed\""
        );
        Ok(())
    }
}
