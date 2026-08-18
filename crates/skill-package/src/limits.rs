use ora_utils::archive::ExtractLimits;

/// Carries every resource limit applied while preparing one skill source.
///
/// Limits stay transport-agnostic so Web, Desktop, and tests enforce identical budgets. Tree
/// materialization limits are delegated to `ora-utils::archive`; the remaining fields are
/// skill-level budgets applied after the snapshot exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    /// Archive, entry, byte, and path limits applied while materializing the snapshot.
    pub extract: ExtractLimits,
    /// Maximum discoverable `SKILL.md` candidates in one source.
    pub max_skills: usize,
    /// Maximum ordinary files allowed inside one skill boundary.
    pub max_files_per_skill: usize,
    /// Maximum bytes read from one `SKILL.md` manifest.
    pub max_manifest_bytes: u64,
}

impl Default for Limits {
    /// Selects the default production limits shared by every runtime adapter.
    fn default() -> Self {
        Self {
            extract: ExtractLimits::default(),
            max_skills: 500,
            max_files_per_skill: 1000,
            max_manifest_bytes: 1024 * 1024,
        }
    }
}
