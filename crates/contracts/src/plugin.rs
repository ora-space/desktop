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
    },
    Ui {
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
    RemoteSite {
        entry_url: String,
    },
    /// A page shipped inside the plugin package; the host serves it, so no URL crosses the wire.
    Panel {},
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

/// Describes one installed plugin discovered from its `orax.toml` manifest.
///
/// `id` is the canonical `<namespace>/<name>` spelling and is what every plugin request carries
/// back; `namespace` and `name` repeat the two segments so the frontend never has to split it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "plugin.ts")]
pub struct InstalledPlugin {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub display_name: String,
    pub version: String,
    pub description: String,
    pub homepage: Option<String>,
    pub license: Option<String>,
    pub main: String,
    #[serde(flatten)]
    #[ts(flatten)]
    pub contribution: InstalledPluginContribution,
    pub enabled: bool,
    /// Security-validated SVG source for the package icon, absent when the package ships none.
    ///
    /// The icon travels as inline source instead of a filesystem path because the webview cannot
    /// read the plugin directory; surfaces render it from a `data:` URL and fall back to a
    /// generic mark when it is absent.
    pub logo: Option<String>,
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
/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    InstalledPluginContribution::export(config)?;
    InstalledPluginSurface::export(config)?;
    InstalledPluginSurfaceSource::export(config)?;
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
    UninstallPluginResponse::export(config)?;
    InstallPluginRequest::export(config)?;
    InstallPluginResponse::export(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        AvailablePlugin, InstallPluginRequest, InstallPluginResponse, InstalledPlugin,
        InstalledPluginContribution, InstalledPluginSurface, InstalledPluginSurfaceSource,
        ListAvailablePluginsRequest, ListAvailablePluginsResponse, ListInstalledPluginsRequest,
        ListInstalledPluginsResponse, PluginRuntimeStatus, SyncAvailablePluginsRequest,
        SyncAvailablePluginsResponse,
    };
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// Verifies the installed-plugin response preserves the package manifest field mapping.
    #[test]
    fn serializes_installed_plugin_contract() {
        let plugin = InstalledPlugin {
            id: "official/ora.claude-code".to_string(),
            namespace: "official".to_string(),
            name: "ora.claude-code".to_string(),
            display_name: "Claude Code".to_string(),
            version: "0.1.0".to_string(),
            description: "Claude Code agent".to_string(),
            homepage: Some("https://example.com/claude-code".to_string()),
            license: Some("Apache-2.0".to_string()),
            main: "dist/index.js".to_string(),
            contribution: InstalledPluginContribution::Agent {
                agent_display_name: "Claude Code".to_string(),
            },
            enabled: false,
            logo: Some("<svg/>".to_string()),
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
                    "id": "official/ora.claude-code",
                    "namespace": "official",
                    "name": "ora.claude-code",
                    "displayName": "Claude Code",
                    "version": "0.1.0",
                    "description": "Claude Code agent",
                    "homepage": "https://example.com/claude-code",
                    "license": "Apache-2.0",
                    "main": "dist/index.js",
                    "kind": "agent",
                    "agentDisplayName": "Claude Code",
                    "enabled": false,
                    "logo": "<svg/>",
                    "runtime": "stopped"
                }]
            })
        );
    }

    /// Verifies a ui plugin flattens its surfaces and their sources onto the wire object.
    #[test]
    fn serializes_ui_plugin_contract() {
        let plugin = InstalledPlugin {
            id: "official/ora-space.skillhub".to_string(),
            namespace: "official".to_string(),
            name: "ora-space.skillhub".to_string(),
            display_name: "ora-space.skillhub".to_string(),
            version: "0.1.0".to_string(),
            description: "SkillHub marketplace".to_string(),
            homepage: None,
            license: None,
            main: "dist/index.js".to_string(),
            contribution: InstalledPluginContribution::Ui {
                surfaces: vec![InstalledPluginSurface {
                    id: "market".to_string(),
                    title: "SkillHub".to_string(),
                    source: InstalledPluginSurfaceSource::RemoteSite {
                        entry_url: "https://www.skillhub.cn/".to_string(),
                    },
                }],
            },
            enabled: true,
            logo: None,
            runtime: PluginRuntimeStatus::Stopped,
        };

        let value = serde_json::to_value(&plugin).expect("ui plugin serializes");
        assert_eq!(
            value,
            json!({
                "id": "official/ora-space.skillhub",
                "namespace": "official",
                "name": "ora-space.skillhub",
                "displayName": "ora-space.skillhub",
                "version": "0.1.0",
                "description": "SkillHub marketplace",
                "homepage": null,
                "license": null,
                "main": "dist/index.js",
                "kind": "ui",
                "surfaces": [{
                    "id": "market",
                    "title": "SkillHub",
                    "source": "remote_site",
                    "entryUrl": "https://www.skillhub.cn/"
                }],
                "enabled": true,
                "logo": null,
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
            id: "official/ora.example".to_string(),
            namespace: "official".to_string(),
            name: "ora.example".to_string(),
            display_name: "Example".to_string(),
            version: "1.0.0".to_string(),
            description: "Example agent".to_string(),
            homepage: None,
            license: None,
            main: "dist/index.js".to_string(),
            contribution: InstalledPluginContribution::Agent {
                agent_display_name: "Example".to_string(),
            },
            enabled: true,
            logo: None,
            runtime: PluginRuntimeStatus::Running,
        };

        assert_eq!(
            serde_json::to_value(plugin).expect("running plugin serializes"),
            json!({
                "id": "official/ora.example",
                "namespace": "official",
                "name": "ora.example",
                "displayName": "Example",
                "version": "1.0.0",
                "description": "Example agent",
                "homepage": null,
                "license": null,
                "main": "dist/index.js",
                "kind": "agent",
                "agentDisplayName": "Example",
                "enabled": true,
                "logo": null,
                "runtime": "running"
            }),
        );
    }

    /// Verifies failed runtime state carries its diagnostic reason beside the discriminator.
    #[test]
    fn serializes_failed_plugin_lifecycle_state() {
        let plugin = InstalledPlugin {
            id: "official/ora.example".to_string(),
            namespace: "official".to_string(),
            name: "ora.example".to_string(),
            display_name: "Example".to_string(),
            version: "1.0.0".to_string(),
            description: "Example agent".to_string(),
            homepage: None,
            license: None,
            main: "dist/index.js".to_string(),
            contribution: InstalledPluginContribution::Agent {
                agent_display_name: "Example".to_string(),
            },
            enabled: true,
            logo: None,
            runtime: PluginRuntimeStatus::Failed {
                failure_reason: "process crashed".to_string(),
            },
        };

        assert_eq!(
            serde_json::to_value(plugin).expect("failed plugin serializes"),
            json!({
                "id": "official/ora.example",
                "namespace": "official",
                "name": "ora.example",
                "displayName": "Example",
                "version": "1.0.0",
                "description": "Example agent",
                "homepage": null,
                "license": null,
                "main": "dist/index.js",
                "kind": "agent",
                "agentDisplayName": "Example",
                "enabled": true,
                "logo": null,
                "runtime": "failed",
                "failureReason": "process crashed"
            }),
        );
    }
}
