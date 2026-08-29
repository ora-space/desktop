//! Supplies the running Ora Desktop product version for plugin host-dependency checks.
//!
//! Version *source* is separated from constraint *matching*: this module only names the product
//! version the host is running. `ora-plugin-manifest` owns SemVer requirement evaluation so
//! marketplace install, local import, update preparation, and startup discovery cannot drift.

use semver::Version;

/// Supplies the running Ora Desktop product version used to evaluate `[dependencies].ora`.
///
/// Implementations must return the Desktop application product version, never a workspace crate
/// version such as `0.0.0`. The same value must be observed by marketplace install, local import,
/// update preparation, and startup discovery so a package cannot pass one gate and fail another.
/// Implementations are constructed at the application bootstrap boundary and injected; they must
/// not read process environment, Cargo package metadata, or global statics internally.
pub trait HostProductVersion {
    /// Returns the Desktop product version for this host process.
    fn product_version(&self) -> &Version;
}

/// The Desktop product version injected at the application bootstrap boundary.
///
/// Production wiring parses `ora-desktop`'s package version. Tests construct an explicit value so
/// they never mutate process environment to simulate a different host.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesktopProductVersion(Version);

impl DesktopProductVersion {
    /// Wraps an already-parsed Desktop product version.
    pub fn new(version: Version) -> Self {
        Self(version)
    }

    /// Parses a Desktop product version string supplied by the application crate.
    pub fn parse(value: &str) -> Result<Self, semver::Error> {
        Version::parse(value).map(Self)
    }
}

impl HostProductVersion for DesktopProductVersion {
    fn product_version(&self) -> &Version {
        &self.0
    }
}

/// Bounded host/plugin version mismatch shared by install, import, update, and discovery.
///
/// Public mapping may expose only these two fields. It must not grow package contents, absolute
/// data paths, raw manifest JSON, or configuration values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostVersionIncompatibility {
    actual: Version,
    required: semver::VersionReq,
}

impl HostVersionIncompatibility {
    /// Records the running host version and the requirement that rejected it.
    pub(crate) fn new(actual: Version, required: semver::VersionReq) -> Self {
        Self { actual, required }
    }

    /// Returns the running Desktop product version as text for stable error parameters.
    pub fn actual_host_version(&self) -> String {
        self.actual.to_string()
    }

    /// Returns the plugin's `[dependencies].ora` constraint as text for stable error parameters.
    pub fn required_version_constraint(&self) -> String {
        self.required.to_string()
    }
}

impl std::fmt::Display for HostVersionIncompatibility {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "plugin requires Ora Desktop {}, running {}",
            self.required, self.actual
        )
    }
}

impl std::error::Error for HostVersionIncompatibility {}

/// Rejects a parsed manifest whose `[dependencies].ora` does not match `host_version`.
///
/// Called from every plugin entry point before the package is written or activated so the four
/// paths cannot disagree about compatibility.
pub(crate) fn ensure_host_compatible(
    manifest: &ora_plugin_manifest::PluginManifest,
    host_version: &impl HostProductVersion,
) -> Result<(), HostVersionIncompatibility> {
    match manifest.ora_host_compatibility(host_version.product_version()) {
        ora_plugin_manifest::OraHostDependencyMatch::Satisfied => Ok(()),
        ora_plugin_manifest::OraHostDependencyMatch::Unsatisfied { actual, required } => {
            Err(HostVersionIncompatibility::new(actual, required))
        }
    }
}
