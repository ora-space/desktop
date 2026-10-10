//! One live Desktop process per application data directory.
//!
//! Workflow recovery treats every `Running` row as work left behind by a process
//! that has already exited. That is true only when this process is the sole owner
//! of the data directory. An operating-system file lock is that proof: the kernel
//! releases it when the holder exits or crashes, and a live holder keeps it even
//! if the process is busy. A heartbeat or a stored process id would not — a slow
//! process can miss a heartbeat, and a process id can be reused by an unrelated
//! program.

use ora_utils::fs::{ExclusiveFileLock, ExclusiveLockError};
use std::path::{Path, PathBuf};

/// Sidecar lock beside the database. It is not the database file: locking the
/// database itself would also block readers on Windows.
pub(crate) const INSTANCE_LOCK_FILE_NAME: &str = "instance.lock";

/// Whether this process may open the database and recover interrupted runs.
#[derive(Debug)]
pub(crate) enum DesktopInstance {
    /// This process holds the lock until the value is dropped.
    Acquired(ExclusiveFileLock),
    /// Another live process already holds the lock.
    AlreadyRunning { path: PathBuf },
}

/// Creates the data directory when needed and takes the instance lock without waiting.
///
/// `AlreadyRunning` is not an I/O failure. The caller must leave without opening
/// the database. Dropping [`DesktopInstance::Acquired`] releases the lock.
pub(crate) fn acquire_desktop_instance(
    data_directory: &Path,
) -> Result<DesktopInstance, ExclusiveLockError> {
    if let Err(source) = std::fs::create_dir_all(data_directory) {
        return Err(ExclusiveLockError::Io {
            path: data_directory.to_path_buf(),
            source,
        });
    }
    let path = data_directory.join(INSTANCE_LOCK_FILE_NAME);
    match ExclusiveFileLock::try_acquire(&path) {
        Ok(lock) => Ok(DesktopInstance::Acquired(lock)),
        Err(ExclusiveLockError::Busy { path }) => Ok(DesktopInstance::AlreadyRunning { path }),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::{DesktopInstance, acquire_desktop_instance};
    use tempfile::TempDir;

    /// The second acquisition sees a live owner, and a later acquisition succeeds
    /// only after that owner is dropped — the same transition a crashed process
    /// produces when the operating system releases the lock.
    #[test]
    fn a_second_acquisition_is_not_the_instance_owner_until_the_first_is_dropped() {
        let temp = TempDir::new().expect("temp dir");
        let first = acquire_desktop_instance(temp.path()).expect("first owner");
        assert!(matches!(first, DesktopInstance::Acquired(_)));

        let second = acquire_desktop_instance(temp.path()).expect("second result");
        assert!(matches!(second, DesktopInstance::AlreadyRunning { .. }));

        drop(first);
        let third = acquire_desktop_instance(temp.path()).expect("owner after release");
        assert!(matches!(third, DesktopInstance::Acquired(_)));
    }
}
