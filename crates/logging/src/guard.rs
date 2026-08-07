use tracing_appender::non_blocking::{ErrorCounter, WorkerGuard};

/// Keeps non-blocking writers alive for as long as the owning process needs them.
///
/// Owns worker guards for every active non-blocking sink (stdout and/or file). Dropping the
/// guard early shuts those workers down and can lose buffered output.
///
/// Non-blocking sinks stay lossy so logging never blocks request or UI threads. When a writer
/// channel is full, lines are dropped; this guard retains the appender's drop counters so
/// those losses stay observable for the process lifetime and are summarized on drop.
#[derive(Debug, Default)]
pub struct LoggingGuard {
    guards: Vec<WorkerGuard>,
    drop_counters: Vec<ErrorCounter>,
}

impl LoggingGuard {
    /// Creates a guard that owns worker lifetimes and drop counters for every non-blocking sink.
    pub(crate) fn new(guards: Vec<WorkerGuard>, drop_counters: Vec<ErrorCounter>) -> Self {
        debug_assert_eq!(guards.len(), drop_counters.len());
        Self {
            guards,
            drop_counters,
        }
    }

    /// Reports whether the active logging setup owns any non-blocking writer workers.
    pub fn has_worker_guard(&self) -> bool {
        !self.guards.is_empty()
    }

    /// Returns how many non-blocking sink lines were dropped because a writer channel was full.
    pub fn dropped_lines(&self) -> usize {
        self.drop_counters
            .iter()
            .map(ErrorCounter::dropped_lines)
            .sum()
    }
}

impl Drop for LoggingGuard {
    /// Emits a stderr summary when any lossy non-blocking sink lines were dropped.
    fn drop(&mut self) {
        let dropped = self.dropped_lines();
        if dropped > 0 {
            eprintln!(
                "ora-logging: dropped {dropped} log line(s) because a non-blocking writer channel was full"
            );
        }
    }
}
