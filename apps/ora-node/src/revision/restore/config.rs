//! Deployment owns the restore cutoff; untrusted session input cannot extend it.
use super::RestorePolicy;
use serde::{Deserialize, Serialize};
use std::{
    io,
    time::{Duration, Instant},
};

/// Restore execution timing, independent of command cleanup and Cloud's startup windows.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RestoreConfig {
    /// One positive, finite budget shared by download and all restore Git steps.
    pub deadline_ms: u64,
}

/// The approved default is five minutes; all default construction derives from this value.
pub(super) const DEFAULT_DEADLINE_MS: u64 = 300_000;

impl Default for RestoreConfig {
    /// Preserves existing deployment files when the restore section is absent.
    fn default() -> Self {
        Self {
            deadline_ms: DEFAULT_DEADLINE_MS,
        }
    }
}

impl RestoreConfig {
    /// Rejects disabled or unrepresentable deadlines before the Node opens state or admits work.
    pub(crate) fn policy(&self) -> io::Result<RestorePolicy> {
        let deadline = Duration::from_millis(self.deadline_ms);
        if deadline.is_zero() || Instant::now().checked_add(deadline).is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "restore.deadline_ms must be positive, finite and representable",
            ));
        }
        Ok(RestorePolicy {
            deadline,
            ..RestorePolicy::DEFAULT
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// Missing configuration keeps five minutes; explicit zero never disables the cutoff.
    #[test]
    fn validates_restore_deadline_configuration() -> Result<(), Box<dyn std::error::Error>> {
        let default: RestoreConfig = serde_json::from_str("{}")?;
        assert_eq!(
            default.policy()?.deadline,
            Duration::from_secs(/*secs*/ 300)
        );
        let configured: RestoreConfig = serde_json::from_str(r#"{"deadline_ms":42}"#)?;
        assert_eq!(
            configured.policy()?.deadline,
            Duration::from_millis(/*millis*/ 42)
        );
        assert!(RestoreConfig { deadline_ms: 0 }.policy().is_err());
        for invalid in [
            r#"{"deadline_ms":-1}"#,
            r#"{"deadline_ms":1.5}"#,
            r#"{"deadline_ms":1e999}"#,
            r#"{"deadline_ms":"Infinity"}"#,
        ] {
            assert!(serde_json::from_str::<RestoreConfig>(invalid).is_err());
        }
        Ok(())
    }
}
