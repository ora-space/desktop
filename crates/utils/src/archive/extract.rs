use super::error::ArchiveError;
use super::extracted::ExtractedTree;
use super::format::ArchiveFormat;
use super::limits::ExtractLimits;
use super::tar_entries::extract_tar_gz;
use super::tree_writer::{ByteBudgetKind, TreeWriter};
use super::zip_entries::extract_zip;
use std::fs::File;
use std::path::Path;

/// The minimum expansion allowance granted to small archives before the ratio clamp applies.
const MIN_EXPANSION_BUDGET: u64 = 10 * 1024 * 1024;

/// Extracts one validated archive into `destination`, enforcing every limit.
///
/// The extension-driven format is validated against the archive content signature; a mismatch
/// or corrupt structure rejects the whole tree before any file is written.
pub fn extract_archive(
    format: ArchiveFormat,
    archive_path: &Path,
    destination: &Path,
    limits: &ExtractLimits,
) -> Result<ExtractedTree, ArchiveError> {
    let metadata = std::fs::metadata(archive_path).map_err(|error| ArchiveError::Io {
        message: format!("failed to stat archive {}: {error}", archive_path.display()),
    })?;
    if metadata.len() > limits.max_archive_bytes {
        return Err(ArchiveError::TooLarge);
    }
    let file = File::open(archive_path).map_err(|error| ArchiveError::Io {
        message: format!("failed to open archive {}: {error}", archive_path.display()),
    })?;
    let expansion_budget = expansion_budget(metadata.len(), limits.max_total_bytes);
    let mut writer = TreeWriter::new(
        destination.to_path_buf(),
        limits.clone(),
        expansion_budget,
        ByteBudgetKind::ArchiveExpansion,
    )?;

    match format {
        ArchiveFormat::Zip => extract_zip(file, &mut writer)?,
        ArchiveFormat::TarGz => extract_tar_gz(file, &mut writer)?,
    }
    Ok(writer.finish())
}

/// Computes the cumulative extraction budget: `min(max_total, max(10 MiB, size * 100))`.
fn expansion_budget(archive_size: u64, max_total_bytes: u64) -> u64 {
    let ratio_budget = archive_size.saturating_mul(100).max(MIN_EXPANSION_BUDGET);
    ratio_budget.min(max_total_bytes)
}

#[cfg(test)]
mod tests {
    use super::expansion_budget;
    use pretty_assertions::assert_eq;

    #[test]
    fn computes_expansion_budget_with_ratio_and_floor() {
        // 50 KiB * 100 = 5 MiB, below the 10 MiB floor -> 10 MiB.
        assert_eq!(
            expansion_budget(50 * 1024, 200 * 1024 * 1024),
            10 * 1024 * 1024
        );
        // 1 MiB * 100 = 100 MiB, between the floor and the cap -> 100 MiB.
        assert_eq!(
            expansion_budget(1024 * 1024, 200 * 1024 * 1024),
            100 * 1024 * 1024
        );
        // 2 MiB * 100 = 200 MiB, exactly at the cap -> 200 MiB.
        assert_eq!(
            expansion_budget(2 * 1024 * 1024, 200 * 1024 * 1024),
            200 * 1024 * 1024
        );
        // 3 MiB * 100 = 300 MiB, clamped to the 200 MiB cap.
        assert_eq!(
            expansion_budget(3 * 1024 * 1024, 200 * 1024 * 1024),
            200 * 1024 * 1024
        );
        // A 200 MiB archive is capped by the raw archive size limit before extraction.
        assert_eq!(
            expansion_budget(100 * 1024 * 1024, 200 * 1024 * 1024),
            200 * 1024 * 1024
        );
    }
}
