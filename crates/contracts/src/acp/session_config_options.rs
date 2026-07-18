use super::common::Meta;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A semantic category for a configuration option.
pub type ConfigOptionCategory = String;

/// A session configuration option.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/session_config_options.ts")]
pub struct ConfigOption {
    /// The stable option identifier.
    pub id: String,
    /// The display name.
    pub name: String,
    /// An optional option description.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub description: Option<String>,
    /// An optional semantic category.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub category: Option<ConfigOptionCategory>,
    /// The input control type.
    #[serde(rename = "type")]
    pub option_type: ConfigOptionType,
    /// The selected value.
    pub current_value: ConfigOptionCurrentValue,
    /// Values required for a select option.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub options: Option<Vec<ConfigOptionValue>>,
    /// Optional extension data.
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    #[ts(type = "Record<string, unknown>")]
    #[ts(optional)]
    pub meta: Option<Meta>,
}

/// A selectable configuration value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/session_config_options.ts")]
pub struct ConfigOptionValue {
    /// The value identifier.
    pub value: String,
    /// The display name.
    pub name: String,
    /// An optional value description.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub description: Option<String>,
    /// Optional extension data.
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    #[ts(type = "Record<string, unknown>")]
    #[ts(optional)]
    pub meta: Option<Meta>,
}

/// Parameters for the `session/set_config_option` request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/session_config_options.ts")]
pub struct SetConfigOptionParams {
    /// The target session.
    pub session_id: String,
    /// The configuration option to change.
    pub config_id: String,
    /// The new configuration value.
    pub value: ConfigOptionCurrentValue,
    /// The option type required for a boolean value.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub option_type: Option<ConfigOptionType>,
    /// Optional extension data.
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    #[ts(type = "Record<string, unknown>")]
    #[ts(optional)]
    pub meta: Option<Meta>,
}

/// The input control type for a configuration option.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "acp/session_config_options.ts")]
pub enum ConfigOptionType {
    /// A selection from named values.
    Select,
    /// A boolean toggle.
    Boolean,
}

/// The current value of a configuration option.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(export_to = "acp/session_config_options.ts")]
pub enum ConfigOptionCurrentValue {
    /// A selected value identifier.
    String(String),
    /// A boolean value.
    Boolean(bool),
}
