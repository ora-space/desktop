use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use time::OffsetDateTime;

/// Supplies wall and monotonic time without coupling appender tests to the system clock.
pub(crate) trait TimeSource: Send + 'static {
    /// Returns the worker's current absolute instant used to select the active local date.
    fn now(&self) -> OffsetDateTime;

    /// Returns monotonic elapsed time used to rate-limit retries across wall-clock changes.
    fn monotonic_elapsed(&self) -> Duration;
}

/// Opens append-only log files so rotation failures can be exercised without platform tricks.
pub(crate) trait FileOpener: Send + 'static {
    /// Opens or creates the file selected for one local processing date.
    fn open(&self, path: &Path) -> io::Result<File>;
}

/// Reads wall and monotonic time from the operating system for production file outputs.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SystemTimeSource {
    started_at: Instant,
}

impl Default for SystemTimeSource {
    fn default() -> Self {
        Self {
            started_at: Instant::now(),
        }
    }
}

impl TimeSource for SystemTimeSource {
    fn now(&self) -> OffsetDateTime {
        // UTC is only the unambiguous source instant; the appender applies its configured IANA
        // timezone before this value can select a filename or rollover boundary.
        OffsetDateTime::now_utc()
    }

    fn monotonic_elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }
}

/// Uses standard append semantics for production log files.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SystemFileOpener;

impl FileOpener for SystemFileOpener {
    fn open(&self, path: &Path) -> io::Result<File> {
        OpenOptions::new().create(true).append(true).open(path)
    }
}
