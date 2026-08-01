use flowmation_application::ProviderError;
use serde_json::Value;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexAccountStatus {
    pub account_type: Option<String>,
    pub plan_type: Option<String>,
    pub requires_openai_auth: bool,
}

impl CodexAccountStatus {
    #[must_use]
    pub fn uses_chatgpt_subscription(&self) -> bool {
        matches!(
            self.account_type.as_deref(),
            Some("chatgpt" | "personalAccessToken")
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexModel {
    pub id: String,
    pub display_name: String,
    pub is_default: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexDeviceLogin {
    pub login_id: String,
    pub verification_url: String,
    pub user_code: String,
}

pub(crate) fn parse_account_status(result: &Value) -> Result<CodexAccountStatus, ProviderError> {
    let account_type = match result.pointer("/account/type") {
        Some(value) => Some(
            value
                .as_str()
                .ok_or_else(|| {
                    ProviderError::InvalidResponse("Codex account type was not a string".to_owned())
                })?
                .to_owned(),
        ),
        None => None,
    };
    let plan_type = result
        .pointer("/account/planType")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let requires_openai_auth = result
        .get("requiresOpenaiAuth")
        .and_then(Value::as_bool)
        .ok_or_else(|| {
            ProviderError::InvalidResponse(
                "Codex account response omitted requiresOpenaiAuth".to_owned(),
            )
        })?;
    Ok(CodexAccountStatus {
        account_type,
        plan_type,
        requires_openai_auth,
    })
}

pub(crate) fn parse_models_page(
    result: &Value,
) -> Result<(Vec<CodexModel>, Option<String>), ProviderError> {
    let entries = result
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ProviderError::InvalidResponse("Codex model list omitted data".to_owned())
        })?;
    let models = entries
        .iter()
        .map(|entry| {
            let id = entry
                .get("id")
                .or_else(|| entry.get("model"))
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    ProviderError::InvalidResponse(
                        "Codex model list contained an entry without an id".to_owned(),
                    )
                })?;
            let display_name = entry
                .get("displayName")
                .and_then(Value::as_str)
                .unwrap_or(id);
            Ok(CodexModel {
                id: id.to_owned(),
                display_name: display_name.to_owned(),
                is_default: entry
                    .get("isDefault")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect::<Result<Vec<_>, ProviderError>>()?;
    let next_cursor = result
        .get("nextCursor")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok((models, next_cursor))
}

pub(crate) fn parse_device_login(result: &Value) -> Result<CodexDeviceLogin, ProviderError> {
    let field = |name: &str| {
        result
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                ProviderError::InvalidResponse(format!(
                    "Codex device login response omitted {name}"
                ))
            })
    };
    Ok(CodexDeviceLogin {
        login_id: field("loginId")?,
        verification_url: field("verificationUrl")?,
        user_code: field("userCode")?,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{parse_account_status, parse_device_login, parse_models_page};

    #[test]
    fn parses_account_and_model_discovery_responses() -> Result<(), Box<dyn std::error::Error>> {
        let account = parse_account_status(&json!({
            "account": { "type": "chatgpt", "planType": "plus" },
            "requiresOpenaiAuth": true
        }))?;
        assert!(account.uses_chatgpt_subscription());
        assert_eq!(account.plan_type.as_deref(), Some("plus"));

        let (models, cursor) = parse_models_page(&json!({
            "data": [{
                "id": "gpt-5.6",
                "displayName": "GPT-5.6",
                "isDefault": true
            }],
            "nextCursor": "next"
        }))?;
        assert_eq!(models[0].id, "gpt-5.6");
        assert!(models[0].is_default);
        assert_eq!(cursor.as_deref(), Some("next"));
        Ok(())
    }

    #[test]
    fn distinguishes_subscription_authentication_from_api_billing()
    -> Result<(), Box<dyn std::error::Error>> {
        for account_type in ["chatgpt", "personalAccessToken"] {
            let account = parse_account_status(&json!({
                "account": { "type": account_type },
                "requiresOpenaiAuth": true
            }))?;
            assert!(account.uses_chatgpt_subscription());
        }
        for account_type in ["apiKey", "amazonBedrock"] {
            let account = parse_account_status(&json!({
                "account": { "type": account_type },
                "requiresOpenaiAuth": true
            }))?;
            assert!(!account.uses_chatgpt_subscription());
        }
        Ok(())
    }

    #[test]
    fn parses_device_login_instructions() -> Result<(), Box<dyn std::error::Error>> {
        let login = parse_device_login(&json!({
            "type": "chatgptDeviceCode",
            "loginId": "login-1",
            "verificationUrl": "https://auth.openai.com/codex/device",
            "userCode": "ABCD-1234"
        }))?;
        assert_eq!(login.login_id, "login-1");
        assert_eq!(login.user_code, "ABCD-1234");
        Ok(())
    }
}
