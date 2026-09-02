//! Catalog and configuration ports for Session MCP resolution.

use crate::plugin::PluginApi;
use ora_contracts::ListInstalledPluginsRequest;
use ora_domain::{AgentRef, PluginId};
use ora_plugin_config::{
    CompiledMcpConfiguration, ConfigurationCompleteness, ConfigurationSummary, SettingValue,
};
use ora_plugin_manager::{PluginContribution, PluginManager};
use semver::Version;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use super::SessionMcpError;

/// One statically valid installed MCP package version.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InstalledMcpCandidate {
    pub plugin_id: PluginId,
    pub version: Version,
    pub package_root: PathBuf,
    pub configuration: CompiledMcpConfiguration,
}

/// Configuration eligibility for one installed MCP, without exposing Setting values unless complete.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum McpConfigurationEligibility {
    /// The package declares no Settings, so revision `0` is always eligible.
    NoSettings,
    /// Required Settings are present; values are used only to build the ACP snapshot.
    Complete {
        revision: u64,
        values: BTreeMap<String, SettingValue>,
    },
    /// The plugin is installed but not yet configured; it is omitted from the Effective MCP Set.
    Incomplete,
    /// The store or declaration cannot be re-read, which fails the whole setup.
    Unavailable,
}

/// Enumerates the current installed MCP snapshot.
///
/// Implementations must be side-effect free. A second call that disagrees with the first is how
/// the resolver detects a package version change mid-setup.
pub(crate) trait SessionMcpCatalog {
    fn installed_mcps(&self) -> Result<Vec<InstalledMcpCandidate>, SessionMcpError>;
}

/// Reads configuration completeness and, when complete, Setting values for one MCP plugin.
pub(crate) trait SessionMcpConfigurationSource {
    fn eligibility(
        &self,
        candidate: &InstalledMcpCandidate,
    ) -> Result<McpConfigurationEligibility, SessionMcpError>;
}

/// Host-backed catalog and configuration source shared by every Session setup path.
#[derive(Clone)]
pub(crate) struct SessionMcpHost {
    plugin_host: Arc<PluginApi>,
}

impl SessionMcpHost {
    pub(crate) fn new(plugin_host: Arc<PluginApi>) -> Self {
        Self { plugin_host }
    }

    /// Translates a Session's persisted agent identity into the plugin that owns its barrier.
    pub(crate) fn plugin_id_for_agent(&self, agent_ref: &AgentRef) -> Option<PluginId> {
        self.plugin_host
            .list(ListInstalledPluginsRequest {})
            .plugins
            .into_iter()
            .find_map(|plugin| {
                let parsed = AgentRef::parse(&plugin.name).ok()?;
                (parsed == *agent_ref)
                    .then(|| PluginId::parse(&plugin.id).ok())
                    .flatten()
            })
    }
}

impl SessionMcpCatalog for SessionMcpHost {
    fn installed_mcps(&self) -> Result<Vec<InstalledMcpCandidate>, SessionMcpError> {
        let manager = PluginManager::discover(self.plugin_host.home_directory());
        Ok(manager
            .installed_plugins()
            .iter()
            .filter_map(|plugin| {
                let PluginContribution::Mcp(descriptor) = &plugin.contributes else {
                    return None;
                };
                Some(InstalledMcpCandidate {
                    plugin_id: plugin.id.clone(),
                    version: plugin.version.clone(),
                    package_root: plugin.package_root.clone(),
                    configuration: descriptor.configuration.clone(),
                })
            })
            .collect())
    }
}

impl SessionMcpConfigurationSource for SessionMcpHost {
    fn eligibility(
        &self,
        candidate: &InstalledMcpCandidate,
    ) -> Result<McpConfigurationEligibility, SessionMcpError> {
        if candidate.configuration.settings.is_none() {
            return Ok(McpConfigurationEligibility::NoSettings);
        }
        match self
            .plugin_host
            .configuration
            .get(&candidate.plugin_id.canonical(), &candidate.package_root)
        {
            Ok(Some(details)) => match details.summary {
                ConfigurationSummary::Available {
                    completeness: ConfigurationCompleteness::Complete,
                } => Ok(McpConfigurationEligibility::Complete {
                    revision: details.revision,
                    values: details
                        .settings
                        .into_iter()
                        .filter_map(|setting| {
                            setting
                                .effective_value
                                .map(|value| (setting.declaration.id, value))
                        })
                        .collect(),
                }),
                ConfigurationSummary::Available {
                    completeness: ConfigurationCompleteness::Incomplete,
                } => Ok(McpConfigurationEligibility::Incomplete),
                ConfigurationSummary::Unavailable { .. } | ConfigurationSummary::NotDeclared => {
                    Ok(McpConfigurationEligibility::Unavailable)
                }
            },
            Ok(None) | Err(_) => Ok(McpConfigurationEligibility::Unavailable),
        }
    }
}
