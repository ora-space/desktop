/// Carries every resource limit applied while materializing one skill source snapshot.
///
/// Limits stay transport-agnostic so Web, Desktop, and tests enforce identical budgets.
/// The session-level expansion budget for archives is derived by the extractor as
/// `min(max_snapshot_bytes, max(10 MiB, archive_size * 100))`, so small archives keep a
/// normal 10 MiB allowance before the 100:1 ratio clamp applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    /// Maximum raw archive file size accepted before extraction.
    pub max_archive_bytes: u64,
    /// Maximum cumulative ordinary-file bytes materialized from one source.
    pub max_snapshot_bytes: u64,
    /// Maximum archive or folder entries (files and directories both count).
    pub max_entries: usize,
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
            max_archive_bytes: 50 * 1024 * 1024,
            max_snapshot_bytes: 200 * 1024 * 1024,
            max_entries: 5000,
            max_skills: 500,
            max_files_per_skill: 1000,
            max_manifest_bytes: 1024 * 1024,
        }
    }
}
