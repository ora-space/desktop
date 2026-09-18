use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::ParseLogLevelError;

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
///
/// Variants are declared from least to most severe so the derived ordering doubles as the
/// threshold comparison used by every level filter (`record >= threshold` keeps the record).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Ord, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    /// Returns the stable lowercase name shared by environment and serialized configuration.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }

    /// Returns the uppercase name used inside persisted JSONL records, matching `tracing`.
    pub const fn as_upper_str(self) -> &'static str {
        match self {
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

impl std::fmt::Display for LogLevel {
    /// Formats the stable lowercase representation used at runtime seams.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for LogLevel {
    type Err = ParseLogLevelError;

    /// Parses a trimmed, ASCII-case-insensitive runtime log-level name.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "trace" => Ok(Self::Trace),
            "debug" => Ok(Self::Debug),
            "info" => Ok(Self::Info),
            "warn" => Ok(Self::Warn),
            "error" => Ok(Self::Error),
            _ => Err(ParseLogLevelError::new(value)),
        }
    }
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
