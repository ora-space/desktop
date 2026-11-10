//! Reproducible samples at the exact durable-intent/first-dispatch crash boundary.
use super::*;
use ora_process_protocol::*;
use pretty_assertions::assert_eq;
use std::os::unix::fs::PermissionsExt;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Concurrent fork/exec fixtures can briefly inherit the old lock; retry only actual contention.
fn recover(root: &std::path::Path) -> Result<HostState, ProcessStateError> {
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 10);
    loop {
        match HostState::recover(root) {
            Err(ProcessStateError::Io(error))
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(/*millis*/ 5));
            }
            result => return result,
        }
    }
}

/// One contradictory Run cannot discard its peers or terminate coordination, even after recovery.
#[tokio::test]
async fn contradictory_nonlaunch_preserves_facts_and_keeps_tick_serving() -> TestResult {
    let home = std::env::var_os("HOME").ok_or("fixture HOME missing")?;
    let directory = tempfile::Builder::new()
        .prefix("nc")
        .permissions(std::fs::Permissions::from_mode(/*mode*/ 0o700))
        .tempdir_in(home)?;
    let root = directory.path().join("s");
    let mut host =
        HostCoordinator::new(HostState::create(&root)?, directory.path().join("unused"))?;
    let scope = ScopeId::new();
    let other_scope = ScopeId::new();
    let run = RunId::new();
    let peer = RunId::new();
    let other_run = RunId::new();
    for (scope, run) in [(scope, run), (scope, peer), (other_scope, other_run)] {
        host.create_scope(scope)?;
        host.start(HostRunIntent {
            scope,
            run,
            spec: RunSpec::new(
                "/usr/bin/true",
                directory.path(),
                DescendantPolicy::WaitForAll,
            ),
            host_disconnect: GuardianHostDisconnect::KeepRunning,
        })?;
    }
    let original = RunSnapshot {
        id: run,
        launch: LaunchFact::Started,
        direct: DirectProcessState::Running,
        cleanup: CleanupState::Pending,
    };
    host.state.observe_run(&original)?;
    host.close(scope)?;
    host.close(other_scope)?;
    let closed = ScopeState::Closed(CleanupEvidence::BestEffortComplete);
    let peer_snapshot = RunSnapshot {
        id: peer,
        launch: LaunchFact::NotStarted("cancelled before guardian acceptance".into()),
        direct: DirectProcessState::NotStarted,
        cleanup: CleanupState::Complete(CleanupEvidence::BestEffortComplete),
    };
    let other_snapshot = RunSnapshot {
        id: other_run,
        ..peer_snapshot.clone()
    };
    for recovered in [false, true] {
        if recovered {
            drop(host);
            host = HostCoordinator::new(recover(&root)?, directory.path().join("unused"))?;
        }
        // Inject only the typed worker result; tick and its durable journal remain production code.
        for (scope, runs) in [
            (
                scope,
                vec![
                    RunSnapshot {
                        id: run,
                        ..peer_snapshot.clone()
                    },
                    peer_snapshot.clone(),
                ],
            ),
            (other_scope, vec![other_snapshot.clone()]),
        ] {
            host.active.insert(scope);
            host.jobs.spawn(async move {
                (
                    scope,
                    worker::Observation {
                        scope: Some(closed),
                        runs,
                        status: HostCoordination::Observing,
                    },
                )
            });
        }
        tokio::time::timeout(Duration::from_secs(/*secs*/ 2), async {
            while host.active.contains(&scope) || host.active.contains(&other_scope) {
                tokio::task::yield_now().await;
                let result = host.tick();
                assert!(
                    result.is_ok(),
                    "one contradictory Run terminated host tick: {result:?}"
                );
            }
        })
        .await?;
        assert_eq!(host.query_run(run)?.last_observed, Some(original.clone()));
        assert_eq!(
            host.query_run(peer)?.last_observed,
            Some(peer_snapshot.clone())
        );
        assert_eq!(
            host.query_run(other_run)?.last_observed,
            Some(other_snapshot.clone())
        );
        assert_eq!(host.query_scope(scope)?.last_observed, Some(closed));
        assert_eq!(host.query_scope(other_scope)?.last_observed, Some(closed));
        assert_eq!(
            host.query_scope(scope)?.coordination,
            HostCoordination::Rejected(GuardianRunRejection::ConflictingRun)
        );
        assert_eq!(
            host.query_scope(other_scope)?.coordination,
            HostCoordination::Observing
        );
        assert!(
            !host.state.scope_settled(scope)?,
            "contradiction manufactured terminal evidence"
        );
        assert!(!host.progress[&scope].settled);
        assert!(host.progress[&scope].retry_at > Instant::now());
        let fresh = ScopeId::new();
        host.create_scope(fresh)?;
        host.close(fresh)?;
        assert!(host.query_scope(fresh)?.close_requested);
        // Leave this scope for the next tick to prove that serving continues after the rejection.
        host.tick()?;
    }
    Ok(())
}

/// A recovered host reports a positive non-launch fact only for a Run it actually accepted.
#[tokio::test]
async fn restart_before_first_dispatch_distinguishes_recorded_and_unknown_runs() -> TestResult {
    let home = std::env::var_os("HOME").ok_or("fixture HOME missing")?;
    let directory = tempfile::Builder::new()
        .prefix("nw")
        .permissions(std::fs::Permissions::from_mode(/*mode*/ 0o700))
        .tempdir_in(home)?;
    let mut recorded_nonlaunch = 0;
    let mut unknown_identity = 0;
    // Equal sampling is intentional injection, not an estimate of production frequency.
    for sample in 0..20 {
        for registered in [false, true] {
            let root = directory
                .path()
                .join(format!("{sample}-{}", usize::from(registered)));
            let scope = ScopeId::new();
            let intent = HostRunIntent {
                scope,
                run: RunId::new(),
                spec: RunSpec::new(
                    "/usr/bin/true",
                    directory.path(),
                    DescendantPolicy::WaitForAll,
                ),
                host_disconnect: GuardianHostDisconnect::KeepRunning,
            };
            let state = HostState::create(&root)?;
            let mut host = HostCoordinator::new(state, directory.path().join("unused"))?;
            host.create_scope(scope)?;
            if registered {
                host.start(intent.clone())?;
            }
            // Drop before tick: the journal is real but no guardian/Start can have been dispatched.
            let old_binding = host.binding();
            drop(host);
            let mut host = HostCoordinator::new(recover(&root)?, directory.path().join("unused"))?;
            assert!(host.binding().epoch > old_binding.epoch);
            host.close(scope)?;
            let deadline = Instant::now() + Duration::from_secs(/*secs*/ 2);
            loop {
                host.tick()?;
                if matches!(
                    host.query_scope(scope)?.last_observed,
                    Some(ScopeState::Closed(_))
                ) {
                    break;
                }
                assert!(Instant::now() < deadline, "recovered Scope did not close");
                tokio::time::sleep(Duration::from_millis(/*millis*/ 5)).await;
            }
            if registered {
                assert_eq!(
                    host.query_run(intent.run)?.last_observed,
                    Some(RunSnapshot {
                        id: intent.run,
                        launch: LaunchFact::NotStarted(
                            "cancelled before guardian acceptance".into()
                        ),
                        direct: DirectProcessState::NotStarted,
                        cleanup: CleanupState::Complete(CleanupEvidence::BestEffortComplete),
                    })
                );
                assert!(
                    host.start(HostRunIntent {
                        run: RunId::new(),
                        ..intent
                    })
                    .is_err(),
                    "closed recovered Scope admitted another Start"
                );
                recorded_nonlaunch += 1;
            } else {
                assert!(matches!(
                    host.query_run(intent.run),
                    Err(ProcessStateError::Rejected("unknown host Run"))
                ));
                unknown_identity += 1;
            }
        }
    }
    assert_eq!((recorded_nonlaunch, unknown_identity), (20, 20));
    println!(
        "host restart window: registered-before-dispatch NotStarted={recorded_nonlaunch}/20; unregistered UnknownIdentity={unknown_identity}/20; controlled samples only"
    );
    Ok(())
}
