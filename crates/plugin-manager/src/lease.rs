use crate::{PluginError, PluginManagerConfig};
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Owns the process-lifetime exclusive state-writer lease and pinned management roots.
#[derive(Debug)]
pub struct ManagerLease {
    data_dir: File,
    plugin_system: File,
    lock: File,
    lock_path: PathBuf,
}

impl ManagerLease {
    /// Creates safe management roots and acquires an exclusive manager.lock handle.
    pub fn acquire(config: &PluginManagerConfig) -> Result<Self, PluginError> {
        std::fs::create_dir_all(config.data_dir()).map_err(internal_io)?;
        std::fs::create_dir_all(config.plugin_system_dir()).map_err(internal_io)?;
        std::fs::create_dir_all(config.plugins_dir()).map_err(internal_io)?;
        std::fs::create_dir_all(config.staging_dir()).map_err(internal_io)?;
        std::fs::create_dir_all(config.trash_dir()).map_err(internal_io)?;
        std::fs::create_dir_all(config.plugin_data_dir()).map_err(internal_io)?;
        std::fs::create_dir_all(config.plugin_runtime_dir()).map_err(internal_io)?;

        let data_dir = open_pinned_directory(config.data_dir())?;
        let plugin_system_path = config.plugin_system_dir();
        let plugin_system = open_pinned_directory(&plugin_system_path)?;
        let lock_path = plugin_system_path.join("manager.lock");
        let mut lock = open_exclusive_lock(&lock_path)?;
        verify_regular_single_link(&lock, &lock_path)?;
        write_lock_diagnostic(&mut lock)?;
        Ok(Self {
            data_dir,
            plugin_system,
            lock,
            lock_path,
        })
    }

    pub fn lock_path(&self) -> &Path {
        &self.lock_path
    }

    /// Confirms the lease's OS handles remain alive before a mutation starts.
    pub fn assert_held(&self) -> Result<(), PluginError> {
        let _ = self.data_dir.metadata().map_err(internal_io)?;
        let _ = self.plugin_system.metadata().map_err(internal_io)?;
        let _ = self.lock.metadata().map_err(internal_io)?;
        Ok(())
    }
}

/// Writes best-effort owner metadata without weakening the exclusive share mode.
fn write_lock_diagnostic(lock: &mut File) -> Result<(), PluginError> {
    lock.set_len(0).map_err(internal_io)?;
    lock.seek(SeekFrom::Start(0)).map_err(internal_io)?;
    let diagnostic = format!("pid={}\n", std::process::id());
    lock.write_all(diagnostic.as_bytes()).map_err(internal_io)?;
    lock.sync_all().map_err(internal_io)
}

#[cfg(windows)]
fn open_pinned_directory(path: &Path) -> Result<File, PluginError> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    let directory = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(internal_io)?;
    let metadata = directory.metadata().map_err(internal_io)?;
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(PluginError::Internal {
            message: "manager root is not a regular no-reparse directory".to_string(),
        });
    }
    Ok(directory)
}

#[cfg(not(windows))]
fn open_pinned_directory(path: &Path) -> Result<File, PluginError> {
    File::open(path).map_err(internal_io)
}

#[cfg(windows)]
fn open_exclusive_lock(path: &Path) -> Result<File, PluginError> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;

    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .share_mode(0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|error| {
            if matches!(error.raw_os_error(), Some(32 | 33)) {
                PluginError::DataDirInUse
            } else {
                internal_io(error)
            }
        })
}

#[cfg(not(windows))]
fn open_exclusive_lock(path: &Path) -> Result<File, PluginError> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path)
        .map_err(internal_io)
}

#[cfg(windows)]
fn verify_regular_single_link(file: &File, path: &Path) -> Result<(), PluginError> {
    use std::mem::zeroed;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, GetFileInformationByHandle,
    };

    let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { zeroed() };
    let succeeded =
        unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut information) };
    if succeeded == 0
        || information.nNumberOfLinks != 1
        || information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || information.dwFileAttributes
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY
            != 0
    {
        return Err(PluginError::Internal {
            message: format!(
                "manager lock is not a safe regular child: {}",
                path.display()
            ),
        });
    }
    Ok(())
}

#[cfg(not(windows))]
fn verify_regular_single_link(file: &File, path: &Path) -> Result<(), PluginError> {
    if !file.metadata().map_err(internal_io)?.is_file() {
        return Err(PluginError::Internal {
            message: format!("manager lock is not a regular file: {}", path.display()),
        });
    }
    Ok(())
}

/// Keeps OS-specific details inside a bounded management bootstrap error.
fn internal_io(error: std::io::Error) -> PluginError {
    PluginError::Internal {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::ManagerLease;
    use crate::PluginManagerConfig;
    use tempfile::TempDir;

    /// On Windows a second backend cannot acquire the same writer lease until Drop.
    #[cfg(windows)]
    #[test]
    fn excludes_second_manager_process_handle() {
        let root =
            TempDir::new().unwrap_or_else(|error| panic!("expected data directory: {error}"));
        let config = PluginManagerConfig::new(root.path());
        let first = ManagerLease::acquire(&config)
            .unwrap_or_else(|error| panic!("expected first lease: {error}"));
        assert!(ManagerLease::acquire(&config).is_err());
        drop(first);
        assert!(ManagerLease::acquire(&config).is_ok());
    }
}
