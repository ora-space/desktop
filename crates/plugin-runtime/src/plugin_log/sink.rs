//! Owns the active `plugin.log` file of one process generation.
//!
//! Plugin logs live in a host-managed root that no plugin API can address, but the host still
//! treats the path as hostile until proven otherwise: every directory level between the root
//! and the active file must be a plain directory that canonicalizes inside the root, the file
//! must be a plain file, and any doubt is a conflict that leaves the path exactly as found.
//!
//! Two more rules guard the file once the path is trusted. Only one writer may hold the active
//! file at a time — across process generations of one host and across hosts sharing one Ora
//! home — which a sidecar lock enforces. And a file whose last byte is not a newline was left
//! mid-record by a crash or partial write; the sink sets it aside under a recovery name instead
//! of appending onto the broken tail.

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Component, Path, PathBuf};

use ora_utils::fs::{
    ExclusiveFileLock, ExclusiveLockError, LineTail, classify_line_tail, next_available_file_name,
    refuse_final_link,
};
use time::macros::format_description;

/// Name of the active log file inside a plugin's log directory.
pub const ACTIVE_LOG_FILE_NAME: &str = "plugin.log";

/// Name of the sidecar whose exclusive lock is the right to write the active file.
pub const WRITER_LOCK_FILE_NAME: &str = "plugin.log.lock";

/// Why a sink could not be opened.
///
/// `Conflict` means the path was left untouched on purpose; `Busy` means another writer — an
/// earlier generation that has not released the file yet, or another host — holds the lock.
#[derive(Debug)]
pub enum SinkOpenError {
    Conflict { path: PathBuf, reason: &'static str },
    Busy { path: PathBuf },
    Io { path: PathBuf, source: io::Error },
}

impl SinkOpenError {
    /// Names the failure class for the host's bounded warning.
    pub fn class(&self) -> &'static str {
        match self {
            Self::Conflict { .. } => "path_conflict",
            Self::Busy { .. } => "writer_busy",
            Self::Io { .. } => "io",
        }
    }
}

impl std::fmt::Display for SinkOpenError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflict { path, reason } => {
                write!(formatter, "{reason} at {}", path.display())
            }
            Self::Busy { path } => write!(
                formatter,
                "another plugin log writer holds {}",
                path.display()
            ),
            Self::Io { path, source } => write!(formatter, "{source} at {}", path.display()),
        }
    }
}

impl std::error::Error for SinkOpenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Conflict { .. } | Self::Busy { .. } => None,
            Self::Io { source, .. } => Some(source),
        }
    }
}

impl From<ExclusiveLockError> for SinkOpenError {
    fn from(error: ExclusiveLockError) -> Self {
        match error {
            ExclusiveLockError::Busy { path } => Self::Busy { path },
            ExclusiveLockError::Io { path, source } => Self::Io { path, source },
        }
    }
}

/// Destination of rendered JSON lines; the file sink in production, a fault injector in tests.
///
/// Implementations report I/O failures and nothing else: the pipeline decides how a failure is
/// counted and never retries through the same sink.
pub trait LineSink {
    /// Appends one already-rendered JSON line, newline included.
    fn write_line(&mut self, line: &str) -> io::Result<()>;

    /// Pushes buffered lines to the operating system.
    fn flush(&mut self) -> io::Result<()>;
}

/// Append-only writer over the active log file, holding the writer lock for its lifetime.
#[derive(Debug)]
pub struct PluginLogSink {
    writer: BufWriter<File>,
    recovered_tail: Option<PathBuf>,
    // Declared last so the lock is released only after the file handle above is closed.
    _lock: ExclusiveFileLock,
}

impl PluginLogSink {
    /// Opens (creating when absent) `<directory>/plugin.log`, where `directory` lies below the
    /// host-managed `root`, without following any link at any level.
    ///
    /// The root is created if missing and canonicalized once; each level of the relative path is
    /// then created only when nothing exists under that name, reused when it is a plain
    /// directory, and refused otherwise. A final canonicalization proves the directory really
    /// is where the lexical join says it is. The writer lock is taken before the active file is
    /// touched, and an unterminated tail is set aside before the first append.
    pub fn open(root: &Path, directory: &Path) -> Result<Self, SinkOpenError> {
        ensure_directory_below_root(root, directory)?;
        let lock = ExclusiveFileLock::try_acquire(&directory.join(WRITER_LOCK_FILE_NAME))?;
        let path = directory.join(ACTIVE_LOG_FILE_NAME);
        let (mut file, created) = open_active_file(&path)?;
        let tail = if created {
            LineTail::Empty
        } else {
            classify_line_tail(&mut file).map_err(|source| SinkOpenError::Io {
                path: path.clone(),
                source,
            })?
        };
        if tail != LineTail::Unterminated {
            return Ok(Self {
                writer: BufWriter::new(file),
                recovered_tail: None,
                _lock: lock,
            });
        }
        // The handle must be closed before the rename can succeed on Windows.
        drop(file);
        let recovery = set_aside_unterminated_file(directory, &path)?;
        let (file, _) = open_active_file(&path)?;
        Ok(Self {
            writer: BufWriter::new(file),
            recovered_tail: Some(recovery),
            _lock: lock,
        })
    }

    /// Returns where an unterminated previous active file was preserved, if that happened.
    pub fn recovered_tail(&self) -> Option<&Path> {
        self.recovered_tail.as_deref()
    }
}

impl LineSink for PluginLogSink {
    fn write_line(&mut self, line: &str) -> io::Result<()> {
        self.writer.write_all(line.as_bytes())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

/// Proves that no writer currently holds the plugin's active file, or reports who might.
///
/// A missing directory trivially has no writer. Otherwise the writer lock is taken and released
/// immediately; the caller is expected to hold its own higher-level exclusion (the per-plugin
/// operation lock) so no new generation starts between this probe and whatever it guards.
pub fn confirm_writer_released(directory: &Path) -> Result<(), SinkOpenError> {
    if !directory.exists() {
        return Ok(());
    }
    ExclusiveFileLock::try_acquire(&directory.join(WRITER_LOCK_FILE_NAME))?;
    Ok(())
}

/// Validates and creates every level between `root` and `directory` as described on `open`.
fn ensure_directory_below_root(root: &Path, directory: &Path) -> Result<(), SinkOpenError> {
    let relative = directory
        .strip_prefix(root)
        .map_err(|_| SinkOpenError::Conflict {
            path: directory.to_path_buf(),
            reason: "log directory is not below the plugin logs root",
        })?;
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(SinkOpenError::Conflict {
            path: directory.to_path_buf(),
            reason: "log directory path contains a non-plain component",
        });
    }
    // Levels above the root belong to the Ora home and may legitimately be links; the root
    // itself must be a plain directory or every containment check below would be measured
    // against the link's target instead.
    if let Some(parent) = root.parent() {
        std::fs::create_dir_all(parent).map_err(|source| SinkOpenError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    ensure_plain_directory(root)?;
    let canonical_root = std::fs::canonicalize(root).map_err(|source| SinkOpenError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        ensure_plain_directory(&current)?;
    }
    let canonical = std::fs::canonicalize(&current).map_err(|source| SinkOpenError::Io {
        path: current.clone(),
        source,
    })?;
    if canonical != canonical_root.join(relative) {
        return Err(SinkOpenError::Conflict {
            path: current,
            reason: "log directory resolves outside the plugin logs root",
        });
    }
    Ok(())
}

/// Opens the active file for appending and reading, refusing anything that is not a plain
/// file; the flag says whether the file was created by this call (and is therefore empty).
fn open_active_file(path: &Path) -> Result<(File, bool), SinkOpenError> {
    let created = match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => false,
        Ok(_) => {
            return Err(SinkOpenError::Conflict {
                path: path.to_path_buf(),
                reason: "active log path exists but is not a regular file",
            });
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => true,
        Err(source) => {
            return Err(SinkOpenError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let mut options = OpenOptions::new();
    options.append(true).read(true).create(true);
    let file = refuse_final_link(&mut options)
        .open(path)
        .map_err(|source| SinkOpenError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    // The pre-open check and the open are not atomic; re-checking through the handle closes
    // the window in which a link could have been swapped in between them.
    let metadata = file.metadata().map_err(|source| SinkOpenError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.file_type().is_file() {
        return Err(SinkOpenError::Conflict {
            path: path.to_path_buf(),
            reason: "active log path changed to a non-regular file while opening",
        });
    }
    Ok((file, created))
}

/// Moves an unterminated active file to a unique recovery name in the same directory.
///
/// The name is chosen while this generation holds the writer lock, so no other writer can claim
/// the same name in between the existence check and the rename; the file is never truncated
/// and no existing file is overwritten.
fn set_aside_unterminated_file(directory: &Path, path: &Path) -> Result<PathBuf, SinkOpenError> {
    let stamp = ora_logging::clock::now_local()
        .format(format_description!(
            "[year][month][day]T[hour][minute][second]"
        ))
        .unwrap_or_default();
    let recovery =
        next_available_file_name(directory, &format!("plugin.recovered-{stamp}.log"), |_| {
            false
        });
    std::fs::rename(path, &recovery).map_err(|source| SinkOpenError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(recovery)
}

/// Creates `path` as a directory when absent, accepts an existing plain directory, and refuses
/// a file, link, or reparse point under that name.
fn ensure_plain_directory(path: &Path) -> Result<(), SinkOpenError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => Ok(()),
        Ok(_) => Err(SinkOpenError::Conflict {
            path: path.to_path_buf(),
            reason: "log directory level exists but is not a plain directory",
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            std::fs::create_dir(path).map_err(|source| SinkOpenError::Io {
                path: path.to_path_buf(),
                source,
            })
        }
        Err(source) => Err(SinkOpenError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{LineSink, PluginLogSink, SinkOpenError, confirm_writer_released};
    use pretty_assertions::assert_eq;
    use std::path::Path;
    use tempfile::TempDir;

    /// The root and every level below it are created, records append across reopen, and an
    /// existing plain tree is reused rather than replaced.
    #[test]
    fn creates_every_level_reuses_and_appends() {
        ora_logging::initialize_test_clock();
        let temp = TempDir::new().expect("temp dir");
        let root = temp.path().join("plugins").join("logs");
        let directory = root.join("official").join("example");
        let mut sink = PluginLogSink::open(&root, &directory).expect("first open");
        sink.write_line("{\"a\":1}\n").expect("write");
        sink.flush().expect("flush");
        drop(sink);
        let mut sink = PluginLogSink::open(&root, &directory).expect("second open");
        sink.write_line("{\"b\":2}\n").expect("write");
        sink.flush().expect("flush");
        assert_eq!(
            (
                std::fs::read_to_string(directory.join("plugin.log")).expect("read log"),
                sink.recovered_tail(),
            ),
            ("{\"a\":1}\n{\"b\":2}\n".to_string(), None)
        );
    }

    /// A file under a directory level, a non-regular active file, and a directory outside the
    /// root are conflicts that leave the filesystem untouched.
    #[test]
    fn foreign_paths_are_a_conflict_and_stay_untouched() {
        ora_logging::initialize_test_clock();
        let temp = TempDir::new().expect("temp dir");
        let root = temp.path().join("logs");
        std::fs::create_dir_all(&root).expect("root");
        std::fs::write(root.join("official"), "not a directory").expect("foreign file");
        let error = PluginLogSink::open(&root, &root.join("official").join("example"))
            .err()
            .expect("conflict");
        assert!(matches!(error, SinkOpenError::Conflict { .. }), "{error}");
        assert_eq!(
            std::fs::read_to_string(root.join("official")).expect("foreign file"),
            "not a directory"
        );

        let other = root.join("other").join("example");
        std::fs::create_dir_all(other.join("plugin.log")).expect("directory named plugin.log");
        let error = PluginLogSink::open(&root, &other).err().expect("conflict");
        assert!(matches!(error, SinkOpenError::Conflict { .. }), "{error}");
        assert!(other.join("plugin.log").is_dir());

        let outside = temp.path().join("elsewhere");
        let error = PluginLogSink::open(&root, &outside)
            .err()
            .expect("conflict");
        assert!(matches!(error, SinkOpenError::Conflict { .. }), "{error}");
        assert!(!outside.exists());

        let escaping = root.join("..").join("escaping");
        let error = PluginLogSink::open(&root, &escaping)
            .err()
            .expect("conflict");
        assert!(matches!(error, SinkOpenError::Conflict { .. }), "{error}");
        assert!(!Path::new(&escaping).exists());
    }

    /// A symlinked directory level or active file is refused so writes never leave the root.
    #[cfg(unix)]
    #[test]
    fn symlinks_at_any_level_are_refused() {
        ora_logging::initialize_test_clock();
        let temp = TempDir::new().expect("temp dir");
        let root = temp.path().join("logs");
        std::fs::create_dir_all(root.join("official")).expect("root");
        let elsewhere = temp.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).expect("elsewhere");
        std::os::unix::fs::symlink(&elsewhere, root.join("official").join("linked"))
            .expect("dir symlink");
        let error = PluginLogSink::open(&root, &root.join("official").join("linked"))
            .err()
            .expect("conflict");
        assert!(matches!(error, SinkOpenError::Conflict { .. }), "{error}");

        let plain = root.join("official").join("plain");
        std::fs::create_dir_all(&plain).expect("plain");
        let target = temp.path().join("elsewhere.log");
        std::fs::write(&target, "").expect("target");
        std::os::unix::fs::symlink(&target, plain.join("plugin.log")).expect("file symlink");
        let error = PluginLogSink::open(&root, &plain).err().expect("conflict");
        assert!(
            matches!(
                error,
                SinkOpenError::Conflict { .. } | SinkOpenError::Io { .. }
            ),
            "{error}"
        );
        assert_eq!(
            (
                std::fs::read_dir(&elsewhere).expect("elsewhere").count(),
                std::fs::read_to_string(&target).expect("target"),
            ),
            (0, String::new())
        );
    }

    /// A symlinked logs root is refused as well: the root itself must be a plain directory.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_root_is_refused() {
        ora_logging::initialize_test_clock();
        let temp = TempDir::new().expect("temp dir");
        let elsewhere = temp.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).expect("elsewhere");
        let root = temp.path().join("logs");
        std::os::unix::fs::symlink(&elsewhere, &root).expect("root symlink");

        let error = PluginLogSink::open(&root, &root.join("official").join("example"))
            .err()
            .expect("conflict");

        assert!(matches!(error, SinkOpenError::Conflict { .. }), "{error}");
        assert_eq!(std::fs::read_dir(&elsewhere).expect("elsewhere").count(), 0);
    }

    /// A directory-level reparse point (junction) is refused on Windows the same way a symlink
    /// is on Unix.
    #[cfg(windows)]
    #[test]
    fn a_junction_level_is_refused() {
        ora_logging::initialize_test_clock();
        let temp = TempDir::new().expect("temp dir");
        let root = temp.path().join("logs");
        std::fs::create_dir_all(root.join("official")).expect("root");
        let elsewhere = temp.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).expect("elsewhere");
        let junction = root.join("official").join("linked");
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&elsewhere)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("run mklink");
        assert!(status.success(), "mklink /J failed");

        let error = PluginLogSink::open(&root, &junction)
            .err()
            .expect("conflict");

        assert!(matches!(error, SinkOpenError::Conflict { .. }), "{error}");
        assert_eq!(std::fs::read_dir(&elsewhere).expect("elsewhere").count(), 0);
    }

    /// While one sink holds the active file, a second open of the same directory — the next
    /// generation, or another host sharing the home — is busy rather than a second writer, and
    /// the release probe reports the same; both succeed once the first sink is dropped.
    #[test]
    fn the_active_file_has_at_most_one_writer() {
        ora_logging::initialize_test_clock();
        let temp = TempDir::new().expect("temp dir");
        let root = temp.path().join("logs");
        let directory = root.join("official").join("example");
        let first = PluginLogSink::open(&root, &directory).expect("first writer");

        let second = PluginLogSink::open(&root, &directory).err().expect("busy");
        let probe = confirm_writer_released(&directory).err().expect("busy");
        assert!(matches!(second, SinkOpenError::Busy { .. }), "{second}");
        assert!(matches!(probe, SinkOpenError::Busy { .. }), "{probe}");

        drop(first);
        confirm_writer_released(&directory).expect("released");
        PluginLogSink::open(&root, &directory).expect("next writer");
        confirm_writer_released(&temp.path().join("never-created")).expect("missing is free");
    }

    /// An active file that ends mid-record is preserved byte for byte under a recovery name
    /// that never overwrites an existing file, and the new active file starts clean.
    #[test]
    fn an_unterminated_tail_is_set_aside_not_joined() {
        ora_logging::initialize_test_clock();
        let temp = TempDir::new().expect("temp dir");
        let root = temp.path().join("logs");
        let directory = root.join("official").join("example");
        std::fs::create_dir_all(&directory).expect("directory");
        std::fs::write(directory.join("plugin.log"), "{\"a\":1}\n{\"b\":").expect("broken log");

        let mut sink = PluginLogSink::open(&root, &directory).expect("open");
        let first_recovery = sink
            .recovered_tail()
            .expect("tail was recovered")
            .to_path_buf();
        sink.write_line("{\"c\":3}\n").expect("write");
        sink.flush().expect("flush");
        drop(sink);
        // Break the new file too; the second recovery must pick a different name.
        std::fs::write(directory.join("plugin.log"), "{\"c\":3}\n{\"d\":").expect("break again");
        let sink = PluginLogSink::open(&root, &directory).expect("reopen");
        let second_recovery = sink
            .recovered_tail()
            .expect("tail was recovered")
            .to_path_buf();

        assert_eq!(
            (
                std::fs::read_to_string(&first_recovery).expect("first recovery"),
                std::fs::read_to_string(&second_recovery).expect("second recovery"),
                std::fs::read_to_string(directory.join("plugin.log")).expect("active"),
                first_recovery == second_recovery,
                first_recovery.parent(),
            ),
            (
                "{\"a\":1}\n{\"b\":".to_string(),
                "{\"c\":3}\n{\"d\":".to_string(),
                String::new(),
                false,
                Some(directory.as_path()),
            )
        );
    }

    /// When the broken file cannot be set aside, the sink is unavailable and the file is left
    /// exactly as found rather than appended to or truncated.
    #[test]
    fn a_failed_recovery_leaves_the_file_and_fails_the_sink() {
        ora_logging::initialize_test_clock();
        // A read-only directory does not stop root from renaming; the arrangement below cannot
        // prove anything under that account.
        #[cfg(unix)]
        // SAFETY: `geteuid` reads the process credential and has no preconditions.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let temp = TempDir::new().expect("temp dir");
        let root = temp.path().join("logs");
        let directory = root.join("official").join("example");
        std::fs::create_dir_all(&directory).expect("directory");
        let active = directory.join("plugin.log");
        std::fs::write(&active, "{\"a\":1}\n{\"b\":").expect("broken log");
        let _blocker = block_rename(&directory, &active);

        let error = PluginLogSink::open(&root, &directory)
            .err()
            .expect("sink fails");

        assert!(matches!(error, SinkOpenError::Io { .. }), "{error}");
        assert_eq!(
            std::fs::read_to_string(&active).expect("active intact"),
            "{\"a\":1}\n{\"b\":"
        );
    }

    /// Makes renaming `active` fail: a read-only directory on Unix, a handle opened without
    /// delete sharing on Windows. The returned guard undoes the arrangement on drop.
    fn block_rename(directory: &Path, active: &Path) -> Box<dyn std::any::Any> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = active;
            struct RestoreMode(std::path::PathBuf);
            impl Drop for RestoreMode {
                fn drop(&mut self) {
                    let _ =
                        std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
                }
            }
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o555))
                .expect("read-only directory");
            Box::new(RestoreMode(directory.to_path_buf()))
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            let _ = directory;
            // FILE_SHARE_READ only: no delete/rename sharing, so the rename is refused.
            Box::new(
                std::fs::OpenOptions::new()
                    .read(true)
                    .share_mode(0x0000_0001)
                    .open(active)
                    .expect("hold without delete sharing"),
            )
        }
    }
}
