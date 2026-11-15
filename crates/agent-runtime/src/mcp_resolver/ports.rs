//! Inputs for the pure resolver; package discovery and configuration storage stay with hosts.
use super::SessionMcpError;
use ora_domain::PluginId;
use ora_plugin_config::{CompiledMcpConfiguration, SettingValue};
use semver::Version;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// One statically valid installed MCP package version.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstalledMcpCandidate {
    pub plugin_id: PluginId,
    pub version: Version,
    pub package_root: PathBuf,
    pub configuration: CompiledMcpConfiguration,
}

/// Configuration eligibility for one installed MCP, without exposing Setting values unless complete.
#[derive(Clone, Eq, PartialEq)]
pub enum McpConfigurationEligibility {
    /// The package declares no Settings, so revision `0` is always eligible.
    NoSettings,
    /// Required Settings are present; values are used only to build the ACP snapshot.
    Complete {
        revision: u64,
        values: BTreeMap<String, SettingValue>,
    },
    /// The plugin is installed but not yet configured.
    ///
    /// Automatic discovery omits it, while an explicit Session permission fails setup so author
    /// intent cannot silently shrink.
    Incomplete,
    /// The store or declaration cannot be re-read, which fails the whole setup.
    Unavailable,
}

/// Enumerates the current installed MCP snapshot.
///
/// Implementations must be side-effect free. A second call that disagrees with the first is how
/// the resolver detects a package version change mid-setup.
pub trait SessionMcpCatalog {
    /// Returns current validated packages; repeated reads detect changes during resolution.
    fn installed_mcps(&self) -> Result<Vec<InstalledMcpCandidate>, SessionMcpError>;
}

/// Reads configuration completeness and, when complete, Setting values for one MCP plugin.
pub trait SessionMcpConfigurationSource {
    /// Returns values only for complete configurations, never partial snapshots.
    fn eligibility(
        &self,
        candidate: &InstalledMcpCandidate,
    ) -> Result<McpConfigurationEligibility, SessionMcpError>;
}

impl std::fmt::Debug for McpConfigurationEligibility {
    /// Values are runtime authority, so diagnostic formatting reports only eligibility/revision.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSettings => formatter.write_str("NoSettings"),
            Self::Complete { revision, .. } => formatter
                .debug_struct("Complete")
                .field("revision", revision)
                .finish_non_exhaustive(),
            Self::Incomplete => formatter.write_str("Incomplete"),
            Self::Unavailable => formatter.write_str("Unavailable"),
        }
    }
}
