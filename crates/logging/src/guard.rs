use tracing_appender::non_blocking::WorkerGuard;

/// Keeps non-blocking file writers alive for as long as the owning process needs them.
#[derive(Debug, Default)]
pub struct LoggingGuard {
    guards: Vec<WorkerGuard>,
}

impl LoggingGuard {
    /// Creates a guard that owns the worker lifetimes for every active non-blocking sink.
    pub fn new(guards: Vec<WorkerGuard>) -> Self {
        Self { guards }
    }

    /// Reports whether the active logging setup owns any non-blocking writer workers.
    pub fn has_worker_guard(&self) -> bool {
        !self.guards.is_empty()
    }
}
