//! Behavior of the Cloud persistence adapter's coordination loop against an in-memory Cloud that
//! serves the real contract: signal-driven claiming, fallback and reopening, drains, stale
//! eligibility and shutdown.
//!
//! Spec: specs/test-cases/controller/api-boundary/watch-signal-claiming.md
#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used)]

#[path = "support/fake_cloud.rs"]
mod fake_cloud;

use fake_cloud::{Call, FakeCloud, WatchPolicy};
use ora_controller::*;
use ora_node_protocol::*;
use pretty_assertions::assert_eq;
use std::{fs, future::Future, os::unix::fs::PermissionsExt, time::Duration};
use tokio::{sync::oneshot, task::JoinHandle};

/// The first lease the fake grants; every call below runs under it unless a test drops it.
const EPOCH: i64 = 1;
/// An interval no test outlives: claims can only come from the stream or its establishment.
const NEVER: u64 = 3_600_000;
/// A short fallback interval so tests observe several claim ticks quickly.
const FAST: u64 = 50;

/// A Controller running in cloud persistence mode against the fake.
struct Controller {
    stop: oneshot::Sender<()>,
    running: JoinHandle<std::io::Result<()>>,
}

impl Controller {
    /// Stops the runtime the way the executable does and waits for it to finish.
    async fn stop(self) {
        self.stop.send(()).unwrap();
        self.running.await.unwrap().unwrap();
    }
}

/// Runs `test` with a fake Cloud and a Controller claiming every `claim_interval_ms` without a
/// stream. The Node endpoint does not exist, so the session only retries its connection and never
/// touches the store; every recorded call comes from the coordination loop.
fn scenario<F, Fut>(claim_interval_ms: u64, test: F)
where
    F: FnOnce(FakeCloud, Box<dyn FnOnce(&str) -> Controller>) -> Fut,
    Fut: Future<Output = ()>,
{
    ora_logging::with_trace_logging(|| {
        let root = tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(/*mode*/ 0o700))
            .tempdir_in(std::env::var_os("HOME").unwrap())
            .unwrap();
        let home = root.path().join("controller");
        let socket = root.path().join("node").join("control.sock");
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let cloud = FakeCloud::new();
                let served = cloud.serve().await;
                let endpoint = served.endpoint.clone();
                let start = Box::new(move |controller_id: &str| {
                    let config = RuntimeConfig {
                        home_directory: home,
                        persistence: Persistence::Cloud {
                            endpoint,
                            claim_interval_ms,
                            substrate: None,
                        },
                        protected_state_directories: Vec::new(),
                        controller_id: ControllerId::new(controller_id),
                        nodes: vec![NodeTarget {
                            node_id: NodeId::new("node"),
                            endpoint: NodeEndpoint::Ipc { path: socket },
                        }],
                        session: SessionConfig {
                            io_timeout_ms: 100,
                            query_interval_ms: 10,
                        },
                        reconnect_ms: 1_000,
                        timezone: "Asia/Shanghai".into(),
                    };
                    let runtime = ControllerRuntime::<CloudStore>::open(config).unwrap();
                    let (stop, stopped) = oneshot::channel::<()>();
                    let running = tokio::spawn(async move {
                        runtime
                            .run(async {
                                let _ = stopped.await;
                            })
                            .await
                    });
                    Controller { stop, running }
                });
                test(cloud, start).await;
                drop(served);
            });
        // Cloud persistence never creates local state.
        assert!(!root.path().join("controller").exists());
    });
}

/// Counts recorded calls of one kind.
fn count(calls: &[Call], call: &Call) -> usize {
    calls.iter().filter(|recorded| *recorded == call).count()
}

/// Waits until the stream is established and the claim it triggers has run.
async fn established(cloud: &FakeCloud) {
    cloud
        .until(|state| state.watching() && state.calls.contains(&Call::ClaimWork { epoch: EPOCH }))
        .await;
}

/// A `WorkAvailable` signal registers the dispatch at once, with no periodic claim in between.
#[test]
fn signals_claim_without_waiting_for_the_claim_interval() {
    scenario(NEVER, |cloud, start| async move {
        let controller = start("owner");
        established(&cloud).await;
        let signalled = tokio::time::Instant::now();
        cloud.enqueue("op-1");
        cloud.signal_work("op-1");
        cloud.until(|state| state.dispatched() == ["op-1"]).await;
        assert!(signalled.elapsed() < Duration::from_secs(/*secs*/ 1));
        controller.stop().await;
        assert_eq!(
            cloud.calls(),
            vec![
                Call::AcquireLease,
                Call::Watch { epoch: EPOCH },
                Call::ClaimWork { epoch: EPOCH },
                Call::ClaimWork { epoch: EPOCH },
                Call::RecordDispatch {
                    epoch: EPOCH,
                    operation_id: "op-1".into()
                },
                Call::ClaimWork { epoch: EPOCH },
                Call::ReleaseLease { epoch: EPOCH },
            ]
        );
    });
}

/// Work Cloud accepted before the subscription existed has no signal coming; establishing the
/// stream claims it at once.
#[test]
fn work_accepted_before_the_stream_opens_is_claimed_on_establishment() {
    scenario(NEVER, |cloud, start| async move {
        cloud.enqueue("op-1");
        cloud.enqueue("op-2");
        let controller = start("owner");
        cloud
            .until(|state| state.dispatched() == ["op-1", "op-2"])
            .await;
        controller.stop().await;
        assert_eq!(
            cloud.calls(),
            vec![
                Call::AcquireLease,
                Call::Watch { epoch: EPOCH },
                Call::ClaimWork { epoch: EPOCH },
                Call::RecordDispatch {
                    epoch: EPOCH,
                    operation_id: "op-1".into()
                },
                Call::ClaimWork { epoch: EPOCH },
                Call::RecordDispatch {
                    epoch: EPOCH,
                    operation_id: "op-2".into()
                },
                Call::ClaimWork { epoch: EPOCH },
                Call::ReleaseLease { epoch: EPOCH },
            ]
        );
    });
}

/// A stream that breaks falls back to periodic claims, and a reopened stream claims on signals
/// again.
#[test]
fn a_broken_stream_falls_back_to_periodic_claims_and_reopens() {
    scenario(FAST, |cloud, start| async move {
        let controller = start("owner");
        established(&cloud).await;
        cloud.answer_watch(WatchPolicy::Unavailable);
        cloud.break_stream();
        // No signal is sent: only the periodic claim can find this work.
        cloud.enqueue("op-1");
        cloud.until(|state| state.dispatched() == ["op-1"]).await;
        cloud.answer_watch(WatchPolicy::Accept);
        cloud.until(|state| state.watching()).await;
        cloud.enqueue("op-2");
        cloud.signal_work("op-2");
        cloud
            .until(|state| state.dispatched() == ["op-1", "op-2"])
            .await;
        controller.stop().await;
    });
}

/// A drain stops claiming without releasing the lease; the stream reopening on a recovered Cloud
/// claims the work accepted meanwhile.
#[test]
fn a_drain_pauses_claims_until_a_new_stream_opens() {
    scenario(FAST, |cloud, start| async move {
        let controller = start("owner");
        established(&cloud).await;
        cloud.drain();
        cloud.enqueue("op-1");
        let drained_at = cloud.calls().len();
        // Every claim tick while drained tries to reopen, so reopen attempts count claim ticks:
        // three refused reopens prove at least three ticks passed without a claim.
        cloud
            .until(|state| count(&state.calls[drained_at..], &Call::Watch { epoch: EPOCH }) >= 3)
            .await;
        let during_drain = cloud.calls()[drained_at..].to_vec();
        assert_eq!(
            during_drain
                .iter()
                .filter(|call| **call != Call::Watch { epoch: EPOCH })
                .collect::<Vec<_>>(),
            Vec::<&Call>::new()
        );
        cloud.answer_watch(WatchPolicy::Accept);
        cloud.until(|state| state.dispatched() == ["op-1"]).await;
        controller.stop().await;
        assert_eq!(
            count(&cloud.calls(), &Call::ReleaseLease { epoch: EPOCH }),
            1
        );
    });
}

/// A serving Cloud that refuses the stream after a drain ends the drain: claiming degrades to the
/// periodic fallback instead of stopping for good.
#[test]
fn a_refused_stream_after_a_drain_resumes_periodic_claims() {
    scenario(FAST, |cloud, start| async move {
        let controller = start("owner");
        established(&cloud).await;
        cloud.drain();
        cloud.answer_watch(WatchPolicy::Unimplemented);
        cloud.enqueue("op-1");
        cloud.until(|state| state.dispatched() == ["op-1"]).await;
        controller.stop().await;
    });
}

/// A stale verdict on opening the stream drops the epoch: nothing is claimed or written until a
/// lease is acquired again, which only the next renewal would attempt.
#[test]
fn a_stale_stream_open_stops_claims_and_writes() {
    scenario(FAST, |cloud, start| async move {
        cloud.answer_watch(WatchPolicy::Stale);
        cloud.enqueue("op-1");
        let controller = start("owner");
        cloud
            .until(|state| state.calls.contains(&Call::Watch { epoch: EPOCH }))
            .await;
        // Without a lease the loop makes no call at all, so there is nothing to wait on; several
        // claim ticks must pass instead, well inside the ten-second renewal.
        tokio::time::sleep(Duration::from_millis(FAST * 6)).await;
        controller.stop().await;
        assert_eq!(
            cloud.calls(),
            vec![Call::AcquireLease, Call::Watch { epoch: EPOCH }]
        );
        assert_eq!(cloud.dispatched(), Vec::<String>::new());
    });
}

/// Shutdown cancels the stream and releases the lease under the epoch it was opened with.
#[test]
fn shutdown_cancels_the_stream_and_releases_the_lease() {
    scenario(NEVER, |cloud, start| async move {
        let controller = start("owner");
        established(&cloud).await;
        let cancelled = cloud.clone();
        let cancelled = tokio::spawn(async move { cancelled.watch_cancelled().await });
        controller.stop().await;
        cancelled.await.unwrap();
        assert_eq!(
            cloud.calls().last(),
            Some(&Call::ReleaseLease { epoch: EPOCH })
        );
    });
}

/// One signal registers a backlog larger than one batch: the loop continues after a full batch
/// instead of waiting for another trigger.
#[test]
fn one_signal_registers_a_backlog_beyond_one_batch() {
    scenario(NEVER, |cloud, start| async move {
        let controller = start("owner");
        established(&cloud).await;
        let operations: Vec<String> = (1..=17).map(|index| format!("op-{index:02}")).collect();
        for operation in &operations {
            cloud.enqueue(operation);
        }
        cloud.signal_work("op-01");
        cloud.until(|state| state.dispatched() == operations).await;
        controller.stop().await;
    });
}
