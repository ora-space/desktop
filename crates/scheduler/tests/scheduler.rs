#![allow(clippy::expect_used)]

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use ora_scheduler::{BoxFuture, CancelOutcome, Job, Scheduler, SchedulerError};
use pretty_assertions::assert_eq;

/// Polls a future once so cancellation tests can stop after shutdown has been initiated but
/// before the supervisor has had a chance to publish completion.
fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    future.poll(&mut context)
}

/// Polls `predicate` until it returns `true` or `max_ms` elapses. Returns whether it satisfied.
async fn wait_for<F>(predicate: F, max_ms: u64) -> bool
where
    F: Fn() -> bool,
{
    let deadline = tokio::time::Instant::now() + Duration::from_millis(max_ms);
    loop {
        if predicate() {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

/// Increments an atomic on every tick. Shared by several tests.
struct Counter {
    count: Arc<AtomicUsize>,
    name: &'static str,
    schedule: &'static str,
    per_tick_sleep: Duration,
}

impl Job for Counter {
    fn name(&self) -> &str {
        self.name
    }
    fn schedule(&self) -> &str {
        self.schedule
    }
    fn run(&self) -> BoxFuture<'_> {
        let count = Arc::clone(&self.count);
        let sleep = self.per_tick_sleep;
        Box::pin(async move {
            count.fetch_add(1, Ordering::SeqCst);
            if sleep > Duration::ZERO {
                tokio::time::sleep(sleep).await;
            }
        })
    }
}

/// Marks a flag the instant the future starts, then sleeps before completing.
struct Notifier {
    started: Arc<AtomicBool>,
    completed: Arc<AtomicBool>,
    run_sleep: Duration,
}

impl Notifier {
    fn into_future(self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        let started = Arc::clone(&self.started);
        let completed = Arc::clone(&self.completed);
        let sleep = self.run_sleep;
        Box::pin(async move {
            started.store(true, Ordering::SeqCst);
            if sleep > Duration::ZERO {
                tokio::time::sleep(sleep).await;
            }
            completed.store(true, Ordering::SeqCst);
        })
    }
}

/// Verifies a delayed future cannot start before its configured deadline.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delayed_task_runs_after_delay() {
    let scheduler = Scheduler::new(chrono_tz::UTC);
    let delay = Duration::from_millis(20);
    let scheduled_at = tokio::time::Instant::now();
    let (fired, fired_at) = tokio::sync::oneshot::channel();
    let _handle = scheduler
        .schedule_after(delay, async move {
            let _ = fired.send(tokio::time::Instant::now());
        })
        .expect("scheduler remains active");
    let fired_at = tokio::time::timeout(Duration::from_millis(500), fired_at)
        .await
        .expect("delayed task should run")
        .expect("delayed task should retain its receiver");
    assert!(fired_at.duration_since(scheduled_at) >= delay);
    scheduler.shutdown().await;
}

/// Verifies explicit cancellation wins while a delayed future is still pending.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn explicit_cancel_before_start_prevents_run() {
    let scheduler = Scheduler::new(chrono_tz::UTC);
    let fired = Arc::new(AtomicUsize::new(0));
    let f = Arc::clone(&fired);
    let mut handle = scheduler
        .schedule_after(Duration::from_millis(500), async move {
            f.fetch_add(1, Ordering::SeqCst);
        })
        .expect("scheduler remains active");
    let outcome = handle.cancel();
    assert_eq!(outcome, CancelOutcome::Cancelled);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(fired.load(Ordering::SeqCst), 0);
    scheduler.shutdown().await;
}

/// Verifies handle ownership cancels pending work unless it is detached.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn drop_handle_before_start_prevents_run() {
    let scheduler = Scheduler::new(chrono_tz::UTC);
    let fired = Arc::new(AtomicUsize::new(0));
    let f = Arc::clone(&fired);
    {
        let _handle = scheduler
            .schedule_after(Duration::from_millis(500), async move {
                f.fetch_add(1, Ordering::SeqCst);
            })
            .expect("scheduler remains active");
    }
    // The handle is dropped immediately; the Drop impl cancels the pending task.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(fired.load(Ordering::SeqCst), 0);
    scheduler.shutdown().await;
}

/// Verifies detaching transfers delayed-task lifetime to the scheduler.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn detached_delayed_task_continues_to_run() {
    let scheduler = Scheduler::new(chrono_tz::UTC);
    let fired = Arc::new(AtomicUsize::new(0));
    let f = Arc::clone(&fired);
    let handle = scheduler
        .schedule_after(Duration::from_millis(20), async move {
            f.fetch_add(1, Ordering::SeqCst);
        })
        .expect("scheduler remains active");
    // Detaching transfers ownership to the scheduler; dropping the handle afterwards must not cancel.
    handle.detach();
    assert!(wait_for(|| fired.load(Ordering::SeqCst) == 1, 500).await);
    assert_eq!(fired.load(Ordering::SeqCst), 1);
    scheduler.shutdown().await;
}

/// Verifies per-handle cancellation never interrupts a future that already started.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_after_start_does_not_interrupt_running_future() {
    let scheduler = Scheduler::new(chrono_tz::UTC);
    let started = Arc::new(AtomicBool::new(false));
    let completed = Arc::new(AtomicBool::new(false));
    let notifier = Notifier {
        started: Arc::clone(&started),
        completed: Arc::clone(&completed),
        run_sleep: Duration::from_millis(200),
    };
    let mut handle = scheduler
        .schedule_after(Duration::from_millis(10), notifier.into_future())
        .expect("scheduler remains active");
    assert!(
        wait_for(|| started.load(Ordering::SeqCst), 300).await,
        "future must start before cancel is attempted"
    );
    let outcome = handle.cancel();
    assert_eq!(outcome, CancelOutcome::AlreadyRunning);
    assert!(
        wait_for(|| completed.load(Ordering::SeqCst), 500).await,
        "future must complete despite the late cancel"
    );
    scheduler.shutdown().await;
}

/// Verifies scheduler shutdown may abort work that per-handle cancellation must preserve.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutdown_aborts_running_task() {
    let scheduler = Scheduler::new(chrono_tz::UTC);
    let started = Arc::new(AtomicBool::new(false));
    let completed = Arc::new(AtomicBool::new(false));
    let notifier = Notifier {
        started: Arc::clone(&started),
        completed: Arc::clone(&completed),
        run_sleep: Duration::from_secs(60),
    };
    let handle = scheduler
        .schedule_after(Duration::from_millis(10), notifier.into_future())
        .expect("scheduler remains active");
    handle.detach();
    assert!(
        wait_for(|| started.load(Ordering::SeqCst), 300).await,
        "future must start before shutdown is requested"
    );
    scheduler.shutdown().await;
    // shutdown aborted the long sleep before completion could fire the completed flag.
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(!completed.load(Ordering::SeqCst));
}

/// Verifies registration closes atomically once shutdown begins.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutdown_rejects_new_registrations() {
    let scheduler = Scheduler::new(chrono_tz::UTC);
    scheduler.shutdown().await;
    let error = scheduler
        .schedule_after(Duration::ZERO, async {})
        .expect_err("shutdown must reject new registrations");
    assert!(matches!(error, SchedulerError::ShuttingDown));
}

/// Verifies eager cron validation cannot leave partially registered work behind.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invalid_cron_expression_schedules_nothing() {
    let scheduler = Scheduler::new(chrono_tz::UTC);
    let counter = Arc::new(AtomicUsize::new(0));
    struct Invalid {
        name: &'static str,
        counter: Arc<AtomicUsize>,
    }
    impl Job for Invalid {
        fn name(&self) -> &str {
            self.name
        }
        fn schedule(&self) -> &str {
            "garbage expression"
        }
        fn run(&self) -> BoxFuture<'_> {
            let c = Arc::clone(&self.counter);
            Box::pin(async move {
                c.fetch_add(1, Ordering::SeqCst);
            })
        }
    }
    let error = scheduler
        .schedule_cron(Invalid {
            name: "invalid",
            counter: Arc::clone(&counter),
        })
        .expect_err("invalid cron must be rejected");
    assert!(matches!(
        error,
        SchedulerError::InvalidCronExpression { .. }
    ));
    assert_eq!(counter.load(Ordering::SeqCst), 0);
    scheduler.shutdown().await;
}

/// Verifies cron jobs can be added after the scheduler service is already running.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cron_job_registered_after_construction_fires() {
    let scheduler = Scheduler::new(chrono_tz::UTC);
    let count = Arc::new(AtomicUsize::new(0));
    let mut cron = scheduler
        .schedule_cron(Counter {
            count: Arc::clone(&count),
            name: "every-second",
            schedule: "* * * * * *",
            per_tick_sleep: Duration::ZERO,
        })
        .expect("scheduler remains active");
    // Schedule was registered after `Scheduler::new` returned; first tick fires within ~1s.
    assert!(
        wait_for(|| count.load(Ordering::SeqCst) >= 1, 1500).await,
        "first cron tick must fire after registration"
    );
    assert_eq!(cron.cancel(), ora_scheduler::CronCancelOutcome::Cancelled);
    let after_cancel = count.load(Ordering::SeqCst);
    // No extra ticks should arrive once cancellation has been observed; allow a small window to detect drift.
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let after_window = count.load(Ordering::SeqCst);
    assert_eq!(
        after_window, after_cancel,
        "cancelled cron must not fire new ticks"
    );
    scheduler.shutdown().await;
}

/// Verifies slow cron iterations stay sequential and do not replay missed ticks.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cron_iterations_never_overlap_and_missed_ticks_are_skipped() {
    // Each tick sleeps 2s. With a 1-second cron half of the ticks would overlap if they ran
    // concurrently; the contract requires sequential iteration so they cannot, and any tick that
    // would have fired while the previous iteration was still running is skipped rather than retried.
    // Empirically two ticks span ~4s: ~1s wait + 2s long sleep, then ~1s wait + 2s sleep.
    let scheduler = Scheduler::new(chrono_tz::UTC);
    let count = Arc::new(AtomicUsize::new(0));
    let mut cron = scheduler
        .schedule_cron(Counter {
            count: Arc::clone(&count),
            name: "long-running",
            schedule: "* * * * * *",
            per_tick_sleep: Duration::from_secs(2),
        })
        .expect("scheduler remains active");

    // Allow ~5s wall clock: should comfortably fit exactly two completed ticks when overlap is
    // suppressed and missed ticks are skipped.
    tokio::time::sleep(Duration::from_millis(5_000)).await;
    let observed = count.load(Ordering::SeqCst);
    assert!(
        (1..=3).contains(&observed),
        "concurrent execution would exceed the 3-tick ceiling; observed {observed}"
    );
    assert_eq!(cron.cancel(), ora_scheduler::CronCancelOutcome::Cancelled);
    scheduler.shutdown().await;
}

/// Verifies detached cron work remains scheduler-owned until shutdown.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn detached_cron_continues_until_shutdown() {
    let scheduler = Scheduler::new(chrono_tz::UTC);
    let count = Arc::new(AtomicUsize::new(0));
    let cron = scheduler
        .schedule_cron(Counter {
            count: Arc::clone(&count),
            name: "detached",
            schedule: "* * * * * *",
            per_tick_sleep: Duration::ZERO,
        })
        .expect("scheduler remains active");
    cron.detach();
    // The detached cron keeps running after the handle is gone.
    assert!(wait_for(|| count.load(Ordering::SeqCst) >= 1, 1500).await);
    // Shutdown terminates the detached cron regardless of handle ownership.
    scheduler.shutdown().await;
    let after_shutdown = count.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(
        count.load(Ordering::SeqCst),
        after_shutdown,
        "shutdown must stop the detached cron"
    );
}

/// Verifies clones register with and shut down one shared supervisor.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scheduler_clone_shares_supervisor_and_shuts_down_once() {
    let scheduler = Scheduler::new(chrono_tz::UTC);
    let clone = scheduler.clone();
    let fired = Arc::new(AtomicUsize::new(0));
    let f = Arc::clone(&fired);
    let _handle = clone.schedule_after(Duration::from_millis(20), async move {
        f.fetch_add(1, Ordering::SeqCst);
    });
    assert!(wait_for(|| fired.load(Ordering::SeqCst) == 1, 500).await);
    // Either clone's shutdown drains the shared supervisor.
    clone.shutdown().await;
    // Post-shutdown the original clone rejects new registrations.
    let error = scheduler
        .schedule_after(Duration::ZERO, async {})
        .expect_err("shutdown must reject new registrations");
    assert!(matches!(error, SchedulerError::ShuttingDown));
}

/// Verifies concurrent shutdown callers all observe supervisor completion.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_shutdown_callers_wait_for_shared_completion() {
    let scheduler = Scheduler::new(chrono_tz::UTC);
    let first = scheduler.clone();
    let second = scheduler.clone();

    tokio::join!(first.shutdown(), second.shutdown());

    let error = scheduler
        .schedule_after(Duration::ZERO, async {})
        .expect_err("both shutdown callers must observe the completed shutdown");
    assert!(matches!(error, SchedulerError::ShuttingDown));
}

/// Verifies cancelling one shutdown waiter cannot detach the shared completion owner.
///
/// A current-thread runtime keeps the first manual poll deterministic: it can initiate shutdown
/// without letting the spawned supervisor publish completion before the waiter is dropped.
#[tokio::test(flavor = "current_thread")]
async fn cancelled_shutdown_wait_does_not_detach_completion_owner() {
    let scheduler = Scheduler::new(chrono_tz::UTC);
    let first_scheduler = scheduler.clone();
    let mut first = Box::pin(first_scheduler.shutdown());
    assert!(matches!(poll_once(first.as_mut()), Poll::Pending));
    drop(first);

    let second_scheduler = scheduler.clone();
    second_scheduler.shutdown().await;
    let error = scheduler
        .schedule_after(Duration::ZERO, async {})
        .expect_err("the replacement waiter must observe completed shutdown");
    assert!(matches!(error, SchedulerError::ShuttingDown));
}

/// Verifies delayed-task cancellation reports its monotonic terminal state.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_idle_delay_handle_returns_cancelled_then_already_done() {
    let scheduler = Scheduler::new(chrono_tz::UTC);
    let mut handle = scheduler
        .schedule_after(Duration::from_millis(500), async {})
        .expect("scheduler remains active");
    assert_eq!(handle.cancel(), CancelOutcome::Cancelled);
    // A second cancel observes the already-cancelled state.
    assert_eq!(handle.cancel(), CancelOutcome::AlreadyDone);
    scheduler.shutdown().await;
}

/// Verifies a completed delayed future cannot be reported as newly cancelled.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn completed_delay_handle_reports_already_done() {
    let scheduler = Scheduler::new(chrono_tz::UTC);
    let completed = Arc::new(AtomicBool::new(false));
    let c = Arc::clone(&completed);
    let mut handle = scheduler
        .schedule_after(Duration::from_millis(10), async move {
            c.store(true, Ordering::SeqCst);
        })
        .expect("scheduler remains active");
    assert!(wait_for(|| completed.load(Ordering::SeqCst), 500).await);
    // The worker may still hold RUNNING briefly after the future body returns;
    // poll cancel until control settles on the terminal state.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    let mut outcome = handle.cancel();
    while outcome == CancelOutcome::AlreadyRunning && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(2)).await;
        outcome = handle.cancel();
    }
    assert_eq!(outcome, CancelOutcome::AlreadyDone);
    scheduler.shutdown().await;
}
