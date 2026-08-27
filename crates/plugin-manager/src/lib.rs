//! Discovers installed Ora plugin packages without executing plugin code, and orchestrates
//! checksum-verified installs of new plugin releases.

mod discovery;
mod install;
mod issue;
mod logo;
mod mcp;
mod skill;
mod validation;
mod webview;
mod workbench;

#[cfg(test)]
mod kind_tests;
#[cfg(test)]
mod tests;

pub use discovery::{MANIFEST_FILE_NAME, installed_root};
pub use install::{InstallError, InstalledPackage, Installer, UpdateError};
pub use issue::{PluginDiscoveryIssue, PluginDiscoveryIssueKind};
pub use mcp::{InstalledMcpDescriptor, MCP_CONFIGURATION_FILE};
pub use skill::{
    InstalledSkill, InstalledSkillDescriptor, SKILL_ASSET_DIRECTORY, SKILL_MANIFEST_FILE_NAME,
};
pub use validation::{
    INSTALLED_ENTRYPOINT, InstalledPlugin, InstalledPluginAgent,
    PluginConfigurationDeclarationValidity, PluginContribution,
};
pub use webview::InstalledWebviewDescriptor;
pub use workbench::{
    InstalledWorkbenchDescriptor, WORKBENCH_ASSET_DIRECTORY, WORKBENCH_PAGE_ENTRY,
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
    /// Discovers the selected plugin versions below `<data_dir>/plugins/installed`.
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
