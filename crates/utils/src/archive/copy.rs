use super::error::ArchiveError;
use super::extracted::ExtractedTree;
use super::limits::ExtractLimits;
use super::tree_writer::{ByteBudgetKind, TreeWriter};
use crate::path::StrictRelativePath;
use std::fs;
use std::io;
use std::path::Path;

/// Materializes one local folder tree into a validated destination tree.
///
/// The folder is walked with `symlink_metadata` so links are never followed; symbolic links are
/// skipped and special files reject the whole source. Entries are processed in sorted name order
/// for deterministic conflict detection.
pub fn copy_tree(
    source_dir: &Path,
    destination: &Path,
    limits: &ExtractLimits,
) -> Result<ExtractedTree, ArchiveError> {
    let mut writer = TreeWriter::new(
        destination.to_path_buf(),
        limits.clone(),
        limits.max_total_bytes,
        ByteBudgetKind::FlatTotal,
    )?;
    copy_children(source_dir, StrictRelativePath::root(), &mut writer)?;
    Ok(writer.finish())
}

/// Recursively copies one directory's children, validating every path before writing.
fn copy_children(
    source: &Path,
    relative: StrictRelativePath,
    writer: &mut TreeWriter,
) -> Result<(), ArchiveError> {
    let read_dir = fs::read_dir(source).map_err(map_io)?;
    let mut entries = read_dir
        .collect::<Result<Vec<_>, io::Error>>()
        .map_err(map_io)?;
    entries.sort_by_key(fs::DirEntry::file_name);

    for entry in entries {
        let metadata = fs::symlink_metadata(entry.path()).map_err(map_io)?;
        let name = entry.file_name();
        let name = name.to_str().ok_or(ArchiveError::PathEncodingInvalid)?;
        let child = relative.append_segment(name);
        let file_type = metadata.file_type();

        if file_type.is_dir() {
            writer.add_directory(child.as_str())?;
            copy_children(&entry.path(), child, writer)?;
        } else if file_type.is_file() {
            let file = fs::File::open(entry.path()).map_err(map_io)?;
            writer.add_file(child.as_str(), file)?;
        } else if file_type.is_symlink() {
            // Links are intentionally omitted: following one could copy data outside the
            // submitted folder, while preserving it would require an unsafe tree format.
            continue;
        } else {
            return Err(ArchiveError::SpecialEntryUnsupported);
        }
    }
    Ok(())
}

/// Converts one source-walk I/O failure into a stable tree error.
fn map_io(error: io::Error) -> ArchiveError {
    ArchiveError::Io {
        message: format!("failed to read the source folder: {error}"),
    }
}
