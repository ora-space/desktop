use crate::error::PrepareError;
use crate::limits::Limits;
use crate::snapshot::{Snapshot, SnapshotWriter};
use flate2::read::GzDecoder;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use tar::EntryType;

/// Supported archive container formats. `.skill` files are ZIP archives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    Zip,
    TarGz,
}

/// The minimum expansion allowance granted to small archives before the ratio clamp applies.
const MIN_EXPANSION_BUDGET: u64 = 10 * 1024 * 1024;

impl ArchiveFormat {
    /// Derives the allowed format from a file name extension, case-insensitively.
    pub fn from_extension(file_name: &str) -> Option<ArchiveFormat> {
        let lower = file_name.to_ascii_lowercase();
        if lower.ends_with(".zip") || lower.ends_with(".skill") {
            Some(ArchiveFormat::Zip)
        } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
            Some(ArchiveFormat::TarGz)
        } else {
            None
        }
    }

    /// Returns the allowed file extensions surfaced in unsupported-format errors.
    pub fn supported_extensions() -> &'static [&'static str] {
        &["zip", "skill", "tar.gz", "tgz"]
    }
}

/// Extracts one validated archive into `destination`, enforcing every session limit.
///
/// The extension-driven format is validated against the archive content signature; a mismatch
/// or corrupt structure rejects the whole snapshot before any skill is staged.
pub fn extract_archive(
    format: ArchiveFormat,
    archive_path: &Path,
    destination: &Path,
    limits: &Limits,
) -> Result<Snapshot, PrepareError> {
    let metadata = std::fs::metadata(archive_path).map_err(|error| PrepareError::Io {
        message: format!("failed to stat archive {}: {error}", archive_path.display()),
    })?;
    if metadata.len() > limits.max_archive_bytes {
        return Err(PrepareError::ArchiveTooLarge);
    }
    let file = File::open(archive_path).map_err(|error| PrepareError::Io {
        message: format!("failed to open archive {}: {error}", archive_path.display()),
    })?;
    let expansion_budget = expansion_budget(metadata.len(), limits.max_snapshot_bytes);
    let mut writer = SnapshotWriter::new(
        destination.to_path_buf(),
        limits.clone(),
        expansion_budget,
        true,
    )?;

    match format {
        ArchiveFormat::Zip => extract_zip(file, &mut writer)?,
        ArchiveFormat::TarGz => extract_tar_gz(file, &mut writer)?,
    }
    Ok(writer.finish())
}

/// Extracts a ZIP archive, rejecting encrypted and special entries before writing.
fn extract_zip(mut file: File, writer: &mut SnapshotWriter) -> Result<(), PrepareError> {
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)
        .map_err(|_| PrepareError::ArchiveCorrupt)?;
    if magic != *b"PK\x03\x04" {
        return Err(PrepareError::ArchiveFormatMismatch);
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|_| PrepareError::ArchiveCorrupt)?;

    let mut archive = zip::ZipArchive::new(file).map_err(|_| PrepareError::ArchiveCorrupt)?;
    for index in 0..archive.len() {
        // An AES entry without a password surfaces as an unsupported-archive or invalid-
        // password error; ZipCrypto entries are detected below through the flag bit.
        let mut entry = match archive.by_index(index) {
            Ok(entry) => entry,
            Err(error) if requires_password(&error) => {
                return Err(PrepareError::ArchiveEncryptedUnsupported);
            }
            Err(_) => return Err(PrepareError::ArchiveCorrupt),
        };
        if entry.encrypted() {
            return Err(PrepareError::ArchiveEncryptedUnsupported);
        }
        if is_special_zip_entry(&entry) {
            return Err(PrepareError::ArchiveSpecialEntryUnsupported);
        }
        let name = std::str::from_utf8(entry.name_raw())
            .map_err(|_| PrepareError::ArchivePathEncodingInvalid)?
            .to_string();
        if entry.is_dir() {
            writer.add_directory(&name)?;
        } else {
            writer.add_file(&name, &mut entry)?;
        }
    }
    Ok(())
}

/// Extracts a gzip-compressed TAR archive, rejecting links and special devices.
fn extract_tar_gz(mut file: File, writer: &mut SnapshotWriter) -> Result<(), PrepareError> {
    let mut magic = [0u8; 2];
    file.read_exact(&mut magic)
        .map_err(|_| PrepareError::ArchiveCorrupt)?;
    if magic != [0x1f, 0x8b] {
        return Err(PrepareError::ArchiveFormatMismatch);
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|_| PrepareError::ArchiveCorrupt)?;

    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let mut entries = archive
        .entries()
        .map_err(|_| PrepareError::ArchiveCorrupt)?;
    for entry in &mut entries {
        let mut entry = entry.map_err(|_| PrepareError::ArchiveCorrupt)?;
        let entry_type = entry.header().entry_type();
        match classify_tar_entry(entry_type) {
            // Metadata entries are skipped without reading; the iterator advances past them.
            TarEntryKind::Metadata => {}
            TarEntryKind::Special => {
                return Err(PrepareError::ArchiveSpecialEntryUnsupported);
            }
            TarEntryKind::File | TarEntryKind::Directory => {
                let path_bytes = entry.path_bytes();
                let name = std::str::from_utf8(&path_bytes)
                    .map_err(|_| PrepareError::ArchivePathEncodingInvalid)?
                    .trim_end_matches('/')
                    .to_string();
                if entry_type.is_dir() {
                    writer.add_directory(&name)?;
                } else {
                    writer.add_file(&name, &mut entry)?;
                }
            }
        }
    }
    Ok(())
}

/// Classifies one TAR entry type for safe extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TarEntryKind {
    File,
    Directory,
    Metadata,
    Special,
}

/// Maps TAR entry types onto the extraction policy, rejecting anything unsafe.
fn classify_tar_entry(entry_type: EntryType) -> TarEntryKind {
    match entry_type {
        EntryType::Regular | EntryType::Continuous => TarEntryKind::File,
        EntryType::Directory => TarEntryKind::Directory,
        EntryType::XHeader
        | EntryType::XGlobalHeader
        | EntryType::GNULongName
        | EntryType::GNULongLink => TarEntryKind::Metadata,
        EntryType::Symlink
        | EntryType::Link
        | EntryType::Char
        | EntryType::Block
        | EntryType::Fifo
        | EntryType::GNUSparse => TarEntryKind::Special,
        EntryType::__Nonexhaustive(_) => TarEntryKind::Special,
    }
}

/// Rejects ZIP entries that are not plain files or directories (symlinks, devices, FIFOs).
fn is_special_zip_entry(entry: &zip::read::ZipFile<'_>) -> bool {
    if entry.is_symlink() {
        return true;
    }
    if let Some(mode) = entry.unix_mode() {
        let file_type = mode & 0o170_000;
        if matches!(
            file_type,
            0o120_000 | 0o020_000 | 0o060_000 | 0o010_000 | 0o140_000
        ) {
            return true;
        }
    }
    false
}

/// Classifies zip entry-open failures that mean the entry requires a password.
fn requires_password(error: &zip::result::ZipError) -> bool {
    match error {
        zip::result::ZipError::InvalidPassword => true,
        zip::result::ZipError::UnsupportedArchive(message) => {
            message.contains("Password required to decrypt")
        }
        _ => false,
    }
}

/// Computes the cumulative extraction budget: `min(max_snapshot, max(10 MiB, size * 100))`.
fn expansion_budget(archive_size: u64, max_snapshot_bytes: u64) -> u64 {
    let ratio_budget = archive_size.saturating_mul(100).max(MIN_EXPANSION_BUDGET);
    ratio_budget.min(max_snapshot_bytes)
}

#[cfg(test)]
mod tests {
    use super::{ArchiveFormat, expansion_budget};
    use pretty_assertions::assert_eq;

    #[test]
    fn derives_archive_formats_from_extensions_case_insensitively() {
        for name in ["skill.zip", "bundle.SKILL", "Skill.skill", "A.ZIP"] {
            assert_eq!(
                ArchiveFormat::from_extension(name),
                Some(ArchiveFormat::Zip)
            );
        }
        for name in ["skills.tar.gz", "bundle.TGZ", "Skills.tar.GZ"] {
            assert_eq!(
                ArchiveFormat::from_extension(name),
                Some(ArchiveFormat::TarGz)
            );
        }
        for name in ["skills.rar", "skills.gz", "skills", ".zip.bak"] {
            assert_eq!(ArchiveFormat::from_extension(name), None);
        }
    }

    #[test]
    fn computes_expansion_budget_with_ratio_and_floor() {
        // 50 KiB * 100 = 5 MiB, below the 10 MiB floor -> 10 MiB.
        assert_eq!(
            expansion_budget(50 * 1024, 200 * 1024 * 1024),
            10 * 1024 * 1024
        );
        // 1 MiB * 100 = 100 MiB, between the floor and the cap -> 100 MiB.
        assert_eq!(
            expansion_budget(1 * 1024 * 1024, 200 * 1024 * 1024),
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
