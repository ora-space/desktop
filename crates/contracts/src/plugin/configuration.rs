//! Plugin Setting declarations and the editor snapshot contracts derived from them.
//!
//! A plugin authors its Settings in its declaration; the host resolves stored overrides, defaults,
//! and redactions into the field list the settings editor renders, bound to a revision and a
//! declaration fingerprint so a concurrent write can be refused instead of silently overwriting a
//! declaration the editor never saw.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

/// Reports whether every required Setting has an effective type-correct value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "plugin.ts")]
pub enum PluginConfigurationCompleteness {
    Complete,
    Incomplete,
}

/// Represents the exclusive list-facing Plugin Configuration state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "plugin.ts")]
pub enum PluginConfigurationSummary {
    NotDeclared,
    Available {
        completeness: PluginConfigurationCompleteness,
    },
    Unavailable {
        error_code: String,
    },
}

/// Enumerates Setting types supported by declaration schema version one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "plugin.ts")]
pub enum PluginSettingType {
    String,
    Number,
    Boolean,
}

/// Carries one non-secret scalar override accepted by schema version one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(export_to = "plugin.ts")]
pub enum PluginSettingValue {
    String(String),
    Number(f64),
    Boolean(bool),
}

/// Describes one immutable plugin-authored Setting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct PluginSettingDeclaration {
    pub id: String,
    pub title: String,
    pub description: String,
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub setting_type: PluginSettingType,
    pub required: bool,
    pub order: Option<i64>,
    pub default: Option<PluginSettingValue>,
}

/// Identifies the source of one effective editor value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "plugin.ts")]
pub enum PluginSettingValueSource {
    Stored,
    Default,
    Absent,
}

/// Projects one Setting into an editor field without exposing raw files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct PluginSettingDetails {
    pub declaration: PluginSettingDeclaration,
    pub stored_value: Option<PluginSettingValue>,
    pub effective_value: Option<PluginSettingValue>,
    /// True when the host deliberately withholds a value used by an MCP process.
    pub redacted: bool,
    pub source: PluginSettingValueSource,
    pub value_error_code: Option<String>,
}

/// Carries one complete editor snapshot bound to a revision and declaration fingerprint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct PluginConfigurationDetails {
    pub plugin_id: String,
    pub schema_version: u32,
    pub revision: u64,
    pub declaration_fingerprint: String,
    pub settings: Vec<PluginSettingDetails>,
    pub summary: PluginConfigurationSummary,
}

/// Requests the current editor snapshot for one installed plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct GetPluginConfigurationRequest {
    pub plugin_id: String,
}

/// Returns the resolved editor snapshot without exposing its filesystem location.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct GetPluginConfigurationResponse {
    pub configuration: PluginConfigurationDetails,
}

/// Replaces every explicit override recognized by the loaded declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct SavePluginConfigurationRequest {
    pub plugin_id: String,
    pub expected_revision: u64,
    pub declaration_fingerprint: String,
    pub values: BTreeMap<String, PluginSettingValue>,
    /// Host-redacted stored values that an unchanged editor must retain.
    pub preserve_setting_ids: Vec<String>,
}

/// Returns the authoritative post-save editor snapshot and list summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct SavePluginConfigurationResponse {
    pub configuration: PluginConfigurationDetails,
}

/// Selects the explicit reset operation authorized by the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "mode",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "plugin.ts")]
pub enum ResetPluginConfigurationMode {
    ResetAll { expected_revision: u64 },
    RecoverCorrupt,
}

/// Requests Reset All or confirmed damaged-data recovery for one plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct ResetPluginConfigurationRequest {
    pub plugin_id: String,
    pub declaration_fingerprint: String,
    #[serde(flatten)]
    #[ts(flatten)]
    pub reset: ResetPluginConfigurationMode,
}

/// Returns the authoritative editor snapshot after a reset operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct ResetPluginConfigurationResponse {
    pub configuration: PluginConfigurationDetails,
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    PluginConfigurationCompleteness::export(config)?;
    PluginConfigurationSummary::export(config)?;
    PluginSettingType::export(config)?;
    PluginSettingValue::export(config)?;
    PluginSettingDeclaration::export(config)?;
    PluginSettingValueSource::export(config)?;
    PluginSettingDetails::export(config)?;
    PluginConfigurationDetails::export(config)?;
    GetPluginConfigurationRequest::export(config)?;
    GetPluginConfigurationResponse::export(config)?;
    SavePluginConfigurationRequest::export(config)?;
    SavePluginConfigurationResponse::export(config)?;
    ResetPluginConfigurationMode::export(config)?;
    ResetPluginConfigurationRequest::export(config)?;
    ResetPluginConfigurationResponse::export(config)?;
    Ok(())
}
