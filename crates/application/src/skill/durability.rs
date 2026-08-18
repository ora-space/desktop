//! Directory and file durability barriers for skill package promotion.
//!
//! Process crashes are recovered from journals. Power-loss atomicity additionally requires
//! package files, the journal file, and the renamed directory's parents to reach stable storage
//! before SQLite commits the catalog row. That ordering makes the remaining crash window
//! "directory exists, row not committed", which startup reconciliation already repairs.

use std::fs::{self, File};
use std::io;
use std::path::Path;

/// Flushes one regular file to stable storage, then flushes its parent directory.
///
/// Skill transaction journals use this so recovery can still see the marker after a power
/// loss. The parent flush is best-effort on platforms that cannot fsync directories.
pub(crate) fn persist_file(path: &Path) -> io::Result<()> {
    File::options().write(true).open(path)?.sync_all()?;
    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

/// Flushes one package file and every directory entry from its parent through `package_root`.
///
/// A file in `scripts/lib/helper.py` needs more than the file and `lib` directory flushed:
/// `scripts` must also persist its `lib` entry before the staging root can be promoted safely.
pub(crate) fn persist_package_file(path: &Path, package_root: &Path) -> io::Result<()> {
    File::options().write(true).open(path)?.sync_all()?;
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "package file has no parent"))?;
    sync_directory_chain(parent, package_root, sync_directory)
}

/// Renames a directory and flushes both parents so the new entry survives power loss.
///
/// A successful return means the destination name is visible in its parent on platforms that
/// support directory fsync. If the metadata flush fails, this attempts to undo the rename so
/// callers do not proceed to a database commit against an undurable promote.
pub(crate) fn rename_directory_persistently(from: &Path, to: &Path) -> io::Result<()> {
    rename_directory_persistently_with(from, to, sync_directory)
}

/// Renames a directory for startup recovery, tolerating hosts without directory fsync.
///
/// Mutation commit paths remain strict on Unix because they can abort before the database commit.
/// Recovery has no such option: an unsupported metadata flush must not make the backend
/// permanently unavailable while a valid backup can still be restored.
pub(crate) fn rename_directory_for_recovery(from: &Path, to: &Path) -> io::Result<()> {
    rename_directory_for_recovery_with(from, to, sync_directory)
}

/// Implements a recovery rename with an injectable directory barrier for deterministic tests.
fn rename_directory_for_recovery_with(
    from: &Path,
    to: &Path,
    sync: impl FnMut(&Path) -> io::Result<()>,
) -> io::Result<()> {
    match rename_directory_persistently_with(from, to, sync) {
        Ok(()) => Ok(()),
        Err(error) if directory_sync_is_unsupported(&error) => {
            if to.exists() && !from.exists() {
                Ok(())
            } else {
                fs::rename(from, to)
            }
        }
        Err(error) => Err(error),
    }
}

/// Implements a durable rename with an injectable directory barrier for deterministic tests.
fn rename_directory_persistently_with(
    from: &Path,
    to: &Path,
    mut sync: impl FnMut(&Path) -> io::Result<()>,
) -> io::Result<()> {
    if from.is_dir() {
        sync(from)?;
    }
    fs::rename(from, to)?;
    if let Err(error) = persist_rename_parents_with(from, to, &mut sync) {
        let _ = fs::rename(to, from);
        return Err(error);
    }
    Ok(())
}

/// Flushes the destination parent and, when it differs, the source parent of a rename.
fn persist_rename_parents_with(
    from: &Path,
    to: &Path,
    mut sync: impl FnMut(&Path) -> io::Result<()>,
) -> io::Result<()> {
    if let Some(parent) = to.parent() {
        sync(parent)?;
    }
    // The source parent records the removal; skip it when both names share one directory.
    if let Some(source) = from.parent()
        && to.parent() != Some(source)
    {
        sync(source)?;
    }
    Ok(())
}

/// Flushes a directory and each ancestor through the package root, deepest first.
fn sync_directory_chain(
    start: &Path,
    package_root: &Path,
    mut sync: impl FnMut(&Path) -> io::Result<()>,
) -> io::Result<()> {
    if !start.starts_with(package_root) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "package file is outside its package root",
        ));
    }
    let mut current = start;
    loop {
        sync(current)?;
        if current == package_root {
            return Ok(());
        }
        current = current.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "package root is not an ancestor of the package file",
            )
        })?;
    }
}

/// Flushes directory metadata so a preceding create, rename, or unlink can survive power loss.
///
/// Linux and other Unix platforms treat a failed directory fsync as a hard error: continuing
/// would re-open the "row committed, directory missing" window. Windows cannot reliably fsync
/// directories, so an unsupported flush is ignored and documented as a platform limit rather
/// than failing every promote.
pub(crate) fn sync_directory(path: &Path) -> io::Result<()> {
    match sync_directory_inner(path) {
        Ok(()) => Ok(()),
        #[cfg(windows)]
        Err(error) if directory_sync_is_unsupported(&error) => Ok(()),
        Err(error) => Err(error),
    }
}

/// Opens a directory and requests a full metadata flush.
fn sync_directory_inner(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::fs::OpenOptions;
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_BACKUP_SEMANTICS is required to open a directory handle on Windows.
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)?
            .sync_all()
    }
    #[cfg(not(windows))]
    {
        File::open(path)?.sync_all()
    }
}

/// Returns whether a directory flush failed because the host cannot fsync directories.
fn directory_sync_is_unsupported(error: &io::Error) -> bool {
    if matches!(
        error.kind(),
        io::ErrorKind::InvalidInput | io::ErrorKind::Unsupported
    ) {
        return true;
    }
    #[cfg(windows)]
    {
        matches!(error.kind(), io::ErrorKind::PermissionDenied)
            || matches!(error.raw_os_error(), Some(1 | 5 | 6 | 50 | 87))
    }
    #[cfg(not(windows))]
    {
        // EINVAL plus the Linux and macOS ENOTSUP values cover filesystems that accept opening a
        // directory but reject fsync on it (for example some overlay or network mounts).
        matches!(error.raw_os_error(), Some(22 | 45 | 95))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        persist_file, rename_directory_for_recovery_with, rename_directory_persistently,
        rename_directory_persistently_with, sync_directory, sync_directory_chain,
    };
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    #[test]
    fn sync_directory_accepts_a_temporary_directory() {
        let temp = TempDir::new().unwrap();
        sync_directory(temp.path()).unwrap();
    }

    #[test]
    fn persist_file_flushes_a_regular_file() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("marker.json");
        fs::write(&path, "{}").unwrap();
        persist_file(&path).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "{}");
    }

    #[test]
    fn rename_directory_persistently_moves_a_directory_into_its_parent() {
        let temp = TempDir::new().unwrap();
        let skills_root = temp.path().join("skills");
        let staging_root = skills_root.join(".ora-staging");
        fs::create_dir_all(&staging_root).unwrap();
        let staging = staging_root.join("txn");
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("SKILL.md"), "body").unwrap();

        let formal = skills_root.join("grilling");
        rename_directory_persistently(&staging, &formal).unwrap();

        assert!(formal.join("SKILL.md").is_file());
        assert!(!staging.exists());
    }

    #[test]
    fn sync_directory_chain_flushes_every_nested_parent_through_the_package_root() {
        let root = Path::new("staging");
        let start = root.join("scripts").join("lib");
        let mut visited = Vec::new();

        sync_directory_chain(&start, root, |path| {
            visited.push(path.to_path_buf());
            Ok(())
        })
        .unwrap();

        assert_eq!(
            visited,
            vec![
                root.join("scripts").join("lib"),
                root.join("scripts"),
                root.to_path_buf(),
            ]
        );
    }

    #[test]
    fn durable_rename_undoes_the_move_when_a_parent_barrier_fails() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("SKILL.md"), "body").unwrap();
        let mut barriers = 0;

        let error = rename_directory_persistently_with(&source, &destination, |_| {
            barriers += 1;
            if barriers == 2 {
                Err(std::io::Error::other("injected directory flush failure"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();

        assert_eq!(error.to_string(), "injected directory flush failure");
        assert!(source.join("SKILL.md").is_file());
        assert!(!destination.exists());
    }

    #[test]
    fn recovery_rename_continues_when_directory_fsync_is_unsupported() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("backup");
        let destination = temp.path().join("grilling");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("SKILL.md"), "body").unwrap();

        rename_directory_for_recovery_with(&source, &destination, |_| {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "injected unsupported directory fsync",
            ))
        })
        .unwrap();

        assert!(destination.join("SKILL.md").is_file());
        assert!(!source.exists());
    }
}
