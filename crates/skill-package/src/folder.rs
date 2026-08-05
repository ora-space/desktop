use crate::error::PrepareError;
use crate::limits::Limits;
use crate::path::RelativePath;
use crate::snapshot::{Snapshot, SnapshotWriter};
use std::fs;
use std::io;
use std::path::Path;

/// Materializes one local folder tree into a validated snapshot.
///
/// The folder is walked with `symlink_metadata` so links are never followed; symbolic links and
/// special files reject the whole source. Entries are processed in sorted name order for
/// deterministic conflict detection.
pub fn copy_folder_to(
    source_dir: &Path,
    destination: &Path,
    limits: &Limits,
) -> Result<Snapshot, PrepareError> {
    let mut writer = SnapshotWriter::new(
        destination.to_path_buf(),
        limits.clone(),
        limits.max_snapshot_bytes,
        false,
    )?;
    copy_tree(source_dir, RelativePath::root(), &mut writer)?;
    Ok(writer.finish())
}

/// Recursively copies one directory's children, validating every path before writing.
fn copy_tree(
    source: &Path,
    relative: RelativePath,
    writer: &mut SnapshotWriter,
) -> Result<(), PrepareError> {
    let read_dir = fs::read_dir(source).map_err(map_io)?;
    let mut entries = read_dir
        .collect::<Result<Vec<_>, io::Error>>()
        .map_err(map_io)?;
    entries.sort_by_key(fs::DirEntry::file_name);

    for entry in entries {
        let metadata = fs::symlink_metadata(entry.path()).map_err(map_io)?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or(PrepareError::ArchivePathEncodingInvalid)?;
        let child = relative.append_segment(name);
        let file_type = metadata.file_type();

        if file_type.is_dir() {
            writer.add_directory(child.as_str())?;
            copy_tree(&entry.path(), child, writer)?;
        } else if file_type.is_file() {
            let file = fs::File::open(entry.path()).map_err(map_io)?;
            writer.add_file(child.as_str(), file)?;
        } else {
            return Err(PrepareError::ArchiveSpecialEntryUnsupported);
        }
    }
    Ok(())
}

/// Converts one source-walk I/O failure into a stable snapshot error.
fn map_io(error: io::Error) -> PrepareError {
    PrepareError::Io {
        message: format!("failed to read the source folder: {error}"),
    }
}
