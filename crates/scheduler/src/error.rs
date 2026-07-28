use thiserror::Error;

/// Errors that can occur when setting up or running the cron scheduler.
#[derive(Debug, Error)]
pub enum SchedulerError {
    #[error("invalid cron expression {schedule:?}: {source}")]
    InvalidCronExpression {
        schedule: String,
        source: cron::error::Error,
    },
}
