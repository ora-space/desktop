//! Explicit deployment trust and finite cleanup waits; no workload can set these through a frame.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SessionStopConfig {
    pub confirmation_timeout_ms: u64,
    pub trusted_host_id: Option<String>,
    /// The storage layer fences the previous host before this Workspace volume is remounted.
    pub single_mount_with_storage_fencing: bool,
}

impl Default for SessionStopConfig {
    fn default() -> Self {
        Self {
            confirmation_timeout_ms: 600_000,
            trusted_host_id: None,
            single_mount_with_storage_fencing: false,
        }
    }
}

impl SessionStopConfig {
    /// A missing host ID disables cross-boot host proof, rather than trusting an image machine-id.
    pub(crate) fn host(&self) -> ora_node_db::SessionStopHost {
        let mut host = ora_node_db::SessionStopHost::current_untrusted();
        host.trusted_host_id = self.trusted_host_id.clone();
        host
    }

    /// Rejects zero, overflowing, or malformed deployment policy before database migration.
    pub(crate) fn validate(&self) -> Result<(), crate::Error> {
        if self.confirmation_timeout_ms == 0
            || self.confirmation_timeout_ms > i64::MAX as u64 / 2
            || self
                .trusted_host_id
                .as_ref()
                .is_some_and(|id| id.trim().is_empty() || id.len() > 256)
        {
            return Err(crate::Error::Configuration(
                "invalid session stop policy".into(),
            ));
        }
        Ok(())
    }
}
