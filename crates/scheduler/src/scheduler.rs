use std::str::FromStr;

use chrono::Utc;
use chrono_tz::Tz;
use cron::Schedule;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

use ora_logging::{ora_debug, ora_info};

use crate::{Job, SchedulerError};

/// Owns a set of registered cron jobs and drives each one in a dedicated tokio task.
pub struct Scheduler {
    timezone: Tz,
    jobs: Vec<Box<dyn Job>>,
}

impl Scheduler {
    /// Creates an empty scheduler whose cron expressions use the supplied IANA timezone.
    pub fn new(timezone: Tz) -> Self {
        Self {
            timezone,
            jobs: Vec::new(),
        }
    }

    /// Registers a job with the scheduler and returns `self` for fluent chaining.
    pub fn register(&mut self, job: impl Job + 'static) -> &mut Self {
        self.jobs.push(Box::new(job));
        self
    }

    /// Parses all cron expressions and spawns one tokio task per job.
    ///
    /// Each task loops indefinitely: it sleeps until the next scheduled tick, runs the job,
    /// then waits for the next tick.  The loop exits when `cancel` is signalled.
    /// Returns a single `JoinHandle` that resolves once all job tasks have stopped.
    pub fn start(self, cancel: CancellationToken) -> Result<JoinHandle<()>, SchedulerError> {
        let timezone = self.timezone;
        let mut parsed: Vec<(Schedule, Box<dyn Job>)> = Vec::with_capacity(self.jobs.len());

        for job in self.jobs {
            let schedule = Schedule::from_str(job.schedule()).map_err(|source| {
                SchedulerError::InvalidCronExpression {
                    schedule: job.schedule().to_string(),
                    source,
                }
            })?;
            parsed.push((schedule, job));
        }

        let handle = tokio::spawn(async move {
            let mut task_handles: Vec<JoinHandle<()>> = Vec::with_capacity(parsed.len());

            for (schedule, job) in parsed {
                let cancel = cancel.clone();
                let task = tokio::spawn(run_job_loop(schedule, timezone, job, cancel));
                task_handles.push(task);
            }

            for handle in task_handles {
                // Ignore join errors — the cancellation token is the shutdown mechanism.
                let _ = handle.await;
            }
        });

        Ok(handle)
    }
}

/// Runs one job on every scheduled tick until the cancellation token fires.
async fn run_job_loop(
    schedule: Schedule,
    timezone: Tz,
    job: Box<dyn Job>,
    cancel: CancellationToken,
) {
    let span = tracing::info_span!(
        "scheduler.job",
        job.name = job.name(),
        schedule = job.schedule()
    );

    async move {
        for next_tick in schedule.upcoming(timezone) {
            let now = Utc::now().with_timezone(&timezone);
            let delay = (next_tick - now)
                .to_std()
                .unwrap_or(std::time::Duration::ZERO);

            tokio::select! {
                _ = tokio::time::sleep(delay) => {
                    ora_debug!(job.name = job.name(), "running scheduled job");
                    job.run().await;
                    ora_debug!(job.name = job.name(), "scheduled job complete");
                }
                _ = cancel.cancelled() => {
                    ora_info!(job.name = job.name(), "scheduler stopping due to cancellation");
                    return;
                }
            }
        }
    }
    .instrument(span)
    .await;
}
