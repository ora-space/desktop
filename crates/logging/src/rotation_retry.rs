use std::time::Duration;

use time::Date;

const INITIAL_ROTATION_RETRY_DELAY: Duration = Duration::from_secs(/*secs*/ 1);
const MAX_ROTATION_RETRY_DELAY: Duration = Duration::from_secs(/*secs*/ 60);

/// Encodes whether a failed rollover may be retried at the current monotonic instant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RotationRetry {
    Ready,
    Waiting {
        target_date: Date,
        retry_at: Duration,
        next_delay: Duration,
    },
}

impl RotationRetry {
    /// Allows immediate attempts for new dates while respecting backoff for the same target date.
    pub(crate) fn allows_attempt(self, target_date: Date, now: Duration) -> bool {
        match self {
            Self::Ready => true,
            Self::Waiting {
                target_date: waiting_date,
                retry_at,
                ..
            } => target_date != waiting_date || now >= retry_at,
        }
    }

    /// Schedules an exponentially delayed retry capped to keep recovery reasonably prompt.
    pub(crate) fn record_failure(&mut self, target_date: Date, now: Duration) {
        let delay = match *self {
            Self::Waiting {
                target_date: waiting_date,
                next_delay,
                ..
            } if waiting_date == target_date => next_delay,
            Self::Ready | Self::Waiting { .. } => INITIAL_ROTATION_RETRY_DELAY,
        };
        *self = Self::Waiting {
            target_date,
            retry_at: now.saturating_add(delay),
            next_delay: delay
                .saturating_mul(/*rhs*/ 2)
                .min(MAX_ROTATION_RETRY_DELAY),
        };
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use pretty_assertions::assert_eq;
    use time::macros::date;

    use super::RotationRetry;

    /// Verifies repeated failures double their delay until the configured maximum is reached.
    #[test]
    fn applies_capped_exponential_backoff() {
        let target_date = date!(2026 - 07 - 31);
        let mut retry = RotationRetry::Ready;
        let mut now = Duration::ZERO;
        let delays = [1, 2, 4, 8, 16, 32, 60, 60].map(Duration::from_secs);

        for delay in delays {
            retry.record_failure(target_date, now);
            let expected_next_delay = delay
                .saturating_mul(/*rhs*/ 2)
                .min(Duration::from_secs(/*secs*/ 60));
            assert_eq!(
                retry,
                RotationRetry::Waiting {
                    target_date,
                    retry_at: now.saturating_add(delay),
                    next_delay: expected_next_delay,
                }
            );
            assert_eq!(retry.allows_attempt(target_date, now), false);
            now = now.saturating_add(delay);
            assert_eq!(retry.allows_attempt(target_date, now), true);
        }
    }

    /// Verifies a newer target date bypasses backoff left by an older failed rollover.
    #[test]
    fn permits_a_new_target_date_immediately() {
        let mut retry = RotationRetry::Ready;
        retry.record_failure(date!(2026 - 07 - 30), Duration::ZERO);

        assert_eq!(
            retry.allows_attempt(date!(2026 - 07 - 31), Duration::ZERO),
            true
        );
    }
}
