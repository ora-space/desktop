use std::path::PathBuf;
use std::time::Duration;

const DEFAULT_REQUEST_TIMEOUT_SECONDS: u64 = 5;

/// Carries the filesystem and timeout settings used to launch bundled plugins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginManagerConfig {
    pub data_dir: PathBuf,
    pub request_timeout: Duration,
}

impl PluginManagerConfig {
    /// Builds plugin-manager configuration with the default request timeout.
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            request_timeout: Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECONDS),
        }
    }

    /// Builds plugin-manager configuration with an explicit request timeout.
    pub fn with_request_timeout(data_dir: impl Into<PathBuf>, request_timeout: Duration) -> Self {
        Self {
            data_dir: data_dir.into(),
            request_timeout,
        }
    }
}
