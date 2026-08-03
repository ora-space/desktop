use tracing_appender::non_blocking::{ErrorCounter, WorkerGuard};

/// Keeps non-blocking file writers alive for as long as the owning process needs them.
///
/// File sinks stay lossy so logging never blocks request or UI threads. When the writer
/// channel is full, lines are dropped; this guard retains the appender's drop counters so
/// those losses stay observable for the process lifetime and are summarized on drop.
#[derive(Debug, Default)]
pub struct LoggingGuard {
    guards: Vec<WorkerGuard>,
    drop_counters: Vec<ErrorCounter>,
}

impl LoggingGuard {
    /// Creates a guard that owns the writer lifetimes and drop counters for every file-backed sink.
    pub(crate) fn new(guards: Vec<WorkerGuard>, drop_counters: Vec<ErrorCounter>) -> Self {
        debug_assert_eq!(guards.len(), drop_counters.len());
        Self {
            guards,
            drop_counters,
        }
    }

    /// Reports whether the active logging setup owns any file-backed writers.
    pub fn has_file_writer(&self) -> bool {
        !self.guards.is_empty()
    }

    /// Returns how many file-sink lines were dropped because the non-blocking channel was full.
    pub fn dropped_lines(&self) -> usize {
        self.drop_counters
            .iter()
            .map(ErrorCounter::dropped_lines)
            .sum()
    }
}

impl Drop for LoggingGuard {
    /// Emits a stderr summary when any lossy file-sink lines were dropped during the process lifetime.
    fn drop(&mut self) {
        let dropped = self.dropped_lines();
        if dropped > 0 {
            eprintln!(
                "ora-logging: dropped {dropped} file log line(s) because the non-blocking writer channel was full"
            );
        }
    }
}
