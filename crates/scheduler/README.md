# ora-scheduler

`ora-scheduler` is an in-process scheduling service for the Ora backend. It drives recurring cron jobs and one-shot delayed futures and owns their lifecycle until [`Scheduler::shutdown`] is awaited. When constructed inside Tokio it uses the current runtime; synchronous composition roots receive an internally owned current-thread runtime.

## Public model

- [`Job`](src/job.rs) declares a stable name, a cron expression, and the asynchronous work performed on each tick.
- [`Scheduler`] is created with an explicit IANA timezone and immediately starts a background supervisor; it is `Clone` and accepts registrations dynamically until `shutdown`.
- [`Scheduler::schedule_cron`] parses the cron expression eagerly via `cron::Schedule::from_str` and returns a [`CronHandle`]; an invalid expression returns [`SchedulerError::InvalidCronExpression`] and spawns nothing.
- [`Scheduler::schedule_after`] is a `setTimeout`-style one-shot delayed future; the returned [`DelayHandle`] is `#[must_use]` and cancels the pending task when dropped unless [`DelayHandle::detach`] is called.
- [`CancelOutcome`] is the named result of cancelling a delayed task: `Cancelled`, `AlreadyRunning`, or `AlreadyDone`.
- [`SchedulerError`] reports invalid cron expressions and rejected registrations after shutdown.

The supervisor consumes a Tokio `mpsc` channel of registrations and owns a `JoinSet` that hosts every spawned task. Tokio's `JoinHandle` and `CancellationToken` are deliberately absent from the public surface; callers express ownership solely through `CronHandle` and `DelayHandle`.

## One-shot delayed execution

`schedule_after(delay, future)` waits the configured duration before running `future` once. Cancellation is cooperative and follows three rules:

- A handle dropped (or explicitly cancelled) before the delay elapses prevents the future from ever starting; the outcome is `CancelOutcome::Cancelled`.
- Once the future has started, dropping or `cancel`-ing the handle returns `CancelOutcome::AlreadyRunning` and leaves the future running; it is not interrupted.
- After the future has returned, `cancel` reports `CancelOutcome::AlreadyDone`.

`DelayHandle::detach` consumes the handle and detaches the task: dropping the handle afterwards no longer cancels it, but the task still belongs to the scheduler and is aborted on `shutdown`.

## Dynamic cron tasks

`Scheduler` accepts cron registrations at any time after construction. Each registration is parsed independently: an invalid expression is rejected atomically and spawns nothing. The returned `CronHandle` is `#[must_use]`; dropping it stops subsequent ticks (unless `detach` was called), but a tick that has already started runs to completion before the loop exits.

Iterations of one cron job never overlap because each tick is awaited sequentially within the task. After each iteration the next fire time is computed strictly after "now" via `Schedule::after`, so ticks missed while the previous iteration was still running are skipped rather than retried.

## Shutdown

`shutdown` begins immediately: pending registrations are rejected with `SchedulerError::ShuttingDown`, every scheduler-owned task is aborted (including running ones, and including detached tasks, which still belong to the scheduler), and the call resolves only after the `JoinSet` is empty. Shutdown may be invoked from any `Scheduler` clone. When every `Scheduler` clone is dropped without an explicit `shutdown`, the supervisor detects the closed registration channel, drains its tasks the same way, and exits autonomously.

## Lifecycle boundaries

The crate owns in-process timing and cooperative shutdown only. It does not persist schedules, provide distributed coordination, retry failed work, enforce execution timeouts, or record job outcomes beyond structured Ora logging. Job implementations and their composition roots own those policies and any required dependencies.
