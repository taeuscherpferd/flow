use std::fmt::{Display, Formatter};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;
use uuid::Uuid;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("{kind} cannot be empty")]
pub struct InvalidId {
    kind: &'static str,
}

macro_rules! strong_id {
    ($name:ident, $kind:literal) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            #[must_use]
            pub fn generated() -> Self {
                Self(Uuid::new_v4().to_string())
            }

            /// Creates an ID from its persisted string representation.
            ///
            /// # Errors
            ///
            /// Returns [`InvalidId`] when `value` is empty.
            pub fn new(value: impl Into<String>) -> Result<Self, InvalidId> {
                let value = value.into();
                if value.is_empty() {
                    return Err(InvalidId { kind: $kind });
                }
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            #[must_use]
            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl Display for $name {
            fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = InvalidId;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }

        impl TryFrom<String> for $name {
            type Error = InvalidId;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

strong_id!(WorkflowRunId, "workflow run ID");
strong_id!(ScheduleId, "schedule ID");
strong_id!(ScheduleOccurrenceId, "schedule occurrence ID");
strong_id!(ScheduleNotificationId, "schedule notification ID");
strong_id!(AgentSessionId, "agent session ID");

#[cfg(test)]
mod tests {
    use super::WorkflowRunId;

    #[test]
    fn strong_ids_preserve_legacy_string_wire_format() -> Result<(), Box<dyn std::error::Error>> {
        let id = WorkflowRunId::new("run-1")?;
        let serialized = serde_json::to_string(&id)?;
        let deserialized: WorkflowRunId = serde_json::from_str(&serialized)?;

        assert_eq!(serialized, "\"run-1\"");
        assert_eq!(deserialized, id);
        Ok(())
    }

    #[test]
    fn strong_ids_reject_empty_persisted_values() {
        let result = serde_json::from_str::<WorkflowRunId>("\"\"");

        assert!(result.is_err());
    }
}
