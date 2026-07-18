use ora_plugin_protocol::{InitializeLimits, PluginVersion};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Centralizes all filesystem, protocol, queue, and lifecycle safety budgets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginLimits {
    pub maximum_file_count: u64,
    pub maximum_file_bytes: u64,
    pub maximum_total_bytes: u64,
    pub maximum_directory_depth: usize,
    pub runtime: InitializeLimits,
}

impl Default for PluginLimits {
    fn default() -> Self {
        Self {
            maximum_file_count: 10_000,
            maximum_file_bytes: 64 * 1024 * 1024,
            maximum_total_bytes: 512 * 1024 * 1024,
            maximum_directory_depth: 64,
            runtime: InitializeLimits::v1_defaults(),
        }
    }
}

/// Names each independent lifecycle deadline instead of sharing one ambiguous timeout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDeadlines {
    pub spawn: Duration,
    pub initialize: Duration,
    pub activate: Duration,
    pub invocation: Duration,
    pub transport_cancel_write: Duration,
    pub transport_cancel_total: Duration,
    pub deactivate: Duration,
    pub exit: Duration,
    pub tree_cleanup: Duration,
    pub pipe_drain: Duration,
}

impl Default for PluginDeadlines {
    fn default() -> Self {
        Self {
            spawn: Duration::from_secs(10),
            initialize: Duration::from_secs(5),
            activate: Duration::from_secs(15),
            invocation: Duration::from_secs(60),
            transport_cancel_write: Duration::from_secs(1),
            transport_cancel_total: Duration::from_secs(5),
            deactivate: Duration::from_secs(5),
            exit: Duration::from_secs(5),
            tree_cleanup: Duration::from_secs(10),
            pipe_drain: Duration::from_secs(5),
        }
    }
}

/// Carries typed production policy and derives every managed plugin path from one data root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginManagerConfig {
    data_dir: PathBuf,
    pub host_version: PluginVersion,
    pub bun_version: PluginVersion,
    pub limits: PluginLimits,
    pub deadlines: PluginDeadlines,
    pub selection_ttl: Duration,
    pub candidate_ttl: Duration,
    pub crash_window: Duration,
    pub crash_threshold: usize,
}

impl PluginManagerConfig {
    /// Builds v1 policy around a single authoritative Ora data directory.
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            host_version: parse_static_version("0.1.0"),
            bun_version: parse_static_version("1.3.14"),
            limits: PluginLimits::default(),
            deadlines: PluginDeadlines::default(),
            selection_ttl: Duration::from_secs(5 * 60),
            candidate_ttl: Duration::from_secs(10 * 60),
            crash_window: Duration::from_secs(5 * 60),
            crash_threshold: 3,
        }
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn plugins_dir(&self) -> PathBuf {
        self.data_dir.join("plugins")
    }

    pub fn staging_dir(&self) -> PathBuf {
        self.plugins_dir().join(".staging")
    }

    pub fn trash_dir(&self) -> PathBuf {
        self.plugins_dir().join(".trash")
    }

    pub fn plugin_system_dir(&self) -> PathBuf {
        self.data_dir.join("plugin-system")
    }

    pub fn plugin_data_dir(&self) -> PathBuf {
        self.data_dir.join("plugin-data")
    }

    pub fn plugin_runtime_dir(&self) -> PathBuf {
        self.data_dir.join("plugin-runtime")
    }
}

/// Parses compile-time constants while keeping constructors free of fallible defaults.
fn parse_static_version(value: &str) -> PluginVersion {
    PluginVersion::parse(value)
        .unwrap_or_else(|error| panic!("static plugin-manager version must be valid: {error}"))
}

#[cfg(test)]
mod tests {
    use super::PluginManagerConfig;
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;

    /// Derives the immutable code, mutable data, state, trash, staging, and runtime roots separately.
    #[test]
    fn derives_managed_layout_from_data_root() {
        let config = PluginManagerConfig::new(PathBuf::from("ora-data"));
        assert_eq!(
            config.plugins_dir(),
            PathBuf::from("ora-data").join("plugins")
        );
        assert_eq!(
            config.plugin_data_dir(),
            PathBuf::from("ora-data").join("plugin-data")
        );
        assert_eq!(
            config.plugin_system_dir(),
            PathBuf::from("ora-data").join("plugin-system")
        );
    }
}
