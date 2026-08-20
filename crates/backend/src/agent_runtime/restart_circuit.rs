use std::collections::VecDeque;
use std::time::{Duration, Instant};

const CRASH_LIMIT: usize = 3;
const CRASH_WINDOW: Duration = Duration::from_secs(60);

/// Decides whether one independently supervised agent may start another generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RestartDecision {
    Retry,
    Stop,
}

/// Retains the bounded failure history needed to stop one agent's crash loop.
#[derive(Debug, Default)]
pub(super) struct RestartCircuit {
    failures: VecDeque<Instant>,
}

impl RestartCircuit {
    /// Records one unexpected failure and opens the circuit after more than three in one minute.
    pub(super) fn record_failure(&mut self, now: Instant) -> RestartDecision {
        let cutoff = now.checked_sub(CRASH_WINDOW);
        while self
            .failures
            .front()
            .is_some_and(|failure| cutoff.is_some_and(|cutoff| *failure <= cutoff))
        {
            self.failures.pop_front();
        }
        self.failures.push_back(now);
        if self.failures.len() > CRASH_LIMIT {
            RestartDecision::Stop
        } else {
            RestartDecision::Retry
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CRASH_WINDOW, RestartCircuit, RestartDecision};
    use pretty_assertions::assert_eq;
    use std::time::{Duration, Instant};

    /// Four failures inside one minute open the circuit after the first three retries.
    #[test]
    fn stops_after_more_than_three_failures_inside_the_window() {
        let started_at = Instant::now();
        let mut circuit = RestartCircuit::default();

        assert_eq!(
            (0..4)
                .map(|offset| {
                    circuit.record_failure(started_at + Duration::from_secs(offset * 10))
                })
                .collect::<Vec<_>>(),
            vec![
                RestartDecision::Retry,
                RestartDecision::Retry,
                RestartDecision::Retry,
                RestartDecision::Stop,
            ]
        );
    }

    /// A failure exactly one window old no longer contributes to the crash-loop threshold.
    #[test]
    fn expires_failures_outside_the_window() {
        let started_at = Instant::now();
        let mut circuit = RestartCircuit::default();
        for offset in 0..3 {
            assert_eq!(
                circuit.record_failure(started_at + Duration::from_secs(offset)),
                RestartDecision::Retry
            );
        }

        assert_eq!(
            circuit.record_failure(started_at + CRASH_WINDOW),
            RestartDecision::Retry
        );
    }
}
