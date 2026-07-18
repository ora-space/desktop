use derive_more::{Display, From};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use ts_rs::TS;

/// A unique identifier for a conversation session between a client and agent.
///
/// Sessions maintain their own context, conversation history, and state,
/// allowing multiple independent interactions with the same agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Display, From, TS)]
#[serde(transparent)]
#[from(Arc<str>, String, &'static str)]
#[ts(type = "string", export_to = "acp/common.ts")]
pub struct SessionId(pub Arc<str>);

impl SessionId {
    /// Wraps a protocol string as a typed [`SessionId`].
    #[must_use]
    pub fn new(id: impl Into<Arc<str>>) -> Self {
        Self(id.into())
    }
}

/// Represents a successful payload or capability marker whose wire representation is `{}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "acp/common.ts")]
pub struct EmptyObject {}

/// An environment variable to set when launching an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/common.ts")]
pub struct EnvVariable {
    /// The name of the environment variable.
    pub name: String,
    /// The value to set for the environment variable.
    pub value: String,
}

impl EnvVariable {
    /// Builds [`EnvVariable`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    SessionId::export(config)?;
    EmptyObject::export(config)?;
    EnvVariable::export(config)?;
    Ok(())
}
