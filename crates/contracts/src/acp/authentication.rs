use std::sync::Arc;

use derive_more::{Display, From};
use serde::{Deserialize, Serialize};
use serde_with::{DefaultOnError, serde_as, skip_serializing_none};
use ts_rs::TS;

use super::serde_util::IntoOption;

/// Describes an available authentication method.
///
/// The `type` field acts as the discriminator in the serialized JSON form.
/// When no `type` is present, the method is treated as `agent`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export_to = "acp/authentication.ts")]
pub enum AuthMethod {
    /// Agent handles authentication itself.
    ///
    /// This is the default when no `type` is specified.
    #[serde(untagged)]
    Agent(AuthMethodAgent),
}

impl AuthMethod {
    /// The unique identifier for this authentication method.
    #[must_use]
    pub fn id(&self) -> &AuthMethodId {
        match self {
            Self::Agent(a) => &a.id,
        }
    }

    /// The human-readable name of this authentication method.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Agent(a) => &a.name,
        }
    }

    /// Optional description providing more details about this authentication method.
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        match self {
            Self::Agent(a) => a.description.as_deref(),
        }
    }
}

/// Agent handles authentication itself.
///
/// This is the default authentication method type.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/authentication.ts")]
pub struct AuthMethodAgent {
    /// Unique identifier for this authentication method.
    pub id: AuthMethodId,
    /// Human-readable name of the authentication method.
    pub name: String,
    /// Optional description providing more details about this authentication method.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub description: Option<String>,
}

impl AuthMethodAgent {
    /// Builds [`AuthMethodAgent`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(id: impl Into<AuthMethodId>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: None,
        }
    }

    /// Optional description providing more details about this authentication method.
    #[must_use]
    pub fn description(mut self, description: impl IntoOption<String>) -> Self {
        self.description = description.into_option();
        self
    }
}

/// Typed identifier used for auth method values on the wire.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Display, From, TS)]
#[serde(transparent)]
#[from(Arc<str>, String, &'static str)]
#[ts(type = "string", export_to = "acp/authentication.ts")]
pub struct AuthMethodId(pub Arc<str>);

impl AuthMethodId {
    /// Wraps a protocol string as a typed [`AuthMethodId`].
    #[must_use]
    pub fn new(id: impl Into<Arc<str>>) -> Self {
        Self(id.into())
    }
}

/// Request parameters for the authenticate method.
///
/// Specifies which authentication method to use.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/authentication.ts")]
pub struct AuthenticateRequest {
    /// The ID of the authentication method to use.
    /// Must be one of the methods advertised in the initialize response.
    pub method_id: AuthMethodId,
}

impl AuthenticateRequest {
    /// Builds [`AuthenticateRequest`] with the required request fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(method_id: impl Into<AuthMethodId>) -> Self {
        Self {
            method_id: method_id.into(),
        }
    }
}

/// Response to the `authenticate` method.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "acp/authentication.ts")]
pub struct AuthenticateResponse {}

impl AuthenticateResponse {
    /// Builds [`AuthenticateResponse`] with the required response fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Request parameters for the logout method.
///
/// Terminates the current authenticated session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "acp/authentication.ts")]
pub struct LogoutRequest {}

impl LogoutRequest {
    /// Builds [`LogoutRequest`] with the required request fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Response to the `logout` method.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "acp/authentication.ts")]
pub struct LogoutResponse {}

impl LogoutResponse {
    /// Builds [`LogoutResponse`] with the required response fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    AuthMethod::export(config)?;
    AuthMethodAgent::export(config)?;
    AuthMethodId::export(config)?;
    AuthenticateRequest::export(config)?;
    AuthenticateResponse::export(config)?;
    LogoutRequest::export(config)?;
    LogoutResponse::export(config)?;
    Ok(())
}
