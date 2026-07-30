use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::JsonValue;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum WorkflowSchema {
    String {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(rename = "enum", default, skip_serializing_if = "Option::is_none")]
        allowed_values: Option<Vec<String>>,
        #[serde(rename = "minLength", default, skip_serializing_if = "Option::is_none")]
        min_length: Option<usize>,
    },
    Number {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        minimum: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        maximum: Option<f64>,
    },
    Boolean {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    Array {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        items: Box<Self>,
    },
    Object {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        properties: BTreeMap<String, Self>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        required: Vec<String>,
        #[serde(
            rename = "additionalProperties",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        additional_properties: Option<bool>,
    },
}

impl WorkflowSchema {
    #[must_use]
    pub const fn is_valid_root(&self) -> bool {
        matches!(self, Self::String { .. } | Self::Object { .. })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchemaValidationResult {
    pub valid: bool,
    pub errors: Vec<String>,
}

#[must_use]
pub fn validate_schema(schema: &WorkflowSchema, value: &JsonValue) -> SchemaValidationResult {
    let mut errors = Vec::new();
    validate_at(schema, value, "input", &mut errors);
    SchemaValidationResult {
        valid: errors.is_empty(),
        errors,
    }
}

fn validate_at(schema: &WorkflowSchema, value: &JsonValue, path: &str, errors: &mut Vec<String>) {
    match schema {
        WorkflowSchema::String {
            allowed_values,
            min_length,
            ..
        } => validate_string(value, path, allowed_values.as_deref(), *min_length, errors),
        WorkflowSchema::Number {
            minimum, maximum, ..
        } => validate_number(value, path, *minimum, *maximum, errors),
        WorkflowSchema::Boolean { .. } => {
            if !value.is_boolean() {
                errors.push(format!("{path} must be a boolean."));
            }
        }
        WorkflowSchema::Array { items, .. } => {
            let Some(values) = value.as_array() else {
                errors.push(format!("{path} must be an array."));
                return;
            };
            for (index, item) in values.iter().enumerate() {
                validate_at(items, item, &format!("{path}[{index}]"), errors);
            }
        }
        WorkflowSchema::Object {
            properties,
            required,
            additional_properties,
            ..
        } => validate_object(
            value,
            path,
            properties,
            required,
            *additional_properties,
            errors,
        ),
    }
}

fn validate_string(
    value: &JsonValue,
    path: &str,
    allowed_values: Option<&[String]>,
    min_length: Option<usize>,
    errors: &mut Vec<String>,
) {
    let Some(value) = value.as_str() else {
        errors.push(format!("{path} must be a string."));
        return;
    };
    if let Some(allowed_values) = allowed_values
        && !allowed_values.iter().any(|allowed| allowed == value)
    {
        errors.push(format!(
            "{path} must be one of: {}.",
            allowed_values.join(", ")
        ));
    }
    if let Some(min_length) = min_length
        && value.encode_utf16().count() < min_length
    {
        errors.push(format!(
            "{path} must contain at least {min_length} characters."
        ));
    }
}

fn validate_number(
    value: &JsonValue,
    path: &str,
    minimum: Option<f64>,
    maximum: Option<f64>,
    errors: &mut Vec<String>,
) {
    let Some(value) = value.as_f64().filter(|value| value.is_finite()) else {
        errors.push(format!("{path} must be a finite number."));
        return;
    };
    if let Some(minimum) = minimum
        && value < minimum
    {
        errors.push(format!("{path} must be at least {minimum}."));
    }
    if let Some(maximum) = maximum
        && value > maximum
    {
        errors.push(format!("{path} must be at most {maximum}."));
    }
}

fn validate_object(
    value: &JsonValue,
    path: &str,
    properties: &BTreeMap<String, WorkflowSchema>,
    required: &[String],
    additional_properties: Option<bool>,
    errors: &mut Vec<String>,
) {
    let Some(value) = value.as_object() else {
        errors.push(format!("{path} must be an object."));
        return;
    };
    for required_property in required {
        if !value.contains_key(required_property) {
            errors.push(format!("{path}.{required_property} is required."));
        }
    }
    for (key, child_value) in value {
        let Some(child_schema) = properties.get(key) else {
            if additional_properties == Some(false) {
                errors.push(format!("{path}.{key} is not allowed."));
            }
            continue;
        };
        validate_at(child_schema, child_value, &format!("{path}.{key}"), errors);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use crate::JsonValue;

    use super::{WorkflowSchema, validate_schema};

    fn person_schema() -> WorkflowSchema {
        WorkflowSchema::Object {
            description: None,
            properties: BTreeMap::from([
                (
                    "age".to_owned(),
                    WorkflowSchema::Number {
                        description: None,
                        minimum: Some(18.0),
                        maximum: Some(120.0),
                    },
                ),
                (
                    "name".to_owned(),
                    WorkflowSchema::String {
                        description: None,
                        allowed_values: None,
                        min_length: Some(2),
                    },
                ),
                (
                    "roles".to_owned(),
                    WorkflowSchema::Array {
                        description: None,
                        items: Box::new(WorkflowSchema::String {
                            description: None,
                            allowed_values: Some(vec!["admin".to_owned(), "user".to_owned()]),
                            min_length: None,
                        }),
                    },
                ),
            ]),
            required: vec!["name".to_owned()],
            additional_properties: Some(false),
        }
    }

    #[test]
    fn validates_the_workflow_schema_subset_and_preserves_error_messages() {
        let result = validate_schema(
            &person_schema(),
            &json!({"age": 12, "extra": true, "roles": ["guest"]}),
        );

        assert!(!result.valid);
        assert_eq!(
            result.errors,
            vec![
                "input.name is required.",
                "input.age must be at least 18.",
                "input.extra is not allowed.",
                "input.roles[0] must be one of: admin, user.",
            ]
        );
    }

    #[test]
    fn accepts_valid_nested_workflow_input() {
        let result = validate_schema(
            &person_schema(),
            &json!({"name": "Ada", "age": 37, "roles": ["admin"]}),
        );

        assert!(result.valid);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn min_length_uses_javascript_utf16_code_units() {
        let schema = WorkflowSchema::String {
            description: None,
            allowed_values: None,
            min_length: Some(2),
        };

        assert!(validate_schema(&schema, &json!("😀")).valid);
    }

    #[test]
    fn schema_json_round_trips_the_legacy_shape() -> Result<(), Box<dyn std::error::Error>> {
        let source = r#"{"type":"object","properties":{"name":{"type":"string","minLength":1}},"required":["name"],"additionalProperties":false}"#;
        let schema: WorkflowSchema = serde_json::from_str(source)?;
        let serialized = serde_json::to_value(schema)?;

        assert_eq!(serialized, serde_json::from_str::<JsonValue>(source)?);
        Ok(())
    }
}
