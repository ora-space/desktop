use crate::path::RelativePathLimits;

/// Carries every resource limit applied while materializing one archive or folder tree.
///
/// The archive expansion budget is derived by the extractor as
/// `min(max_total_bytes, max(10 MiB, archive_size * 100))`, so small archives keep a normal
/// 10 MiB allowance before the 100:1 ratio clamp applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractLimits {
    /// Maximum raw archive file size accepted before extraction.
    pub max_archive_bytes: u64,
    /// Maximum cumulative ordinary-file bytes materialized from one tree.
    pub max_total_bytes: u64,
    /// Maximum archive or folder entries (files and directories both count).
    pub max_entries: usize,
    /// Limits applied to every entry path.
    pub path: RelativePathLimits,
}

impl Default for ExtractLimits {
    fn default() -> Self {
        Self {
            max_archive_bytes: 50 * 1024 * 1024,
            max_total_bytes: 200 * 1024 * 1024,
            max_entries: 5000,
            path: RelativePathLimits::default(),
        }
    }
}
