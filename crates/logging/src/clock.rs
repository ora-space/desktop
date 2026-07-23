//! Provides Ora's process-wide local clock.
//!
//! This module does not use `OffsetDateTime::now_local()` because platform local-offset discovery
//! can return `IndeterminateOffset` in otherwise valid runtime environments. Ora instead resolves
//! an IANA timezone during application startup and uses its cached rules for every local timestamp.

use std::sync::OnceLock;

use chrono::{Offset as _, TimeZone as _};
use time::{OffsetDateTime, UtcOffset};

/// Owns one immutable timezone so tests can exercise clock behavior without sharing global state.
struct Clock {
    timezone: OnceLock<chrono_tz::Tz>,
}

impl Clock {
    /// Creates an uninitialized clock whose timezone must be supplied before local-time access.
    const fn new() -> Self {
        Self {
            timezone: OnceLock::new(),
        }
    }

    /// Fixes this clock's timezone for its lifetime.
    fn initialize(&self, timezone: chrono_tz::Tz) -> Result<(), chrono_tz::Tz> {
        self.timezone.set(timezone)
    }

    /// Returns the configured timezone or fails fast when startup skipped clock initialization.
    fn timezone(&self) -> chrono_tz::Tz {
        match self.timezone.get() {
            Some(timezone) => *timezone,
            None => panic!("ora-logging clock must be initialized before use"),
        }
    }

    /// Returns the current time expressed in this clock's configured timezone.
    fn now_local(&self) -> OffsetDateTime {
        now_local_in(self.timezone())
    }
}

/// Stores the timezone selected by the process composition root during logging initialization.
static PROCESS_CLOCK: Clock = Clock::new();

/// Initializes the process-wide timezone exactly once during logging startup.
pub(crate) fn initialize(timezone: chrono_tz::Tz) -> Result<(), chrono_tz::Tz> {
    PROCESS_CLOCK.initialize(timezone)
}

/// Returns the current time expressed in the process-wide configured timezone.
pub fn now_local() -> OffsetDateTime {
    PROCESS_CLOCK.now_local()
}

/// Returns the current Unix timestamp in milliseconds.
///
/// The nanosecond value is divided before narrowing so a representable millisecond timestamp cannot
/// be truncated merely because its nanosecond representation exceeds `i64`.
pub fn now_millis() -> i64 {
    timestamp_millis(now_local().unix_timestamp_nanos())
}

/// Returns the current configured local UTC offset.
pub fn local_offset() -> UtcOffset {
    offset_for_unix_timestamp(
        PROCESS_CLOCK.timezone(),
        OffsetDateTime::now_utc().unix_timestamp(),
    )
}

/// Returns the configured local UTC offset in milliseconds for SQL wall-clock reconstruction.
pub fn local_offset_millis() -> i64 {
    i64::from(local_offset().whole_seconds()) * 1000
}

/// Expresses the current instant in an explicitly supplied timezone for injected consumers.
pub(crate) fn now_local_in(timezone: chrono_tz::Tz) -> OffsetDateTime {
    // UTC is only the unambiguous source instant; `localize` applies the configured presentation
    // offset before this value reaches callers or structured logs.
    localize(OffsetDateTime::now_utc(), timezone)
}

/// Expresses a fixed UTC instant in an IANA timezone while preserving the represented instant.
fn localize(utc_now: OffsetDateTime, timezone: chrono_tz::Tz) -> OffsetDateTime {
    let offset = offset_for_unix_timestamp(timezone, utc_now.unix_timestamp());
    utc_now.to_offset(offset)
}

/// Resolves the UTC offset for a specific instant so daylight-saving transitions remain accurate.
fn offset_for_unix_timestamp(timezone: chrono_tz::Tz, timestamp: i64) -> UtcOffset {
    let local = match timezone.timestamp_opt(timestamp, 0).single() {
        Some(local) => local,
        None => panic!("an in-range Unix timestamp must map to one IANA timezone instant"),
    };
    let offset_seconds = local.offset().fix().local_minus_utc();

    match UtcOffset::from_whole_seconds(offset_seconds) {
        Ok(offset) => offset,
        Err(error) => {
            panic!("an IANA timezone offset must fit within time::UtcOffset: {error}")
        }
    }
}

/// Converts nanoseconds to milliseconds before performing the checked integer narrowing.
fn timestamp_millis(timestamp_nanos: i128) -> i64 {
    let milliseconds = timestamp_nanos / 1_000_000;

    match i64::try_from(milliseconds) {
        Ok(milliseconds) => milliseconds,
        Err(error) => panic!("current Unix milliseconds must fit within i64: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use time::{
        Month, UtcOffset,
        macros::{datetime, offset},
    };

    use super::{Clock, localize, timestamp_millis};

    /// Verifies independent clock instances retain distinct immutable timezone configuration.
    #[test]
    fn keeps_timezone_configuration_isolated_per_clock() {
        let shanghai = Clock::new();
        let london = Clock::new();

        assert_eq!(shanghai.initialize(chrono_tz::Asia::Shanghai), Ok(()));
        assert_eq!(london.initialize(chrono_tz::Europe::London), Ok(()));
        assert_eq!(shanghai.initialize(chrono_tz::UTC), Err(chrono_tz::UTC));
        assert_eq!(shanghai.timezone(), chrono_tz::Asia::Shanghai);
        assert_eq!(london.timezone(), chrono_tz::Europe::London);
    }

    /// Verifies Shanghai uses its stable eight-hour offset for a fixed instant.
    #[test]
    fn localizes_fixed_instants_for_shanghai() {
        let utc = datetime!(2026-07-23 12:00 UTC);

        assert_eq!(
            localize(utc, chrono_tz::Asia::Shanghai),
            datetime!(2026-07-23 20:00 +08:00)
        );
    }

    /// Verifies IANA rules select London's winter and summer offsets for their respective instants.
    #[test]
    fn localizes_london_across_daylight_saving_time() {
        let winter = datetime!(2026-01-15 12:00 UTC);
        let summer = datetime!(2026-07-15 12:00 UTC);

        assert_eq!(
            localize(winter, chrono_tz::Europe::London).offset(),
            UtcOffset::UTC
        );
        assert_eq!(
            localize(summer, chrono_tz::Europe::London).offset(),
            offset!(+1)
        );
        assert_eq!(
            localize(summer, chrono_tz::Europe::London).month(),
            Month::July
        );
    }

    /// Verifies millisecond conversion narrows only after reducing nanosecond magnitude.
    #[test]
    fn converts_nanoseconds_before_narrowing() {
        let largest_i64_milliseconds = i128::from(i64::MAX) * 1_000_000;

        assert_eq!(timestamp_millis(largest_i64_milliseconds), i64::MAX);
    }

    /// Verifies local-time access cannot silently select a timezone before initialization.
    #[test]
    #[should_panic(expected = "ora-logging clock must be initialized before use")]
    fn rejects_local_time_access_before_initialization() {
        Clock::new().now_local();
    }
}
