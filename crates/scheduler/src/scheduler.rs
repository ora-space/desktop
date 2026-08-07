use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Local;
use chrono_tz::Tz;
use cron::Schedule;
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinSet;
use tokio::time::sleep;

use ora_logging::{ora_debug, ora_info, ora_warn};

use crate::handle::{CronControl, DelayControl};
use crate::{CronHandle, DelayHandle, Job, SchedulerError};

/// Boxed, pinned, owned future used as the `schedule_after` payload. Encapsulates
/// `Pin<Box<dyn Future<Output = ()> + Send + 'static>>` so the public signature stays readable.
pub(crate) type OwnedFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// Commands sent to the supervisor task by [`Scheduler`] clones.
enum SchedulerCommand {
    ScheduleCron {
        schedule: Box<Schedule>,
        job: Box<dyn Job>,
        control: Arc<CronControl>,
    },
    ScheduleAfter {
        delay: Duration,
        future: OwnedFuture,
        control: Arc<DelayControl>,
    },
    Shutdown,
}

/// Per-scheduler state shared by every clone.
///
/// `is_shutdown` lives behind the same `Mutex` that guards the `send`, so registration and
/// shutdown are mutually exclusive: a registration either observes `is_shutdown == false` and
/// successfully queues its command, or it observes `is_shutdown == true` and rejects without
/// ever producing a phantom handle for a task that will not run.
struct SchedulerInner {
    command_tx: mpsc::UnboundedSender<SchedulerCommand>,
    is_shutdown: std::sync::Mutex<bool>,
}

/// Publishes one terminal state that every shutdown caller can await independently.
struct ShutdownCompletion {
    completed: AtomicBool,
    notify: Notify,
}

impl ShutdownCompletion {
    /// Creates a completion latch that remains pending until the supervisor drains its tasks.
    fn new() -> Self {
        Self {
            completed: AtomicBool::new(false),
            notify: Notify::new(),
        }
    }

    /// Marks the supervisor complete and wakes every caller waiting for shutdown.
    fn complete(&self) {
        self.completed.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }

    /// Waits for the supervisor to finish without owning or consuming a join handle.
    async fn wait(&self) {
        loop {
            if self.completed.load(Ordering::Acquire) {
                return;
            }
            let notified = self.notify.notified();
            if self.completed.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }
}

/// A cloneable scheduler that drives recurring cron jobs and one-shot delayed futures in the
/// background. Once created with [`Scheduler::new`] it accepts dynamic registrations until
/// [`Scheduler::shutdown`] is awaited.
///
/// The scheduler takes an explicit IANA timezone supplied at construction so cron expressions
/// fire at the configured local time. It uses the current Tokio runtime when one exists and owns
/// a small current-thread runtime otherwise, so synchronous composition roots can create it too.
///
/// Tokio's `JoinHandle` and cancellation tokens are intentionally absent from this surface;
/// callers express ownership solely through [`CronHandle`] and [`DelayHandle`].
#[derive(Clone)]
pub struct Scheduler {
    inner: Arc<SchedulerInner>,
    completion: Arc<ShutdownCompletion>,
}

impl Scheduler {
    /// Spawns the background supervisor and returns a cloneable handle ready to register jobs.
    ///
    /// The supervisor begins running immediately and consumes registrations until `shutdown` is
    /// awaited (or until every `Scheduler` clone is dropped, in which case it drains its tasks
    /// autonomously and exits).
    ///
    pub fn new(timezone: Tz) -> Self {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let inner = Arc::new(SchedulerInner {
            command_tx,
            is_shutdown: Mutex::new(false),
        });
        let completion = Arc::new(ShutdownCompletion::new());
        match tokio::runtime::Handle::try_current() {
            Ok(_) => {
                drop(tokio::spawn(run_supervisor(
                    command_rx,
                    timezone,
                    Arc::clone(&completion),
                )));
            }
            Err(_) => {
                let thread_completion = Arc::clone(&completion);
                let _ = std::thread::spawn(move || {
                    let runtime = match tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        Ok(runtime) => runtime,
                        Err(error) => panic!("failed to create scheduler runtime: {error}"),
                    };
                    runtime.block_on(run_supervisor(command_rx, timezone, thread_completion));
                });
            }
        }
        Scheduler { inner, completion }
    }

    /// Dynamically registers a recurring cron job.
    ///
    /// The cron expression is parsed eagerly via [`Schedule::from_str`]; an invalid expression
    /// returns [`SchedulerError::InvalidCronExpression`] and spawns nothing. The returned
    /// [`CronHandle`] stops the loop when dropped, unless [`CronHandle::detach`] is called.
    pub fn schedule_cron(&self, job: impl Job + 'static) -> Result<CronHandle, SchedulerError> {
        let mut shutdown = self
            .inner
            .is_shutdown
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *shutdown {
            return Err(SchedulerError::ShuttingDown);
        }
        let schedule_repr = job.schedule().to_owned();
        let schedule = Schedule::from_str(&schedule_repr).map_err(|source| {
            SchedulerError::InvalidCronExpression {
                schedule: schedule_repr.clone(),
                source,
            }
        })?;
        let job_name = job.name().to_owned();
        let control = Arc::new(CronControl::new());
        let queued = self.inner.command_tx.send(SchedulerCommand::ScheduleCron {
            schedule: Box::new(schedule),
            job: Box::new(job),
            control: Arc::clone(&control),
        });
        if queued.is_err() {
            // The supervisor has exited; treat it as a shutting-down rejection.
            *shutdown = true;
            return Err(SchedulerError::ShuttingDown);
        }
        drop(shutdown);
        ora_debug!(job_name = %job_name, schedule = %schedule_repr, "registered cron job");
        Ok(CronHandle {
            control,
            detached: false,
        })
    }

    /// Schedules a one-shot delayed future, modeled after `setTimeout`.
    ///
    /// The returned [`DelayHandle`] is `#[must_use]`; dropping it before the delay elapses
    /// cancels the task. To let the task outlive the handle, call [`DelayHandle::detach`].
    pub fn schedule_after<F>(
        &self,
        delay: Duration,
        future: F,
    ) -> Result<DelayHandle, SchedulerError>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let mut shutdown = self
            .inner
            .is_shutdown
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *shutdown {
            return Err(SchedulerError::ShuttingDown);
        }
        let control = Arc::new(DelayControl::new());
        let queued = self.inner.command_tx.send(SchedulerCommand::ScheduleAfter {
            delay,
            future: Box::pin(future),
            control: Arc::clone(&control),
        });
        if queued.is_err() {
            *shutdown = true;
            return Err(SchedulerError::ShuttingDown);
        }
        drop(shutdown);
        ora_debug!(
            delay_ms = delay.as_millis() as u64,
            "scheduled delayed task"
        );
        Ok(DelayHandle {
            control,
            detached: false,
        })
    }

    /// Begins shutdown: rejects new registrations, aborts pending and running tasks (including
    /// detached tasks, which still belong to the scheduler), and resolves once every scheduler-
    /// owned task has exited. Every clone waits for the same supervisor completion, so concurrent
    /// callers and callers whose first wait is cancelled retain the shutdown guarantee.
    pub async fn shutdown(&self) {
        let should_signal = {
            let mut shutdown = self
                .inner
                .is_shutdown
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if *shutdown {
                false
            } else {
                *shutdown = true;
                true
            }
        };
        if should_signal {
            // The supervisor also exits when every Sender is dropped, but sending Shutdown makes
            // an explicit shutdown prompt and deterministic.
            let _ = self.inner.command_tx.send(SchedulerCommand::Shutdown);
        }
        self.completion.wait().await;
    }
}

/// Owns the `JoinSet` of all spawned work and dispatches the registration queue.
///
/// Selects between incoming commands and natural task completion. `Shutdown` (or the receiver
/// returning `None`, which happens once every `Scheduler` clone is dropped) breaks the loop;
/// the drain phase then aborts any remaining task and joins each so the supervisor returns only
/// once no scheduler-owned task remains.
async fn run_supervisor(
    mut command_rx: mpsc::UnboundedReceiver<SchedulerCommand>,
    timezone: Tz,
    completion: Arc<ShutdownCompletion>,
) {
    let mut tasks: JoinSet<()> = JoinSet::new();
    loop {
        tokio::select! {
            cmd = command_rx.recv() => match cmd {
                Some(SchedulerCommand::ScheduleCron { schedule, job, control }) => {
                    tasks.spawn(run_cron_loop(*schedule, timezone, job, control));
                }
                Some(SchedulerCommand::ScheduleAfter { delay, future, control }) => {
                    tasks.spawn(run_delayed_future(delay, control, future));
                }
                Some(SchedulerCommand::Shutdown) | None => {
                    ora_info!("scheduler shutting down");
                    break;
                }
            },
            Some(_) = tasks.join_next() => {
                // A task completed naturally; discard its result and continue.
            },
        }
    }
    // Drain phase: shutdown aborts everything that is still alive (pending and running alike),
    // then joins each until the JoinSet is empty. The supervisor returns only once every
    // scheduler-owned task has exited, satisfying the shutdown contract.
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    completion.complete();
}

/// Runs one cron job until its control is cancelled or the loop is aborted by shutdown.
///
/// Iterations never overlap because each tick is awaited sequentially within this task. After
/// each iteration the next fire time is computed strictly after "now" via
/// [`Schedule::after`], so ticks missed while the previous iteration was running are not
/// retried - the cron resumes at the next future tick.
async fn run_cron_loop(
    schedule: Schedule,
    timezone: Tz,
    job: Box<dyn Job>,
    control: Arc<CronControl>,
) {
    let job_name = job.name().to_owned();
    let job_schedule = job.schedule().to_owned();
    loop {
        if control.is_stopped() {
            return;
        }
        // Start from local time, then express the same instant in the explicitly configured
        // timezone so cron expressions never silently fall back to the machine's timezone.
        let now = Local::now().with_timezone(&timezone);
        let delay = match schedule.after(&now).next() {
            Some(next) => (next - now).to_std().unwrap_or(Duration::ZERO),
            None => {
                ora_warn!(job_name = %job_name, schedule = %job_schedule, "cron yields no future tick; stopping");
                return;
            }
        };
        tokio::select! {
            _ = sleep(delay) => {
                if control.is_stopped() {
                    ora_info!(job_name = %job_name, "cron stopped before run");
                    return;
                }
                ora_debug!(job_name = %job_name, schedule = %job_schedule, "running cron tick");
                job.run().await;
                ora_debug!(job_name = %job_name, "cron tick complete");
            }
            _ = control.token().cancelled() => {
                ora_info!(job_name = %job_name, "cron stopped by handle");
                return;
            }
        }
    }
}

/// Runs the delayed task: waits `delay`, then runs `future` unless cancelled before the delay
/// elapsed.
///
/// Once the future has started, handle cancellation does not interrupt it; only scheduler
/// shutdown (which aborts the `JoinSet`) is allowed to drop a running future. The
/// `try_cancel` in the cancel arm is defensive: it is a no-op when the handle has already
/// performed the transition, but keeps the state machine total if any other caller ever fires
/// the token.
async fn run_delayed_future(delay: Duration, control: Arc<DelayControl>, future: OwnedFuture) {
    tokio::select! {
        _ = sleep(delay) => {
            if control.claim_running() {
                future.await;
                control.mark_done();
            }
        }
        _ = control.token().cancelled() => {
            let _ = control.try_cancel();
        }
    }
}
