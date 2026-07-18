use std::sync::Arc;

use derive_more::{Display, From};
use serde::{Deserialize, Serialize};
use serde_with::{DefaultOnError, serde_as, skip_serializing_none};
use ts_rs::TS;

use super::{common::SessionId, serde_util::IntoOption};

/// Unique identifier for a session configuration option.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, From, Display, TS)]
#[serde(transparent)]
#[from(Arc<str>, String, &'static str)]
#[ts(type = "string", export_to = "acp/session_config_options.ts")]
pub struct SessionConfigId(pub Arc<str>);

impl SessionConfigId {
    /// Wraps a protocol string as a typed [`SessionConfigId`].
    #[must_use]
    pub fn new(id: impl Into<Arc<str>>) -> Self {
        Self(id.into())
    }
}

/// Unique identifier for a session configuration option value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, From, Display, TS)]
#[serde(transparent)]
#[from(Arc<str>, String, &'static str)]
#[ts(type = "string", export_to = "acp/session_config_options.ts")]
pub struct SessionConfigValueId(pub Arc<str>);

impl SessionConfigValueId {
    /// Wraps a protocol string as a typed [`SessionConfigValueId`].
    #[must_use]
    pub fn new(id: impl Into<Arc<str>>) -> Self {
        Self(id.into())
    }
}

/// Unique identifier for a session configuration option value group.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, From, Display, TS)]
#[serde(transparent)]
#[from(Arc<str>, String, &'static str)]
#[ts(type = "string", export_to = "acp/session_config_options.ts")]
pub struct SessionConfigGroupId(pub Arc<str>);

impl SessionConfigGroupId {
    /// Wraps a protocol string as a typed [`SessionConfigGroupId`].
    #[must_use]
    pub fn new(id: impl Into<Arc<str>>) -> Self {
        Self(id.into())
    }
}

/// A possible value for a session configuration option.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/session_config_options.ts")]
pub struct SessionConfigSelectOption {
    /// Unique identifier for this option value.
    pub value: SessionConfigValueId,
    /// Human-readable label for this option value.
    pub name: String,
    /// Optional description for this option value.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub description: Option<String>,
}

impl SessionConfigSelectOption {
    /// Builds [`SessionConfigSelectOption`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(value: impl Into<SessionConfigValueId>, name: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            name: name.into(),
            description: None,
        }
    }

    /// Sets or clears the optional `description` field.
    #[must_use]
    pub fn description(mut self, description: impl IntoOption<String>) -> Self {
        self.description = description.into_option();
        self
    }
}

/// A group of possible values for a session configuration option.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/session_config_options.ts")]
pub struct SessionConfigSelectGroup {
    /// Unique identifier for this group.
    pub group: SessionConfigGroupId,
    /// Human-readable label for this group.
    pub name: String,
    /// The set of option values in this group.
    #[serde_as(deserialize_as = "DefaultOnError")]
    pub options: Vec<SessionConfigSelectOption>,
}

impl SessionConfigSelectGroup {
    /// Builds [`SessionConfigSelectGroup`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(
        group: impl Into<SessionConfigGroupId>,
        name: impl Into<String>,
        options: Vec<SessionConfigSelectOption>,
    ) -> Self {
        Self {
            group: group.into(),
            name: name.into(),
            options,
        }
    }
}

/// Possible values for a session configuration option.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(untagged)]
#[ts(export_to = "acp/session_config_options.ts")]
pub enum SessionConfigSelectOptions {
    /// A flat list of options with no grouping.
    Ungrouped(Vec<SessionConfigSelectOption>),
    /// A list of options grouped under headers.
    Grouped(Vec<SessionConfigSelectGroup>),
}

impl From<Vec<SessionConfigSelectOption>> for SessionConfigSelectOptions {
    fn from(options: Vec<SessionConfigSelectOption>) -> Self {
        SessionConfigSelectOptions::Ungrouped(options)
    }
}

impl From<Vec<SessionConfigSelectGroup>> for SessionConfigSelectOptions {
    fn from(groups: Vec<SessionConfigSelectGroup>) -> Self {
        SessionConfigSelectOptions::Grouped(groups)
    }
}

/// A single-value selector (dropdown) session configuration option payload.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/session_config_options.ts")]
pub struct SessionConfigSelect {
    /// The currently selected value.
    pub current_value: SessionConfigValueId,
    /// The set of selectable options.
    pub options: SessionConfigSelectOptions,
}

impl SessionConfigSelect {
    /// Builds [`SessionConfigSelect`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(
        current_value: impl Into<SessionConfigValueId>,
        options: impl Into<SessionConfigSelectOptions>,
    ) -> Self {
        Self {
            current_value: current_value.into(),
            options: options.into(),
        }
    }
}

/// A boolean on/off toggle session configuration option payload.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/session_config_options.ts")]
pub struct SessionConfigBoolean {
    /// The current value of the boolean option.
    pub current_value: bool,
}

impl SessionConfigBoolean {
    /// Builds [`SessionConfigBoolean`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(current_value: bool) -> Self {
        Self { current_value }
    }
}

/// Semantic category for a session configuration option.
///
/// This is intended to help Clients distinguish broadly common selectors (e.g. model selector vs
/// session mode selector vs thought/reasoning level) for UX purposes (keyboard shortcuts, icons,
/// placement). It MUST NOT be required for correctness. Clients MUST handle missing or unknown
/// categories gracefully.
///
/// Category names beginning with `_` are free for custom use, like other ACP extension methods.
/// Category names that do not begin with `_` are reserved for the ACP spec.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "acp/session_config_options.ts")]
pub enum SessionConfigOptionCategory {
    /// Session mode selector.
    Mode,
    /// Model selector.
    Model,
    /// Model-related configuration parameter.
    ModelConfig,
    /// Thought/reasoning level selector.
    ThoughtLevel,
    /// Unknown / uncategorized selector.
    #[serde(untagged)]
    Other(String),
}

/// Type-specific session configuration option payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export_to = "acp/session_config_options.ts")]
pub enum SessionConfigKind {
    /// Single-value selector (dropdown).
    Select(SessionConfigSelect),
    /// Boolean on/off toggle.
    Boolean(SessionConfigBoolean),
}

/// A session configuration option selector and its current state.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/session_config_options.ts")]
pub struct SessionConfigOption {
    /// Unique identifier for the configuration option.
    pub id: SessionConfigId,
    /// Human-readable label for the option.
    pub name: String,
    /// Optional description for the Client to display to the user.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub description: Option<String>,
    /// Optional semantic category for this option (UX only).
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub category: Option<SessionConfigOptionCategory>,
    /// Type-specific fields for this configuration option.
    #[serde(flatten)]
    pub kind: SessionConfigKind,
}

impl SessionConfigOption {
    /// Builds [`SessionConfigOption`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(
        id: impl Into<SessionConfigId>,
        name: impl Into<String>,
        kind: SessionConfigKind,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: None,
            category: None,
            kind,
        }
    }

    /// Builds a select-style session configuration option with its current value and choices.
    #[must_use]
    pub fn select(
        id: impl Into<SessionConfigId>,
        name: impl Into<String>,
        current_value: impl Into<SessionConfigValueId>,
        options: impl Into<SessionConfigSelectOptions>,
    ) -> Self {
        Self::new(
            id,
            name,
            SessionConfigKind::Select(SessionConfigSelect::new(current_value, options)),
        )
    }

    /// Builds a boolean-style session configuration option with its current value.
    #[must_use]
    pub fn boolean(
        id: impl Into<SessionConfigId>,
        name: impl Into<String>,
        current_value: bool,
    ) -> Self {
        Self::new(
            id,
            name,
            SessionConfigKind::Boolean(SessionConfigBoolean::new(current_value)),
        )
    }

    /// Sets or clears the optional `description` field.
    #[must_use]
    pub fn description(mut self, description: impl IntoOption<String>) -> Self {
        self.description = description.into_option();
        self
    }

    /// Sets or clears the optional `category` field.
    #[must_use]
    pub fn category(mut self, category: impl IntoOption<SessionConfigOptionCategory>) -> Self {
        self.category = category.into_option();
        self
    }
}

/// Wraps a value-id payload so the untagged `ValueId` variant of
/// [`SessionConfigOptionValue`] serializes without a `type` discriminator on the wire.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/session_config_options.ts")]
pub struct SessionConfigOptionValueId {
    /// The value ID.
    pub value: SessionConfigValueId,
}

/// The value to set for a session configuration option.
///
/// The `type` field acts as the discriminator in the serialized JSON form.
/// When no `type` is present, the value is treated as a [`SessionConfigValueId`]
/// via the [`ValueId`](Self::ValueId) fallback variant.
///
/// The `type` discriminator describes the *shape* of the value, not the option
/// kind. For example every option kind that picks from a list of ids
/// (`select`, `radio`, …) would use [`ValueId`](Self::ValueId), while a
/// future freeform text option would get its own variant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export_to = "acp/session_config_options.ts")]
pub enum SessionConfigOptionValue {
    /// A boolean value (`type: "boolean"`).
    Boolean {
        /// The boolean value.
        value: bool,
    },
    /// A [`SessionConfigValueId`] string value.
    ///
    /// This is the default when `type` is absent on the wire. Unknown `type`
    /// values with string payloads also gracefully deserialize into this
    /// variant.
    #[serde(untagged)]
    ValueId(SessionConfigOptionValueId),
}

impl SessionConfigOptionValue {
    /// Create a value-id option value (used by `select` and other id-based option types).
    #[must_use]
    pub fn value_id(id: impl Into<SessionConfigValueId>) -> Self {
        Self::ValueId(SessionConfigOptionValueId { value: id.into() })
    }

    /// Create a boolean option value.
    #[must_use]
    pub fn boolean(val: bool) -> Self {
        Self::Boolean { value: val }
    }

    /// Return the inner [`SessionConfigValueId`] if this is a
    /// [`ValueId`](Self::ValueId) value.
    #[must_use]
    pub fn as_value_id(&self) -> Option<&SessionConfigValueId> {
        match self {
            Self::ValueId(inner) => Some(&inner.value),
            _ => None,
        }
    }

    /// Return the inner [`bool`] if this is a [`Boolean`](Self::Boolean) value.
    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Boolean { value } => Some(*value),
            _ => None,
        }
    }
}

impl From<SessionConfigValueId> for SessionConfigOptionValue {
    fn from(value: SessionConfigValueId) -> Self {
        Self::ValueId(SessionConfigOptionValueId { value })
    }
}

impl From<bool> for SessionConfigOptionValue {
    fn from(value: bool) -> Self {
        Self::Boolean { value }
    }
}

impl From<&str> for SessionConfigOptionValue {
    fn from(value: &str) -> Self {
        Self::ValueId(SessionConfigOptionValueId {
            value: SessionConfigValueId::new(value),
        })
    }
}

/// Request parameters for setting a session configuration option.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/session_config_options.ts")]
pub struct SetSessionConfigOptionRequest {
    /// The ID of the session to set the configuration option for.
    pub session_id: SessionId,
    /// The ID of the configuration option to set.
    pub config_id: SessionConfigId,
    /// The value to set, including a `type` discriminator and the raw `value`.
    ///
    /// When `type` is absent on the wire, defaults to treating the value as a
    /// [`SessionConfigValueId`] for `select` options.
    #[serde(flatten)]
    pub value: SessionConfigOptionValue,
}

impl SetSessionConfigOptionRequest {
    /// Builds [`SetSessionConfigOptionRequest`] with the required request fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(
        session_id: impl Into<SessionId>,
        config_id: impl Into<SessionConfigId>,
        value: impl Into<SessionConfigOptionValue>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            config_id: config_id.into(),
            value: value.into(),
        }
    }
}

/// Response to `session/set_config_option` method.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/session_config_options.ts")]
pub struct SetSessionConfigOptionResponse {
    /// The full set of configuration options and their current values.
    #[serde_as(deserialize_as = "DefaultOnError")]
    pub config_options: Vec<SessionConfigOption>,
}

impl SetSessionConfigOptionResponse {
    /// Builds [`SetSessionConfigOptionResponse`] with the required response fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(config_options: Vec<SessionConfigOption>) -> Self {
        Self { config_options }
    }
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    SessionConfigId::export(config)?;
    SessionConfigValueId::export(config)?;
    SessionConfigGroupId::export(config)?;
    SessionConfigSelectOption::export(config)?;
    SessionConfigSelectGroup::export(config)?;
    SessionConfigSelectOptions::export(config)?;
    SessionConfigSelect::export(config)?;
    SessionConfigBoolean::export(config)?;
    SessionConfigOptionCategory::export(config)?;
    SessionConfigKind::export(config)?;
    SessionConfigOption::export(config)?;
    SessionConfigOptionValue::export(config)?;
    SessionConfigOptionValueId::export(config)?;
    SetSessionConfigOptionRequest::export(config)?;
    SetSessionConfigOptionResponse::export(config)?;
    Ok(())
}
