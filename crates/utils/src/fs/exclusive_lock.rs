use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

use crate::fs::refuse_final_link;

/// Why an exclusive lock could not be taken.
#[derive(Debug)]
pub enum ExclusiveLockError {
    /// Another handle — in this process or in another one — currently holds the lock.
    Busy { path: PathBuf },
    /// The lock file could not be opened or the platform lock call failed.
    Io { path: PathBuf, source: io::Error },
}

impl std::fmt::Display for ExclusiveLockError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy { path } => write!(
                formatter,
                "another writer holds the lock at {}",
                path.display()
            ),
            Self::Io { path, source } => write!(formatter, "{source} at {}", path.display()),
        }
    }
}

impl std::error::Error for ExclusiveLockError {}

/// An exclusive advisory lock on a dedicated lock file, released when dropped.
///
/// The lock is taken on a *sidecar* file rather than on the data file it guards. That matters on
/// Windows, where `LockFileEx` also blocks other processes from reading the locked byte range:
/// locking the data file itself would make a concurrent read-only export fail while the writer
/// runs. Locking a sidecar leaves the data file readable and still serializes writers, both
/// across process generations inside one host and across separate host processes sharing the
/// same directory.
///
/// Advisory semantics apply: only participants that take the same lock are excluded. The lock
/// is held by the open handle, so it is released when the holder is dropped or its process
/// exits, whichever comes first — a crashed holder never leaves a stale lock behind.
#[derive(Debug)]
pub struct ExclusiveFileLock {
    file: File,
    path: PathBuf,
}

impl ExclusiveFileLock {
    /// Takes the lock at `path` without waiting, creating the lock file when absent.
    ///
    /// The final component is opened without following links so a link planted under the lock
    /// name cannot redirect the lock (and the create) to a foreign file.
    pub fn try_acquire(path: &Path) -> Result<Self, ExclusiveLockError> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        let file = refuse_final_link(&mut options)
            .open(path)
            .map_err(|source| ExclusiveLockError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        match file.try_lock() {
            Ok(()) => Ok(Self {
                file,
                path: path.to_path_buf(),
            }),
            Err(TryLockError::WouldBlock) => Err(ExclusiveLockError::Busy {
                path: path.to_path_buf(),
            }),
            Err(TryLockError::Error(source)) => Err(ExclusiveLockError::Io {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    /// Returns the lock file's path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ExclusiveFileLock {
    fn drop(&mut self) {
        // Closing the handle releases the lock on every platform; the explicit unlock only
        // makes the release happen before any other field is torn down. Its failure has no
        // remaining consequence worth propagating from a destructor.
        let _ = self.file.unlock();
    }
}

#[cfg(test)]
mod tests {
    use super::{ExclusiveFileLock, ExclusiveLockError};
    use tempfile::TempDir;

    /// A second acquisition through an independent handle is refused as busy while the first is
    /// held and succeeds once the first is dropped; the lock file itself is left in place.
    #[test]
    fn a_held_lock_is_busy_for_every_other_handle_until_dropped() {
        let temp = TempDir::new().expect("temp dir");
        let path = temp.path().join("plugin.log.lock");
        let first = ExclusiveFileLock::try_acquire(&path).expect("first acquisition");

        let second = ExclusiveFileLock::try_acquire(&path).err().expect("busy");
        assert!(
            matches!(second, ExclusiveLockError::Busy { .. }),
            "{second}"
        );

        drop(first);
        let third = ExclusiveFileLock::try_acquire(&path).expect("acquire after release");
        assert!(path.is_file());
        assert_eq!(third.path(), path.as_path());
    }

    /// The lock file is never created through a link planted under its name.
    #[cfg(unix)]
    #[test]
    fn a_link_under_the_lock_name_is_refused() {
        let temp = TempDir::new().expect("temp dir");
        let target = temp.path().join("elsewhere");
        let path = temp.path().join("plugin.log.lock");
        std::os::unix::fs::symlink(&target, &path).expect("dangling symlink");

        let error = ExclusiveFileLock::try_acquire(&path)
            .err()
            .expect("refused");

        assert!(matches!(error, ExclusiveLockError::Io { .. }), "{error}");
        assert!(!target.exists());
    }
}
