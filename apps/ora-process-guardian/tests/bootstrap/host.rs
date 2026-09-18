use super::*;
use ora_process_protocol::{
    CleanupEvidence, CleanupState, DescendantPolicy, DirectProcessState, ExitOutcome,
    GuardianHostDisconnect, HostCoordination, HostRunIntent, LaunchFact, RunId, RunSnapshot,
    RunSpec, ScopeState,
};
use ora_process_runtime::HostCoordinator;
use pretty_assertions::assert_eq;

/// Drives production coordination until the original Run has durable terminal cleanup evidence.
async fn complete(
    host: &mut HostCoordinator,
    run: RunId,
) -> Result<RunSnapshot, Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 10);
    loop {
        host.tick()?;
        if let Some(snapshot) = host.query_run(run)?.last_observed
            && matches!(snapshot.cleanup, CleanupState::Complete(_))
        {
            return Ok(snapshot);
        }
        assert!(
            Instant::now() < deadline,
            "host did not observe completed Run"
        );
        tokio::time::sleep(Duration::from_millis(/*millis*/ 5)).await;
    }
}

/// Close accepted before any dispatch cancels the original attempt without launching a guardian.
#[tokio::test]
async fn close_before_dispatch_never_executes_and_survives_recovery() -> TestResult {
    let fixture = Fixture::new()?;
    let root = fixture.directory.path().join("s");
    let mut host = HostCoordinator::new(HostState::create(&root)?, fixture.executable.clone());
    let scope = ScopeId::new();
    host.create_scope(scope)?;
    let mut spec = RunSpec::new(
        "/bin/sh",
        fixture.directory.path(),
        DescendantPolicy::WaitForAll,
    );
    spec.args = ["-c", "printf unwanted > marker"].map(Into::into).to_vec();
    let intent = HostRunIntent {
        scope,
        run: RunId::new(),
        spec,
        host_disconnect: GuardianHostDisconnect::KeepRunning,
    };
    host.start(intent.clone())?;
    host.close(scope)?;
    drop(host);
    let mut host = HostCoordinator::new(recover(&root).await?, fixture.executable.clone());
    let expected = RunSnapshot {
        id: intent.run,
        launch: LaunchFact::NotStarted("cancelled before guardian acceptance".into()),
        direct: DirectProcessState::NotStarted,
        cleanup: CleanupState::Complete(CleanupEvidence::BestEffortComplete),
    };
    assert_eq!(complete(&mut host, intent.run).await?, expected);
    assert_eq!(
        host.query_scope(scope)?.last_observed,
        Some(ScopeState::Closed(CleanupEvidence::BestEffortComplete))
    );
    assert_eq!(host.start(intent.clone())?.last_observed, Some(expected));
    assert!(!root.join("scopes").join(scope.to_string()).exists());
    assert!(!fixture.directory.path().join("marker").exists());
    Ok(())
}

/// Automatic dispatch recovers the original attempt and preserves facts after guardian loss.
#[tokio::test]
async fn host_projection_and_replay_do_not_repeat_side_effects() -> TestResult {
    let fixture = Fixture::new()?;
    let root = fixture.directory.path().join("s");
    let mut host = HostCoordinator::new(HostState::create(&root)?, fixture.executable.clone());
    let scope = ScopeId::new();
    host.create_scope(scope)?;
    let mut spec = RunSpec::new(
        "/bin/sh",
        fixture.directory.path(),
        DescendantPolicy::WaitForAll,
    );
    spec.args = ["-c", "printf x >> count; exit 7"].map(Into::into).to_vec();
    let intent = HostRunIntent {
        scope,
        run: RunId::new(),
        spec,
        host_disconnect: GuardianHostDisconnect::KeepRunning,
    };
    host.start(intent.clone())?;
    let expected = RunSnapshot {
        id: intent.run,
        launch: LaunchFact::Started,
        direct: DirectProcessState::Exited(ExitOutcome::Code(7)),
        cleanup: CleanupState::Complete(CleanupEvidence::BestEffortComplete),
    };
    assert_eq!(complete(&mut host, intent.run).await?, expected);
    drop(host);
    let state = recover(&root).await?;
    let access = state.guardian_access(scope)?.ok_or("missing access")?;
    let pid = guardian_pid(&access).await?;
    let mut host = HostCoordinator::new(state, fixture.executable.clone());
    assert_eq!(
        host.query_run(intent.run)?.coordination,
        HostCoordination::Pending
    );
    assert_eq!(
        host.start(intent.clone())?.last_observed,
        Some(expected.clone())
    );
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 10);
    while host.query_run(intent.run)?.coordination != HostCoordination::Observing {
        host.tick()?;
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(/*millis*/ 5)).await;
    }
    assert_eq!(fs::read(fixture.directory.path().join("count"))?, b"x");
    let original = linux_process_snapshot()?
        .flatten()
        .find(|stat| stat.pid == pid)
        .ok_or("guardian missing")?;
    assert_eq!(
        fs::read_link(Path::new("/proc").join(pid.to_string()).join("exe"))?,
        fixture.executable
    );
    LinuxPidFd::from_observation(&original)?.signal(ProcessSignal::Kill)?;
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 10);
    while host.query_run(intent.run)?.coordination != HostCoordination::Unavailable {
        host.tick()?;
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(/*millis*/ 5)).await;
    }
    assert_eq!(
        host.query_run(intent.run)?.last_observed,
        Some(expected.clone())
    );
    drop(host);
    let host = HostCoordinator::new(recover(&root).await?, fixture.executable.clone());
    assert_eq!(host.query_run(intent.run)?.last_observed, Some(expected));
    assert_eq!(fs::read(fixture.directory.path().join("count"))?, b"x");
    Ok(())
}

/// A consumed but unavailable guardian cannot hold another Scope's dispatch or recovery hostage.
#[tokio::test]
async fn unavailable_scope_does_not_stall_independent_work() -> TestResult {
    let fixture = Fixture::new()?;
    let root = fixture.directory.path().join("s");
    let mut state = HostState::create(&root)?;
    let unavailable = ScopeId::new();
    state.record_scope_intent(unavailable)?;
    let failed_guardian = fixture.directory.path().join("false");
    fs::copy("/bin/false", &failed_guardian)?;
    let _ = state.start_guardian(unavailable, &failed_guardian).await;
    assert!(state.guardian_access(unavailable)?.is_some());
    let mut host = HostCoordinator::new(state, fixture.executable.clone());
    let scope = ScopeId::new();
    host.create_scope(scope)?;
    let intent = HostRunIntent {
        scope,
        run: RunId::new(),
        spec: RunSpec::new(
            "/bin/true",
            fixture.directory.path(),
            DescendantPolicy::WaitForAll,
        ),
        host_disconnect: GuardianHostDisconnect::KeepRunning,
    };
    host.start(intent.clone())?;
    let result = complete(&mut host, intent.run).await?;
    assert_eq!(
        result.direct,
        DirectProcessState::Exited(ExitOutcome::Code(0))
    );
    assert_eq!(
        host.query_scope(unavailable)?.coordination,
        HostCoordination::Unavailable
    );
    assert_eq!(host.query_scope(unavailable)?.last_observed, None);
    Ok(())
}

/// Accepted stop survives loss of the coordinator before it can send that stop to the guardian.
#[tokio::test]
async fn recovered_host_finishes_previously_accepted_stop() -> TestResult {
    let fixture = Fixture::new()?;
    let root = fixture.directory.path().join("s");
    let mut host = HostCoordinator::new(HostState::create(&root)?, fixture.executable.clone());
    let scope = ScopeId::new();
    host.create_scope(scope)?;
    let intent = HostRunIntent {
        scope,
        run: RunId::new(),
        spec: runs::sleeper(&fixture)?,
        host_disconnect: GuardianHostDisconnect::KeepRunning,
    };
    host.start(intent.clone())?;
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 10);
    loop {
        host.tick()?;
        if host
            .query_run(intent.run)?
            .last_observed
            .is_some_and(|snapshot| snapshot.direct == DirectProcessState::Running)
        {
            break;
        }
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(/*millis*/ 5)).await;
    }
    host.stop(intent.run)?;
    drop(host);
    let mut host = HostCoordinator::new(recover(&root).await?, fixture.executable.clone());
    assert!(host.query_run(intent.run)?.stop_requested);
    assert_eq!(
        complete(&mut host, intent.run).await?,
        RunSnapshot {
            id: intent.run,
            launch: LaunchFact::Started,
            direct: DirectProcessState::Exited(ExitOutcome::Signal(libc::SIGKILL)),
            cleanup: CleanupState::Complete(CleanupEvidence::BestEffortComplete)
        }
    );
    Ok(())
}
