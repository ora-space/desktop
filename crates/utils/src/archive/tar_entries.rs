use super::error::ArchiveError;
use super::tree_writer::TreeWriter;
use flate2::read::GzDecoder;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use tar::EntryType;

/// Extracts a gzip-compressed TAR archive, rejecting links and special devices.
pub(super) fn extract_tar_gz(mut file: File, writer: &mut TreeWriter) -> Result<(), ArchiveError> {
    let mut magic = [0u8; 2];
    file.read_exact(&mut magic)
        .map_err(|_| ArchiveError::Corrupt)?;
    if magic != [0x1f, 0x8b] {
        return Err(ArchiveError::FormatMismatch);
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|_| ArchiveError::Corrupt)?;

    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let mut entries = archive.entries().map_err(|_| ArchiveError::Corrupt)?;
    for entry in &mut entries {
        let mut entry = entry.map_err(|_| ArchiveError::Corrupt)?;
        let entry_type = entry.header().entry_type();
        match classify_tar_entry(entry_type) {
            // Metadata entries are skipped without reading; the iterator advances past them.
            TarEntryKind::Metadata => {}
            TarEntryKind::Special => {
                return Err(ArchiveError::SpecialEntryUnsupported);
            }
            TarEntryKind::File | TarEntryKind::Directory => {
                let path_bytes = entry.path_bytes();
                let name = std::str::from_utf8(&path_bytes)
                    .map_err(|_| ArchiveError::PathEncodingInvalid)?
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
