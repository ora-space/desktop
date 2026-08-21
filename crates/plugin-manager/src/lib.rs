//! Discovers installed Ora plugin packages and orchestrates new plugin installs.

mod discovery;
mod install;
mod issue;
mod logo;
mod validation;

#[cfg(test)]
mod tests;

pub use install::{InstallError, Installer};
pub use issue::{PluginDiscoveryIssue, PluginDiscoveryIssueKind};
pub use validation::{
    InstalledPlugin, InstalledPluginAgent, PluginContribution, PluginEngines, PluginPackageType,
};

use std::path::Path;

/// The maximum number of bytes read from one plugin package manifest.
pub const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

/// Owns one immutable startup snapshot of installed plugins and discovery problems.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginManager {
    installed_plugins: Vec<InstalledPlugin>,
    discovery_issues: Vec<PluginDiscoveryIssue>,
}

impl PluginManager {
    /// Discovers direct child plugin packages below `<data_dir>/plugins`.
    pub fn discover(data_dir: impl AsRef<Path>) -> Self {
        let discovery::PluginDiscovery {
            installed_plugins,
            discovery_issues,
        } = discovery::discover(data_dir.as_ref());

        Self {
            installed_plugins,
            discovery_issues,
        }
    }

    /// Returns the valid installed plugins in stable identifier order.
    pub fn installed_plugins(&self) -> &[InstalledPlugin] {
        &self.installed_plugins
    }

    /// Returns non-fatal problems encountered while building the snapshot.
    pub fn discovery_issues(&self) -> &[PluginDiscoveryIssue] {
        &self.discovery_issues
    }
}
