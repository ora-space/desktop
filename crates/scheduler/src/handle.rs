//! Shared control blocks and `#[must_use]` handles for the [`crate::Scheduler`].
//!
//! Both one-shot delayed tasks and cron jobs expose a handle whose drop cancels pending work
//! unless [`DelayHandle::detach`] / [`CronHandle::detach`] has been called. The handle carries
//! the [`CancellationToken`] shared with the spawned task, so cancellation is cooperative: a
//! cron loop only suppresses subsequent ticks, and cancelling a delayed future that has already
//! started does not interrupt it. Scheduler shutdown uses [`tokio::task::JoinSet::abort_all`]
//! (owned by the supervisor), which is the one path that may forcefully drop a running future.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use tokio_util::sync::CancellationToken;

/// Outcome of explicitly cancelling a one-shot delayed task via [`DelayHandle::cancel`].
///
/// `cancel` is cooperative: a future that has already started is not interrupted, and a future
/// that has already run to completion (or was cancelled earlier) is reported as already done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    /// The task was waiting in its delay and has now been prevented from running.
    Cancelled,
    /// The task already started; the running future continues to completion.
    AlreadyRunning,
    /// The task already completed (or was previously cancelled); this call was a no-op.
    AlreadyDone,
}

/// Outcome of explicitly cancelling a recurring cron task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CronCancelOutcome {
    /// The cron loop was active and has now been stopped.
    Cancelled,
    /// The cron loop had already been stopped or completed.
    AlreadyStopped,
}

/// Internal state shared between [`DelayHandle`] and the spawned delayed task.
///
/// The state machine is monotonic per the documented transitions, enforced with
/// `compare_exchange` so handle cancellation and the task's "delay elapsed" claim race
/// deterministically: the first writer wins, the second observes the new state.
///
/// - `PENDING` (0): the task is sleeping through the requested delay.
/// - `RUNNING` (1): the delay elapsed; the user future is in progress.
/// - `DONE` (2): the user future returned.
/// - `CANCELLED` (3): the handle cancelled the task before its delay elapsed.
pub(crate) struct DelayControl {
    state: AtomicU8,
    token: CancellationToken,
}

impl DelayControl {
    const PENDING: u8 = 0;
    const RUNNING: u8 = 1;
    const DONE: u8 = 2;
    const CANCELLED: u8 = 3;

    /// Creates the pending state that a delayed task must pass through before it can run.
    pub(crate) fn new() -> Self {
        Self {
            state: AtomicU8::new(Self::PENDING),
            token: CancellationToken::new(),
        }
    }

    /// The cancellation token shared with the spawned task.
    pub(crate) fn token(&self) -> &CancellationToken {
        &self.token
    }

    /// Claims the `Running` state from `Pending`. Returns whether the claim succeeded.
    ///
    /// A failing claim means the handle cancelled the task concurrently, so the future must not
    /// run even though the delay elapsed. This is the only transition into `Running`.
    pub(crate) fn claim_running(&self) -> bool {
        self.state
            .compare_exchange(
                Self::PENDING,
                Self::RUNNING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    /// Marks the task `Done` once the user future has returned.
    pub(crate) fn mark_done(&self) {
        self.state.store(Self::DONE, Ordering::Release);
    }

    /// Attempts to transition `Pending` -> `Cancelled` and fire the cancellation token.
    /// Returns whether this caller performed the transition (false means another caller raced it).
    pub(crate) fn try_cancel(&self) -> bool {
        let first = self
            .state
            .compare_exchange(
                Self::PENDING,
                Self::CANCELLED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok();
        if first {
            self.token.cancel();
        }
        first
    }

    /// Reads the current state after a cancellation race has been resolved.
    fn state(&self) -> u8 {
        self.state.load(Ordering::Acquire)
    }
}

/// Internal state shared between [`CronHandle`] and the spawned cron loop.
///
/// Cron jobs are recurring, so the state is a single stopped-bit: cancellation just suppresses
/// subsequent ticks; a tick already started runs to completion. Scheduler shutdown uses
/// `JoinSet::abort_all` rather than this token.
pub(crate) struct CronControl {
    stopped: AtomicBool,
    token: CancellationToken,
}

impl CronControl {
    /// Creates an active cron control block whose loop may observe later cancellation.
    pub(crate) fn new() -> Self {
        Self {
            stopped: AtomicBool::new(false),
            token: CancellationToken::new(),
        }
    }

    /// Returns the cancellation signal used while the cron loop waits for a tick.
    pub(crate) fn token(&self) -> &CancellationToken {
        &self.token
    }

    /// Reads whether this cron loop must stop before starting another tick.
    pub(crate) fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    /// Cancels subsequent ticks and fires the cancellation token watched by the loop.
    /// Returns whether this call performed the stop (false means it was already stopped).
    pub(crate) fn cancel(&self) -> CronCancelOutcome {
        let was_active = self
            .stopped
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok();
        if was_active {
            self.token.cancel();
            CronCancelOutcome::Cancelled
        } else {
            CronCancelOutcome::AlreadyStopped
        }
    }
}

/// Handle for a one-shot delayed task created with [`crate::Scheduler::schedule_after`].
///
/// The handle is `#[must_use]`: dropping it before the delay elapses cancels the task. To let
/// the task outlive the handle, call [`DelayHandle::detach`].
///
/// Cancelling a task that has already started does not interrupt the running future; only the
/// scheduler's `shutdown` (which aborts its `JoinSet`) is allowed to drop a running future.
#[must_use = "dropping the handle cancels the pending task; call .detach() to let it run free"]
pub struct DelayHandle {
    pub(crate) control: Arc<DelayControl>,
    pub(crate) detached: bool,
}

impl DelayHandle {
    /// Cancels the pending task without interrupting a future that has already started.
    pub fn cancel(&mut self) -> CancelOutcome {
        if self.control.try_cancel() {
            return CancelOutcome::Cancelled;
        }
        match self.control.state() {
            DelayControl::RUNNING => CancelOutcome::AlreadyRunning,
            _ => CancelOutcome::AlreadyDone,
        }
    }

    /// Lets the task outlive this handle. After `detach`, dropping the handle will not cancel it.
    pub fn detach(mut self) {
        self.detached = true;
    }
}

impl Drop for DelayHandle {
    fn drop(&mut self) {
        if !self.detached {
            let _ = self.cancel();
        }
    }
}

impl fmt::Debug for DelayHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DelayHandle")
            .field("detached", &self.detached)
            .finish_non_exhaustive()
    }
}

/// Handle for a dynamically registered cron job created with [`crate::Scheduler::schedule_cron`].
///
/// The handle is `#[must_use]`: dropping it stops subsequent ticks. To keep the cron running
/// after the handle is dropped, call [`CronHandle::detach`]. Cancelling never interrupts a tick
/// that has already started; the loop simply exits after the current iteration completes.
#[must_use = "dropping the handle stops the cron loop; call .detach() to keep it running"]
pub struct CronHandle {
    pub(crate) control: Arc<CronControl>,
    pub(crate) detached: bool,
}

impl CronHandle {
    /// Stops subsequent ticks. A tick that has already started runs to completion.
    pub fn cancel(&mut self) -> CronCancelOutcome {
        self.control.cancel()
    }

    /// Lets the cron job outlive this handle. After `detach`, dropping the handle will not stop it.
    pub fn detach(mut self) {
        self.detached = true;
    }
}

impl Drop for CronHandle {
    fn drop(&mut self) {
        if !self.detached {
            let _ = self.cancel();
        }
    }
}

impl fmt::Debug for CronHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CronHandle")
            .field("detached", &self.detached)
            .finish_non_exhaustive()
    }
}
