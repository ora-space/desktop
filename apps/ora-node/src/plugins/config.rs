//! Deployment-owned download timing for plugin artifacts; remote commands cannot change it.
use ora_utils::http::DownloadOptions;
use serde::{Deserialize, Deserializer, Serialize, de::Error};
use std::time::Duration;

/// Bounded per-attempt timing; private fields keep programmatic construction within the same policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PluginConfig {
    #[serde(deserialize_with = "download_timeout")]
    download_timeout_seconds: u64,
}

impl Default for PluginConfig {
    /// Preserves the existing one-minute attempts and 250-second overall budget.
    fn default() -> Self {
        Self {
            download_timeout_seconds: 60,
        }
    }
}

impl PluginConfig {
    /// Extends only transfer timing while preserving connection, retry and disk bounds.
    pub(super) fn download_options(self) -> DownloadOptions {
        let mut options = DownloadOptions::default();
        options.connect_timeout = Some(Duration::from_secs(10));
        options.per_attempt_timeout = Some(Duration::from_secs(self.download_timeout_seconds));
        // Three full attempts plus bounded retry/connect overhead preserve the default total.
        options.total_timeout = Some(Duration::from_secs(self.download_timeout_seconds * 3 + 70));
        options.max_retries = 2;
        options.max_bytes = Some(512 * 1024 * 1024);
        options
    }
}

/// Rejects invalid deployment timing while parsing, before Node acquires any runtime resources.
fn download_timeout<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    let seconds = u64::deserialize(deserializer)?;
    if !(10..=1200).contains(&seconds) {
        return Err(D::Error::custom(
            "plugin download timeout must be between 10 and 1200 seconds",
        ));
    }
    Ok(seconds)
}
