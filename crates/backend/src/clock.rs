use ora_application::Clock;
use std::time::{SystemTime, UNIX_EPOCH};

/// Supplies wall-clock timestamps to the concrete backend handler composition.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SystemClock;

impl Clock for SystemClock {
    /// Returns the current Unix timestamp in milliseconds for persisted audit fields.
    fn now_timestamp_millis(&self) -> i64 {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(duration) => duration.as_millis() as i64,
            Err(_) => 0,
        }
    }
}

impl ora_utils::clock::TimestampSource for SystemClock {
    /// Uses Ora's configured local clock for Effect business-stage timestamps.
    fn current_timestamp_millis(&self) -> i64 {
        ora_logging::clock::now_millis()
    }
}
