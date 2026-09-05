pub use ora_utils::clock::TimestampSource;
use std::time::{SystemTime, UNIX_EPOCH};

/// Reads migration timestamps before logging or its local timezone has been initialized.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemTimestampSource;

impl TimestampSource for SystemTimestampSource {
    /// Converts the system clock into the integer millisecond format stored in SQLite.
    fn current_timestamp_millis(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64
    }
}

/// Reads Effect persistence timestamps after the application initializes its local clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalTimestampSource;

impl TimestampSource for LocalTimestampSource {
    /// Uses the configured local clock without introducing a process-timezone fallback.
    fn current_timestamp_millis(&self) -> i64 {
        ora_logging::clock::now_millis()
    }
}
