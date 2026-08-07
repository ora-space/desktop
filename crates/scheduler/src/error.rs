use thiserror::Error;

/// Errors returned when registering work with the [`crate::Scheduler`].
#[derive(Debug, Error)]
pub enum SchedulerError {
    /// The supplied cron expression could not be parsed; no task was spawned.
    #[error("invalid cron expression {schedule:?}: {source}")]
    InvalidCronExpression {
        schedule: String,
        source: cron::error::Error,
    },
    /// The scheduler has been shut down and rejects new registrations.
    #[error("scheduler is shut down")]
    ShuttingDown,
}
