use std::num::NonZeroUsize;
use std::path::PathBuf;

/// Describes the process-wide logging behavior installed by `ora-logging`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoggingConfig {
    pub level: LogLevel,
    pub output: LogOutput,
    pub timezone: chrono_tz::Tz,
}

impl LoggingConfig {
    /// Builds a logging configuration from an explicit level, output mode, and process timezone.
    pub fn new(level: LogLevel, output: LogOutput, timezone: chrono_tz::Tz) -> Self {
        Self {
            level,
            output,
            timezone,
        }
    }
}

/// Enumerates the supported event filtering levels for shared runtime logging.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// Names the supported output topologies without relying on booleans at call sites.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LogOutput {
    Stdout,
    File(FileLoggingConfig),
    StdoutAndFile(FileLoggingConfig),
}

/// Captures the file-specific logging settings used by file-backed outputs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileLoggingConfig {
    pub path: PathBuf,
    pub rotation: RotationPolicy,
    pub max_days: NonZeroUsize,
}

impl FileLoggingConfig {
    /// Builds the file-backed logging settings from a path, rotation policy, and retention window.
    pub fn new(path: impl Into<PathBuf>, rotation: RotationPolicy, max_days: NonZeroUsize) -> Self {
        Self {
            path: path.into(),
            rotation,
            max_days,
        }
    }
}

/// Lists the rotation strategies supported by the first logging implementation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RotationPolicy {
    Daily,
}
