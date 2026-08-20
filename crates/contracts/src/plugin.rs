use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Describes the kind-specific contribution of one installed plugin, discriminated by `kind`.
///
/// The agent variant names its display name `agentDisplayName` because the contribution is
/// flattened into [`InstalledPlugin`], which already owns the top-level `displayName`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "plugin.ts")]
pub enum InstalledPluginContribution {
    Agent {
        agent_display_name: String,
        contract_version: u32,
    },
    Ui {
        contract_version: u32,
        surfaces: Vec<InstalledPluginSurface>,
    },
}

/// Describes one surface a ui plugin contributes, with its source flattened beside the identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct InstalledPluginSurface {
    pub id: String,
    pub title: String,
    #[serde(flatten)]
    #[ts(flatten)]
    pub source: InstalledPluginSurfaceSource,
}

/// Describes where a surface loads its content from, discriminated by `source`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "source",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "plugin.ts")]
pub enum InstalledPluginSurfaceSource {
    RemoteSite { entry_url: String },
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
    pub main: String,
    #[serde(flatten)]
    #[ts(flatten)]
    pub contribution: InstalledPluginContribution,
    pub enabled: bool,
    #[serde(flatten)]
    #[ts(flatten)]
    pub runtime: PluginRuntimeStatus,
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
}

/// Confirms the identifier removed after process shutdown and package deletion complete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct UninstallPluginResponse {
    pub plugin_id: String,
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    InstalledPluginContribution::export(config)?;
    InstalledPluginSurface::export(config)?;
    InstalledPluginSurfaceSource::export(config)?;
    PluginRuntimeStatus::export(config)?;
    InstalledPlugin::export(config)?;
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
    UninstallPluginResponse::export(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        InstalledPlugin, InstalledPluginContribution, InstalledPluginSurface,
        InstalledPluginSurfaceSource, ListInstalledPluginsRequest, ListInstalledPluginsResponse,
        PluginRuntimeStatus,
    };
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// Verifies the installed-plugin response preserves the package manifest field mapping.
    #[test]
    fn serializes_installed_plugin_contract() {
        let plugin = InstalledPlugin {
            id: "ora.claude-code".to_string(),
            package_name: "@ora-plugins/claude-code".to_string(),
            display_name: "Claude Code".to_string(),
            version: "0.1.0".to_string(),
            main: "dist/index.js".to_string(),
            contribution: InstalledPluginContribution::Agent {
                agent_display_name: "Claude Code".to_string(),
                contract_version: 1,
            },
            enabled: false,
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
                    "main": "dist/index.js",
                    "kind": "agent",
                    "agentDisplayName": "Claude Code",
                    "contractVersion": 1,
                    "enabled": false,
                    "runtime": "stopped"
                }]
            })
        );
    }

    /// Verifies a ui plugin flattens its surfaces and their sources onto the wire object.
    #[test]
    fn serializes_ui_plugin_contract() {
        let plugin = InstalledPlugin {
            id: "ora-space.skillhub".to_string(),
            package_name: "@ora-space/skillhub-ui".to_string(),
            display_name: "SkillHub".to_string(),
            version: "0.1.0".to_string(),
            main: "dist/index.js".to_string(),
            contribution: InstalledPluginContribution::Ui {
                contract_version: 1,
                surfaces: vec![InstalledPluginSurface {
                    id: "market".to_string(),
                    title: "SkillHub".to_string(),
                    source: InstalledPluginSurfaceSource::RemoteSite {
                        entry_url: "https://www.skillhub.cn/".to_string(),
                    },
                }],
            },
            enabled: true,
            runtime: PluginRuntimeStatus::Stopped,
        };

        let value = serde_json::to_value(&plugin).expect("ui plugin serializes");
        assert_eq!(
            value,
            json!({
                "id": "ora-space.skillhub",
                "packageName": "@ora-space/skillhub-ui",
                "displayName": "SkillHub",
                "version": "0.1.0",
                "main": "dist/index.js",
                "kind": "ui",
                "contractVersion": 1,
                "surfaces": [{
                    "id": "market",
                    "title": "SkillHub",
                    "source": "remote_site",
                    "entryUrl": "https://www.skillhub.cn/"
                }],
                "enabled": true,
                "runtime": "stopped"
            }),
        );
        assert_eq!(
            serde_json::from_value::<InstalledPlugin>(value).expect("ui plugin round-trips"),
            plugin
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

    /// Verifies lifecycle state is flattened into the installed-plugin wire object.
    #[test]
    fn serializes_running_plugin_lifecycle_state() {
        let plugin = InstalledPlugin {
            id: "ora.example".to_string(),
            package_name: "@ora/example".to_string(),
            display_name: "Example".to_string(),
            version: "1.0.0".to_string(),
            main: "dist/index.js".to_string(),
            contribution: InstalledPluginContribution::Agent {
                agent_display_name: "Example".to_string(),
                contract_version: 1,
            },
            enabled: true,
            runtime: PluginRuntimeStatus::Running,
        };

        assert_eq!(
            serde_json::to_value(plugin).expect("running plugin serializes"),
            json!({
                "id": "ora.example",
                "packageName": "@ora/example",
                "displayName": "Example",
                "version": "1.0.0",
                "main": "dist/index.js",
                "kind": "agent",
                "agentDisplayName": "Example",
                "contractVersion": 1,
                "enabled": true,
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
            main: "dist/index.js".to_string(),
            contribution: InstalledPluginContribution::Agent {
                agent_display_name: "Example".to_string(),
                contract_version: 1,
            },
            enabled: true,
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
                "main": "dist/index.js",
                "kind": "agent",
                "agentDisplayName": "Example",
                "contractVersion": 1,
                "enabled": true,
                "runtime": "failed",
                "failureReason": "process crashed"
            }),
        );
    }
}
