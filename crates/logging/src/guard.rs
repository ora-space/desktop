use tracing_appender::non_blocking::WorkerGuard;

use crate::LoggingHealthSnapshot;
use crate::health::LoggingHealthHandle;
use crate::retention::RetentionWorkerGuard;

/// Keeps non-blocking file writers alive for as long as the owning process needs them.
#[derive(Debug, Default)]
pub struct LoggingGuard {
    writer_guards: Vec<WorkerGuard>,
    _retention_guards: Vec<RetentionWorkerGuard>,
    health: LoggingHealthHandle,
}

impl LoggingGuard {
    /// Creates a guard that owns writer and retention lifetimes for every file-backed sink.
    pub(crate) fn new(
        writer_guards: Vec<WorkerGuard>,
        retention_guards: Vec<RetentionWorkerGuard>,
        health: LoggingHealthHandle,
    ) -> Self {
        Self {
            writer_guards,
            _retention_guards: retention_guards,
            health,
        }
    }

    /// Reports whether the active logging setup owns any file-backed writers.
    pub fn has_file_writer(&self) -> bool {
        !self.writer_guards.is_empty()
    }

    /// Returns the current logging degradation state and cumulative failure counters.
    pub fn health(&self) -> LoggingHealthSnapshot {
        self.health.snapshot()
    }

    /// Returns a cloneable handle for backend health endpoints and diagnostic services.
    pub fn health_handle(&self) -> LoggingHealthHandle {
        self.health.clone()
    }
}
