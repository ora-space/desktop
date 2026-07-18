use crate::{
    CatalogEntry, IntegrityStatus, ManifestValidity, RuntimeCompatibility, RuntimeSupport,
};

/// The only persisted user intent; every other disable condition is derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UserEnablement {
    Enabled,
    Disabled,
}

/// The single admission result consumed by registry and runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectiveEnablement {
    Enabled,
    Disabled(EffectiveDisableReason),
}

/// Ordered fail-closed causes for effective disablement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EffectiveDisableReason {
    PendingRemoval,
    MissingInstallFiles,
    IntegrityMismatch,
    InvalidManifest,
    IncompatibleEngine,
    UnsupportedKind,
    Policy,
    CrashLoop,
    User,
}

/// Additional state facts required to derive admission from a catalog entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EnablementFacts {
    pub pending_removal: bool,
    pub missing_install_files: bool,
    pub policy_denied: bool,
    pub crash_loop: bool,
}

/// Applies the design's strict primary-reason order without mutating user intent.
pub fn derive_effective_enablement(
    entry: &CatalogEntry,
    user: UserEnablement,
    facts: EnablementFacts,
) -> EffectiveEnablement {
    let reason = if facts.pending_removal {
        Some(EffectiveDisableReason::PendingRemoval)
    } else if facts.missing_install_files {
        Some(EffectiveDisableReason::MissingInstallFiles)
    } else if entry.integrity != IntegrityStatus::Verified {
        Some(EffectiveDisableReason::IntegrityMismatch)
    } else if entry.validity != ManifestValidity::Valid {
        Some(EffectiveDisableReason::InvalidManifest)
    } else if entry.compatibility != RuntimeCompatibility::Compatible {
        Some(EffectiveDisableReason::IncompatibleEngine)
    } else if !matches!(entry.support, RuntimeSupport::Supported) {
        Some(EffectiveDisableReason::UnsupportedKind)
    } else if facts.policy_denied {
        Some(EffectiveDisableReason::Policy)
    } else if facts.crash_loop {
        Some(EffectiveDisableReason::CrashLoop)
    } else if user == UserEnablement::Disabled {
        Some(EffectiveDisableReason::User)
    } else {
        None
    };

    reason.map_or(EffectiveEnablement::Enabled, EffectiveEnablement::Disabled)
}

#[cfg(test)]
mod tests {
    use super::{
        EffectiveDisableReason, EffectiveEnablement, EnablementFacts, UserEnablement,
        derive_effective_enablement,
    };
    use crate::{
        CatalogEntry, IntegrityStatus, ManifestValidity, RuntimeCompatibility, RuntimeSupport,
    };
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;

    /// Proves higher-priority recovery facts cannot be hidden by user intent.
    #[test]
    fn derives_primary_disable_reason_in_strict_order() {
        let entry = CatalogEntry {
            plugin_id: None,
            location: PathBuf::from("plugin"),
            manifest: None,
            validity: ManifestValidity::Invalid,
            compatibility: RuntimeCompatibility::Compatible,
            support: RuntimeSupport::Supported,
            integrity: IntegrityStatus::DigestMismatch,
            diagnostics: Vec::new(),
        };
        assert_eq!(
            derive_effective_enablement(
                &entry,
                UserEnablement::Enabled,
                EnablementFacts {
                    pending_removal: true,
                    ..EnablementFacts::default()
                }
            ),
            EffectiveEnablement::Disabled(EffectiveDisableReason::PendingRemoval)
        );
    }
}
