use crate::DomainModelError;
use serde::{Deserialize, Serialize};

/// Identifies one agent provider Ora can bind a session to.
///
/// The value is the provider's namespaced package id, which is exactly what persistence has
/// always stored. It is deliberately an open string rather than a closed set: agents arrive with
/// installed plugins, so a reference Ora does not recognize means "that provider is not installed
/// right now", which is an ordinary runtime state rather than corrupt data.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AgentRef(String);

impl AgentRef {
    /// Validates an agent identity before it crosses the domain boundary.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, DomainModelError> {
        let normalized = value.as_ref().trim();
        if normalized.is_empty() {
            return Err(DomainModelError::InvalidAgentRef(value.as_ref().to_owned()));
        }
        Ok(Self(normalized.to_owned()))
    }

    /// Returns the namespaced identity used for persistence and contract mapping.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AgentRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl TryFrom<String> for AgentRef {
    type Error = DomainModelError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<AgentRef> for String {
    fn from(value: AgentRef) -> Self {
        value.0
    }
}
