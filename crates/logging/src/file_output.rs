use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use time::{Date, macros::format_description};
use tracing_appender::non_blocking::{NonBlocking, NonBlockingBuilder, WorkerGuard};

use crate::{FileLoggingConfig, FileSystemAction, LoggingInitError, RotationPolicy};

/// Contains the prepared writer state needed for one file-backed sink.
pub(crate) struct PreparedFileOutput {
    pub(crate) writer: NonBlocking,
    pub(crate) guard: WorkerGuard,
}

/// Creates the rotating file writer and applies retention cleanup before the sink starts writing.
///
/// The writer is non-blocking and non-lossy: routine IO runs on a background worker, and a full
/// channel exerts backpressure on callers instead of dropping lines.
pub(crate) fn prepare_file_output(
    config: &FileLoggingConfig,
) -> Result<PreparedFileOutput, LoggingInitError> {
    let active_path = ActiveLogPath::from_path(&config.path)?;
    ensure_directory_exists(active_path.directory())?;
    cleanup_old_logs(&active_path, config.max_days.get())?;

    let appender = match config.rotation {
        RotationPolicy::Daily => {
            tracing_appender::rolling::daily(active_path.directory(), active_path.file_name())
        }
    };
    // Prefer backpressure over dropping when the queue is full so file logs stay complete.
    let (writer, guard) = NonBlockingBuilder::default()
        .lossy(/*is_lossy*/ false)
        .finish(appender);

    Ok(PreparedFileOutput { writer, guard })
}

/// Creates the parent directory tree when file-backed logging targets a nested location.
fn ensure_directory_exists(directory: &Path) -> Result<(), LoggingInitError> {
    fs::create_dir_all(directory).map_err(|source| LoggingInitError::FileSystem {
        action: FileSystemAction::CreateDirectory,
        path: directory.to_path_buf(),
        source,
    })
}

/// Deletes only the oldest matching rotated log files until the retained window fits `max_days`.
pub(crate) fn cleanup_old_logs(
    active_path: &ActiveLogPath,
    max_days: usize,
) -> Result<(), LoggingInitError> {
    let directory =
        fs::read_dir(active_path.directory()).map_err(|source| LoggingInitError::FileSystem {
            action: FileSystemAction::ReadDirectory,
            path: active_path.directory().to_path_buf(),
            source,
        })?;

    let mut dated_files = directory
        .filter_map(Result::ok)
        .filter_map(|entry| parse_dated_log_file(&entry.path(), active_path))
        .collect::<Vec<_>>();
    dated_files.sort_by_key(|candidate| candidate.date);

    let files_to_delete = dated_files.len().saturating_sub(max_days);
    for candidate in dated_files.into_iter().take(files_to_delete) {
        fs::remove_file(&candidate.path).map_err(|source| LoggingInitError::FileSystem {
            action: FileSystemAction::RemoveFile,
            path: candidate.path,
            source,
        })?;
    }

    Ok(())
}

/// Recognizes only the files owned by one configured log-file prefix.
fn parse_dated_log_file(path: &Path, active_path: &ActiveLogPath) -> Option<DatedLogFile> {
    let file_name = path.file_name()?.to_str()?;
    let prefix = format!("{}.", active_path.file_name());

    if !file_name.starts_with(&prefix) {
        return None;
    }

    let suffix = &file_name[prefix.len()..];
    let date = Date::parse(suffix, &format_description!("[year]-[month]-[day]")).ok()?;

    Some(DatedLogFile {
        path: path.to_path_buf(),
        date,
    })
}

/// Splits a configured active log path into the directory and filename prefix that rotation needs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ActiveLogPath {
    directory: PathBuf,
    file_name: String,
}

impl ActiveLogPath {
    /// Validates the configured path and extracts the base location used by rotated files.
    pub(crate) fn from_path(path: &Path) -> Result<Self, LoggingInitError> {
        let file_name = path
            .file_name()
            .and_then(OsStr::to_str)
            .map(str::to_string)
            .ok_or_else(|| LoggingInitError::InvalidFilePath {
                path: path.to_path_buf(),
            })?;
        let directory = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));

        Ok(Self {
            directory,
            file_name,
        })
    }

    /// Returns the directory that stores all daily-rotated files for this log stream.
    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }

    /// Returns the filename prefix that identifies one log stream inside its directory.
    pub(crate) fn file_name(&self) -> &str {
        &self.file_name
    }
}

/// Couples one rotated log file path with its parsed date for retention ordering.
#[derive(Clone, Debug, Eq, PartialEq)]
struct DatedLogFile {
    path: PathBuf,
    date: Date,
}
