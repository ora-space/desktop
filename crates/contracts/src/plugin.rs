use serde::{Deserialize, Serialize};
use ts_rs::TS;

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

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    InstalledPluginAgent::export(config)?;
    InstalledPlugin::export(config)?;
    ListInstalledPluginsRequest::export(config)?;
    ListInstalledPluginsResponse::export(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        InstalledPlugin, InstalledPluginAgent, ListInstalledPluginsRequest,
        ListInstalledPluginsResponse,
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
            kind: "agent".to_string(),
            main: "dist/index.js".to_string(),
            agent: InstalledPluginAgent {
                display_name: "Claude Code".to_string(),
                contract_version: 1,
            },
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
                    }
                }]
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
}
