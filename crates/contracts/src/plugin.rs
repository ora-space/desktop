use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

/// Represents whether the installed package and its immutable declaration are usable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "validity",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "plugin.ts")]
pub enum PluginInstallationValidity {
    Valid,
    InvalidDeclaration { error_code: String },
}

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

/// Describes the single agent contributed by an installed agent plugin package.
///
/// The agent carries no id: one package provides exactly one agent, identified by the package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct InstalledPluginAgent {
    pub display_name: String,
    pub contract_version: u32,
}

/// Represents the process-scoped lifecycle of one installed plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "runtime",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "plugin.ts")]
pub enum PluginRuntimeStatus {
    Stopped,
    Starting,
    Running,
    Failed { failure_reason: String },
}

/// Describes one installed plugin discovered from its package manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct InstalledPlugin {
    pub id: String,
    pub package_name: String,
    pub display_name: String,
    pub version: String,
    pub kind: String,
    pub main: String,
    pub agent: InstalledPluginAgent,
    pub enabled: bool,
    /// Security-validated SVG source for the package icon, absent when the package ships none.
    ///
    /// The icon travels as inline source instead of a filesystem path because the webview cannot
    /// read the plugin directory; surfaces render it from a `data:` URL and fall back to a
    /// generic mark when it is absent.
    pub logo: Option<String>,
    pub installation_validity: PluginInstallationValidity,
    pub configuration: PluginConfigurationSummary,
    #[serde(flatten)]
    #[ts(flatten)]
    pub runtime: PluginRuntimeStatus,
}

/// Describes one marketplace plugin listed by the cached registry index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct AvailablePlugin {
    pub id: String,
    pub name: String,
    pub namespace: String,
    pub version: String,
    pub description: String,
    /// Security-validated SVG source for the marketplace icon, absent when none is published.
    pub logo: Option<String>,
}

/// Requests the cached marketplace registry index used to populate the plugin catalog.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct ListAvailablePluginsRequest {}

/// Returns the marketplace plugins cached in the registry index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct ListAvailablePluginsResponse {
    pub updated_at: i64,
    pub plugins: Vec<AvailablePlugin>,
}

/// Requests a marketplace source sync followed by an atomic registry-index rebuild.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct SyncAvailablePluginsRequest {}

/// Returns the registry index rebuilt immediately after a marketplace sync succeeds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct SyncAvailablePluginsResponse {
    pub updated_at: i64,
    pub plugins: Vec<AvailablePlugin>,
}

/// Requests the immutable startup snapshot of installed plugin packages.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct ListInstalledPluginsRequest {}

/// Returns every valid installed plugin in stable identifier order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct ListInstalledPluginsResponse {
    pub plugins: Vec<InstalledPlugin>,
}

/// Requests durable eligibility for one installed plugin without starting its process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct EnablePluginRequest {
    pub plugin_id: String,
}

/// Returns the enabled plugin snapshot observed after persistence succeeds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct EnablePluginResponse {
    pub plugin: InstalledPlugin,
}

/// Requests persistent ineligibility for one installed plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct DisablePluginRequest {
    pub plugin_id: String,
}

/// Returns the stopped and disabled plugin snapshot after persistence succeeds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct DisablePluginResponse {
    pub plugin: InstalledPlugin,
}

/// Requests explicit filesystem discovery and state reconciliation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct ScanPluginsRequest {}

/// Returns the refreshed installed-plugin snapshot produced by an explicit scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct ScanPluginsResponse {
    pub plugins: Vec<InstalledPlugin>,
}

/// Requests process activation for one enabled plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct ActivatePluginRequest {
    pub plugin_id: String,
}

/// Returns the immediate starting or already-running plugin snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct ActivatePluginResponse {
    pub plugin: InstalledPlugin,
}

/// Requests process shutdown for one plugin without changing durable eligibility.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct StopPluginRequest {
    pub plugin_id: String,
}

/// Returns the stopped plugin snapshot after process exit is confirmed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct StopPluginResponse {
    pub plugin: InstalledPlugin,
}

/// Requests complete removal of one plugin package and its durable lifecycle state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct UninstallPluginRequest {
    pub plugin_id: String,
    pub data_disposition: PluginDataDisposition,
}

/// Selects whether uninstall retains or deletes host-owned plugin data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "plugin.ts")]
pub enum PluginDataDisposition {
    Delete,
    Retain,
}

/// Confirms the identifier removed after process shutdown and package deletion complete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct UninstallPluginResponse {
    pub plugin_id: String,
}

/// Requests installation of one marketplace plugin by its registry identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct InstallPluginRequest {
    pub plugin_id: String,
}

/// Confirms the identifier installed after download, verification, and extraction complete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct InstallPluginResponse {
    pub plugin_id: String,
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
    PluginInstallationValidity::export(config)?;
    PluginConfigurationCompleteness::export(config)?;
    PluginConfigurationSummary::export(config)?;
    PluginSettingType::export(config)?;
    PluginSettingValue::export(config)?;
    PluginSettingDeclaration::export(config)?;
    PluginSettingValueSource::export(config)?;
    PluginSettingDetails::export(config)?;
    PluginConfigurationDetails::export(config)?;
    InstalledPluginAgent::export(config)?;
    PluginRuntimeStatus::export(config)?;
    InstalledPlugin::export(config)?;
    AvailablePlugin::export(config)?;
    ListAvailablePluginsRequest::export(config)?;
    ListAvailablePluginsResponse::export(config)?;
    SyncAvailablePluginsRequest::export(config)?;
    SyncAvailablePluginsResponse::export(config)?;
    ListInstalledPluginsRequest::export(config)?;
    ListInstalledPluginsResponse::export(config)?;
    EnablePluginRequest::export(config)?;
    EnablePluginResponse::export(config)?;
    DisablePluginRequest::export(config)?;
    DisablePluginResponse::export(config)?;
    ScanPluginsRequest::export(config)?;
    ScanPluginsResponse::export(config)?;
    ActivatePluginRequest::export(config)?;
    ActivatePluginResponse::export(config)?;
    StopPluginRequest::export(config)?;
    StopPluginResponse::export(config)?;
    UninstallPluginRequest::export(config)?;
    PluginDataDisposition::export(config)?;
    UninstallPluginResponse::export(config)?;
    InstallPluginRequest::export(config)?;
    InstallPluginResponse::export(config)?;
    GetPluginConfigurationRequest::export(config)?;
    GetPluginConfigurationResponse::export(config)?;
    SavePluginConfigurationRequest::export(config)?;
    SavePluginConfigurationResponse::export(config)?;
    ResetPluginConfigurationMode::export(config)?;
    ResetPluginConfigurationRequest::export(config)?;
    ResetPluginConfigurationResponse::export(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        AvailablePlugin, InstallPluginRequest, InstallPluginResponse, InstalledPlugin,
        InstalledPluginAgent, ListAvailablePluginsRequest, ListAvailablePluginsResponse,
        ListInstalledPluginsRequest, ListInstalledPluginsResponse, PluginConfigurationSummary,
        PluginInstallationValidity, PluginRuntimeStatus, PluginSettingValue,
        ResetPluginConfigurationMode, ResetPluginConfigurationRequest,
        SavePluginConfigurationRequest, SyncAvailablePluginsRequest, SyncAvailablePluginsResponse,
    };
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::collections::BTreeMap;

    /// Verifies the installed-plugin response preserves the package manifest field mapping.
    #[test]
    fn serializes_installed_plugin_contract() {
        let plugin = InstalledPlugin {
            id: "ora.claude-code".to_string(),
            package_name: "@ora-plugins/claude-code".to_string(),
            display_name: "Claude Code".to_string(),
            version: "0.1.0".to_string(),
            kind: "agent".to_string(),
            main: "dist/index.js".to_string(),
            agent: InstalledPluginAgent {
                display_name: "Claude Code".to_string(),
                contract_version: 1,
            },
            enabled: false,
            logo: Some("<svg/>".to_string()),
            installation_validity: PluginInstallationValidity::Valid,
            configuration: PluginConfigurationSummary::NotDeclared,
            runtime: PluginRuntimeStatus::Stopped,
        };

        assert_eq!(
            serde_json::to_value(ListInstalledPluginsRequest {}).unwrap(),
            json!({})
        );
        assert_eq!(
            serde_json::to_value(ListInstalledPluginsResponse {
                plugins: vec![plugin],
            })
            .unwrap(),
            json!({
                "plugins": [{
                    "id": "ora.claude-code",
                    "packageName": "@ora-plugins/claude-code",
                    "displayName": "Claude Code",
                    "version": "0.1.0",
                    "kind": "agent",
                    "main": "dist/index.js",
                    "agent": {
                        "displayName": "Claude Code",
                        "contractVersion": 1
                    },
                    "enabled": false,
                    "logo": "<svg/>",
                    "installationValidity": { "validity": "valid" },
                    "configuration": { "state": "not_declared" },
                    "runtime": "stopped"
                }]
            })
        );
    }

    /// Configuration writes keep revision checks and reset mode tags explicit on the wire.
    #[test]
    fn serializes_plugin_configuration_write_contracts() {
        assert_eq!(
            serde_json::to_value(SavePluginConfigurationRequest {
                plugin_id: "official/weather".to_string(),
                expected_revision: 4,
                declaration_fingerprint: "sha256".to_string(),
                values: BTreeMap::from([
                    (
                        "endpoint".to_string(),
                        PluginSettingValue::String("https://api.test".to_string()),
                    ),
                    ("enabled".to_string(), PluginSettingValue::Boolean(false)),
                ]),
            })
            .unwrap(),
            json!({
                "pluginId": "official/weather",
                "expectedRevision": 4,
                "declarationFingerprint": "sha256",
                "values": {
                    "enabled": false,
                    "endpoint": "https://api.test",
                },
            })
        );
        assert_eq!(
            serde_json::to_value(ResetPluginConfigurationRequest {
                plugin_id: "official/weather".to_string(),
                declaration_fingerprint: "sha256".to_string(),
                reset: ResetPluginConfigurationMode::ResetAll {
                    expected_revision: 4,
                },
            })
            .unwrap(),
            json!({
                "pluginId": "official/weather",
                "declarationFingerprint": "sha256",
                "mode": "reset_all",
                "expectedRevision": 4,
            })
        );
        assert_eq!(
            serde_json::to_value(ResetPluginConfigurationRequest {
                plugin_id: "official/weather".to_string(),
                declaration_fingerprint: "sha256".to_string(),
                reset: ResetPluginConfigurationMode::RecoverCorrupt,
            })
            .unwrap(),
            json!({
                "pluginId": "official/weather",
                "declarationFingerprint": "sha256",
                "mode": "recover_corrupt",
            })
        );
    }

    /// Verifies an empty startup snapshot has a stable collection shape.
    #[test]
    fn serializes_empty_installed_plugin_response() {
        assert_eq!(
            serde_json::to_value(ListInstalledPluginsResponse {
                plugins: Vec::new(),
            })
            .unwrap(),
            json!({ "plugins": [] })
        );
    }

    /// Verifies the marketplace registry response carries the lightweight index metadata.
    #[test]
    fn serializes_available_plugin_response() {
        assert_eq!(
            serde_json::to_value(ListAvailablePluginsRequest {}).unwrap(),
            json!({})
        );
        assert_eq!(
            serde_json::to_value(ListAvailablePluginsResponse {
                updated_at: 1_776_244_428,
                plugins: vec![AvailablePlugin {
                    id: "official/weather".to_string(),
                    name: "weather".to_string(),
                    namespace: "official".to_string(),
                    version: "1.2.0".to_string(),
                    description: "Weather plugin".to_string(),
                    logo: None,
                }],
            })
            .unwrap(),
            json!({
                "updatedAt": 1_776_244_428,
                "plugins": [{
                    "id": "official/weather",
                    "name": "weather",
                    "namespace": "official",
                    "version": "1.2.0",
                    "description": "Weather plugin",
                    "logo": null
                }]
            })
        );
    }

    /// Verifies the marketplace sync response mirrors the rebuilt registry index wire shape.
    #[test]
    fn serializes_sync_available_plugin_response() {
        assert_eq!(
            serde_json::to_value(SyncAvailablePluginsRequest {}).unwrap(),
            json!({})
        );
        assert_eq!(
            serde_json::to_value(SyncAvailablePluginsResponse {
                updated_at: 1_776_244_428,
                plugins: Vec::new(),
            })
            .unwrap(),
            json!({
                "updatedAt": 1_776_244_428,
                "plugins": []
            })
        );
    }

    /// Verifies the install request/response wire shape for a marketplace plugin.
    #[test]
    fn serializes_install_plugin_contract() {
        assert_eq!(
            serde_json::to_value(InstallPluginRequest {
                plugin_id: "official/weather".to_string(),
            })
            .unwrap(),
            json!({ "pluginId": "official/weather" })
        );
        assert_eq!(
            serde_json::to_value(InstallPluginResponse {
                plugin_id: "official/weather".to_string(),
            })
            .unwrap(),
            json!({ "pluginId": "official/weather" })
        );
    }

    /// Verifies lifecycle state is flattened into the installed-plugin wire object.
    #[test]
    fn serializes_running_plugin_lifecycle_state() {
        let plugin = InstalledPlugin {
            id: "ora.example".to_string(),
            package_name: "@ora/example".to_string(),
            display_name: "Example".to_string(),
            version: "1.0.0".to_string(),
            kind: "agent".to_string(),
            main: "dist/index.js".to_string(),
            agent: InstalledPluginAgent {
                display_name: "Example".to_string(),
                contract_version: 1,
            },
            enabled: true,
            logo: None,
            installation_validity: PluginInstallationValidity::Valid,
            configuration: PluginConfigurationSummary::NotDeclared,
            runtime: PluginRuntimeStatus::Running,
        };

        assert_eq!(
            serde_json::to_value(plugin).expect("running plugin serializes"),
            json!({
                "id": "ora.example",
                "packageName": "@ora/example",
                "displayName": "Example",
                "version": "1.0.0",
                "kind": "agent",
                "main": "dist/index.js",
                "agent": {
                    "displayName": "Example",
                    "contractVersion": 1
                },
                "enabled": true,
                "logo": null,
                "installationValidity": { "validity": "valid" },
                "configuration": { "state": "not_declared" },
                "runtime": "running"
            }),
        );
    }

    /// Verifies failed runtime state carries its diagnostic reason beside the discriminator.
    #[test]
    fn serializes_failed_plugin_lifecycle_state() {
        let plugin = InstalledPlugin {
            id: "ora.example".to_string(),
            package_name: "@ora/example".to_string(),
            display_name: "Example".to_string(),
            version: "1.0.0".to_string(),
            kind: "agent".to_string(),
            main: "dist/index.js".to_string(),
            agent: InstalledPluginAgent {
                display_name: "Example".to_string(),
                contract_version: 1,
            },
            enabled: true,
            logo: None,
            installation_validity: PluginInstallationValidity::Valid,
            configuration: PluginConfigurationSummary::NotDeclared,
            runtime: PluginRuntimeStatus::Failed {
                failure_reason: "process crashed".to_string(),
            },
        };

        assert_eq!(
            serde_json::to_value(plugin).expect("failed plugin serializes"),
            json!({
                "id": "ora.example",
                "packageName": "@ora/example",
                "displayName": "Example",
                "version": "1.0.0",
                "kind": "agent",
                "main": "dist/index.js",
                "agent": {
                    "displayName": "Example",
                    "contractVersion": 1
                },
                "enabled": true,
                "logo": null,
                "installationValidity": { "validity": "valid" },
                "configuration": { "state": "not_declared" },
                "runtime": "failed",
                "failureReason": "process crashed"
            }),
        );
    }
}
