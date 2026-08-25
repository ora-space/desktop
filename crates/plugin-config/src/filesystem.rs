use std::io::Read;
use std::path::Path;

/// Supplies bounded reads and durable writes without coupling policy to the operating system.
///
/// Implementations must distinguish a missing file from other I/O failures and must never return
/// more than `limit + 1` bytes, which lets callers reject concurrently growing files safely.
pub trait ConfigurationFileSystem: Clone {
    /// Reads a regular file up to one byte beyond `limit`, returning `None` when it is absent.
    fn read_bounded(&self, path: &Path, limit: usize) -> std::io::Result<Option<Vec<u8>>>;
    /// Creates the directory hierarchy needed for one new value file.
    fn create_dir_all(&self, path: &Path) -> std::io::Result<()>;
    /// Atomically replaces one file with fully flushed contents.
    fn atomic_write(&self, path: &Path, contents: &[u8]) -> std::io::Result<()>;
    /// Moves one file only when the destination is absent so backups cannot be overwritten.
    fn move_no_replace(&self, source: &Path, destination: &Path) -> std::io::Result<()>;
}

/// Filesystem adapter used by the production configuration service.
#[derive(Debug, Clone, Copy, Default)]
pub struct StandardConfigurationFileSystem;

impl ConfigurationFileSystem for StandardConfigurationFileSystem {
    fn read_bounded(&self, path: &Path, limit: usize) -> std::io::Result<Option<Vec<u8>>> {
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let mut contents = Vec::new();
        // Read one extra byte so callers can distinguish an at-limit file from one that grew
        // beyond the accepted bound without risking integer wraparound on this public port.
        file.take((limit as u64).saturating_add(1))
            .read_to_end(&mut contents)?;
        Ok(Some(contents))
    }

    fn create_dir_all(&self, path: &Path) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            // Apply restrictive permissions when each directory is created. Tightening the mode
            // afterwards would leave a crash-visible interval with the process umask's defaults.
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(path)?;
        }
        #[cfg(not(unix))]
        std::fs::create_dir_all(path)?;
        #[cfg(windows)]
        crate::windows_permissions::restrict_to_current_user(
            path,
            crate::windows_permissions::AccessControlTarget::Directory,
        )?;
        Ok(())
    }

    fn atomic_write(&self, path: &Path, contents: &[u8]) -> std::io::Result<()> {
        ora_utils::atomic::write_with_prepare(path, contents, |temporary| {
            #[cfg(windows)]
            crate::windows_permissions::restrict_to_current_user(
                temporary,
                crate::windows_permissions::AccessControlTarget::File,
            )?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(temporary, std::fs::Permissions::from_mode(0o600))?;
            }
            Ok(())
        })
    }

    fn move_no_replace(&self, source: &Path, destination: &Path) -> std::io::Result<()> {
        // A normal rename may replace a same-named backup. Linking first provides an exclusive
        // destination; if unlinking the source then fails, removing that link restores the
        // original single-name state without discarding either file's contents.
        std::fs::hard_link(source, destination)?;
        if let Err(error) = std::fs::remove_file(source) {
            let _ = std::fs::remove_file(destination);
            return Err(error);
        }
        Ok(())
    }
}
