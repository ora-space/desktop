use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use time::Date;
use tracing_appender::non_blocking::{NonBlocking, NonBlockingBuilder, WorkerGuard};

use crate::appender_dependencies::{SystemFileOpener, SystemTimeSource};
use crate::health::LoggingHealthHandle;
use crate::local_daily_appender::LocalDailyAppender;
use crate::retention::{FilesystemRetentionCleaner, RetentionWorkerGuard};
use crate::{FileLoggingConfig, FileSystemAction, LoggingInitError, RotationPolicy};

/// Contains the prepared writer state needed for one file-backed sink.
pub(crate) struct PreparedFileOutput {
    pub(crate) writer: NonBlocking,
    pub(crate) writer_guard: WorkerGuard,
    pub(crate) retention_guard: RetentionWorkerGuard,
    pub(crate) health: LoggingHealthHandle,
}

/// Creates the local-calendar rotating writer used by one file-backed sink.
pub(crate) fn prepare_file_output(
    config: &FileLoggingConfig,
    timezone: chrono_tz::Tz,
) -> Result<PreparedFileOutput, LoggingInitError> {
    let active_path = ActiveLogPath::from_path(&config.path)?;
    ensure_directory_exists(active_path.directory())?;
    let health = LoggingHealthHandle::default();

    let runtime = match config.rotation {
        RotationPolicy::Daily => LocalDailyAppender::prepare(
            active_path.clone(),
            timezone,
            SystemTimeSource::default(),
            SystemFileOpener,
            FilesystemRetentionCleaner::new(active_path, config.max_days),
            health.recorder(),
        )?,
    };
    let (writer, writer_guard) = NonBlockingBuilder::default()
        .lossy(/*is_lossy*/ true)
        .finish(runtime.appender);
    health.add_drop_counter(writer.error_counter());

    Ok(PreparedFileOutput {
        writer,
        writer_guard,
        retention_guard: runtime.retention_guard,
        health,
    })
}

/// Creates the parent directory tree when file-backed logging targets a nested location.
fn ensure_directory_exists(directory: &Path) -> Result<(), LoggingInitError> {
    fs::create_dir_all(directory).map_err(|source| LoggingInitError::FileSystem {
        action: FileSystemAction::CreateDirectory,
        path: directory.to_path_buf(),
        source,
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

    /// Builds the path for one local calendar date in this rotated log series.
    pub(crate) fn path_for_date(&self, date: Date) -> PathBuf {
        self.directory.join(format!("{}.{date}", self.file_name))
    }
}
