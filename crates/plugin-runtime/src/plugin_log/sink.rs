//! Owns the active `plugin.log` file of one process generation.
//!
//! Plugin logs live in a host-managed root that no plugin API can address, but the host still
//! treats the path as hostile until proven otherwise: every directory level between the root
//! and the active file must be a plain directory that canonicalizes inside the root, the file
//! must be a plain file, and any doubt is a conflict that leaves the path exactly as found.

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Component, Path, PathBuf};

/// Name of the active log file inside a plugin's log directory.
pub const ACTIVE_LOG_FILE_NAME: &str = "plugin.log";

/// Why a sink could not be opened; `Conflict` means the path was left untouched on purpose.
#[derive(Debug)]
pub enum SinkOpenError {
    Conflict { path: PathBuf, reason: &'static str },
    Io { path: PathBuf, source: io::Error },
}

impl std::fmt::Display for SinkOpenError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflict { path, reason } => {
                write!(formatter, "{reason} at {}", path.display())
            }
            Self::Io { path, source } => write!(formatter, "{source} at {}", path.display()),
        }
    }
}

/// Append-only writer over the active log file.
#[derive(Debug)]
pub struct PluginLogSink {
    writer: BufWriter<File>,
}

impl PluginLogSink {
    /// Opens (creating when absent) `<directory>/plugin.log`, where `directory` lies below the
    /// host-managed `root`, without following any link at any level.
    ///
    /// The root is created if missing and canonicalized once; each level of the relative path is
    /// then created only when nothing exists under that name, reused when it is a plain
    /// directory, and refused otherwise. A final canonicalization proves the directory really
    /// is where the lexical join says it is.
    pub fn open(root: &Path, directory: &Path) -> Result<Self, SinkOpenError> {
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
        std::fs::create_dir_all(root).map_err(|source| SinkOpenError::Io {
            path: root.to_path_buf(),
            source,
        })?;
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
        let path = current.join(ACTIVE_LOG_FILE_NAME);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_file() => {}
            Ok(_) => {
                return Err(SinkOpenError::Conflict {
                    path,
                    reason: "active log path exists but is not a regular file",
                });
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(source) => return Err(SinkOpenError::Io { path, source }),
        }
        let file = open_without_following_links(&path).map_err(|source| SinkOpenError::Io {
            path: path.clone(),
            source,
        })?;
        // The pre-open check and the open are not atomic; re-checking through the handle closes
        // the window in which a link could have been swapped in between them.
        let metadata = file.metadata().map_err(|source| SinkOpenError::Io {
            path: path.clone(),
            source,
        })?;
        if !metadata.file_type().is_file() {
            return Err(SinkOpenError::Conflict {
                path,
                reason: "active log path changed to a non-regular file while opening",
            });
        }
        Ok(Self {
            writer: BufWriter::new(file),
        })
    }

    /// Appends one already-rendered JSON line.
    pub fn write_line(&mut self, line: &str) -> io::Result<()> {
        self.writer.write_all(line.as_bytes())
    }

    /// Pushes buffered lines to the operating system.
    pub fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
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

/// Opens the active file in append mode with the platform flag that refuses to traverse a link
/// at the final path component.
fn open_without_following_links(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_OPEN_REPARSE_POINT: open the reparse point itself rather than its target, so
        // the post-open file-type check sees the link instead of whatever it points at.
        options.custom_flags(0x0020_0000);
    }
    options.open(path)
}

#[cfg(test)]
mod tests {
    use super::{PluginLogSink, SinkOpenError};
    use pretty_assertions::assert_eq;
    use std::path::Path;
    use tempfile::TempDir;

    /// The root and every level below it are created, records append across reopen, and an
    /// existing plain tree is reused rather than replaced.
    #[test]
    fn creates_every_level_reuses_and_appends() {
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
            std::fs::read_to_string(directory.join("plugin.log")).expect("read log"),
            "{\"a\":1}\n{\"b\":2}\n"
        );
    }

    /// A file under a directory level, a non-regular active file, and a directory outside the
    /// root are conflicts that leave the filesystem untouched.
    #[test]
    fn foreign_paths_are_a_conflict_and_stay_untouched() {
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
        assert!(matches!(error, SinkOpenError::Conflict { .. }), "{error}");
        assert_eq!(
            (
                std::fs::read_dir(&elsewhere).expect("elsewhere").count(),
                std::fs::read_to_string(&target).expect("target"),
            ),
            (0, String::new())
        );
    }
}
