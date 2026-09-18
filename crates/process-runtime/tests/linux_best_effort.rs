#![cfg(target_os = "linux")]

use std::path::Path;
use std::time::{Duration, Instant};

use ora_process_protocol::{
    CleanupEvidence, CleanupState, ContainmentGuarantee, ContainmentRequest, DescendantPolicy,
    DirectProcessState, ExitOutcome, LaunchFact, RunId, RunSnapshot, RunSpec, ScopeState,
    StopRequest,
};
use ora_process_runtime::{AdmissionError, LinuxBestEffort, ScopeRuntime};
use ora_utils::process::{LinuxPidFd, linux_process_snapshot};
use pretty_assertions::assert_eq;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Owner exit triggers cleanup independently of a requester; mismatched owners cannot launch work.
#[test]
fn owner_exit_stops_only_its_run_and_stale_identity_cannot_launch() -> TestResult {
    use ora_process_protocol::RunLifetime;
    use ora_utils::process::linux_process;
    let directory = tempfile::tempdir()?;
    let mut owner = std::process::Command::new("/bin/sleep").arg("60").spawn()?;
    let identity = linux_process(owner.id())?;
    let mut scope = ScopeRuntime::new(
        ContainmentRequest::BestEffort,
        LinuxBestEffort::with_discarded_io()?,
    )?;
    let mut spec = shell(
        "exec /bin/sleep 60",
        directory.path(),
        DescendantPolicy::WaitForAll,
    );
    spec.lifetime = RunLifetime::TerminateOnOwnerExit {
        pid: identity.pid,
        start_ticks: identity.start_ticks + 1,
    };
    let rejected = scope.start(RunId::new(), spec.clone())?;
    assert!(matches!(rejected.launch, LaunchFact::NotStarted(_)));
    spec.lifetime = RunLifetime::TerminateOnOwnerExit {
        pid: identity.pid,
        start_ticks: identity.start_ticks,
    };
    let owned = RunId::new();
    scope.start(owned, spec)?;
    let independent = RunId::new();
    scope.start(
        independent,
        shell(
            "exec /bin/sleep 60",
            directory.path(),
            DescendantPolicy::WaitForAll,
        ),
    )?;
    owner.kill()?;
    owner.wait()?;
    until(&mut scope, |scope| {
        scope
            .run(owned)
            .is_some_and(|r| matches!(r.cleanup, CleanupState::Complete(_)))
    });
    assert_eq!(
        scope.run(owned),
        Some(RunSnapshot {
            id: owned,
            launch: LaunchFact::Started,
            direct: DirectProcessState::Exited(ExitOutcome::Signal(libc::SIGKILL)),
            cleanup: CleanupState::Complete(CleanupEvidence::BestEffortComplete),
        })
    );
    assert_eq!(
        scope.run(independent).map(|run| run.direct),
        Some(DirectProcessState::Running)
    );
    scope.close(StopRequest::Force, Instant::now())?;
    until(&mut scope, |scope| {
        matches!(scope.state(), ScopeState::Closed(_))
    });
    Ok(())
}

/// Polls actual process facts with a failure deadline, never uses sleep as evidence of completion.
fn until(
    scope: &mut ScopeRuntime<LinuxBestEffort>,
    mut ready: impl FnMut(&ScopeRuntime<LinuxBestEffort>) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 10);
    loop {
        let failures = scope.reconcile(Instant::now());
        if ready(scope) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "process observation timed out: {failures:?}"
        );
        std::thread::sleep(Duration::from_millis(/*millis*/ 5));
    }
}

/// Requires explicit output discard and propagates only the chosen process environment.
fn shell(script: &str, cwd: &Path, descendants: DescendantPolicy) -> RunSpec {
    let mut spec = RunSpec::new("/bin/sh", cwd, descendants);
    spec.args = ["-c".into(), script.into()].into();
    spec
}

/// Independently pins fixture identities so completion assertions also check actual termination.
fn recorded_process(directory: &Path) -> Result<LinuxPidFd, Box<dyn std::error::Error>> {
    let pid = std::fs::read_to_string(directory.join("leaf-pid"))?.parse::<u32>()?;
    for stat in linux_process_snapshot()? {
        let stat = stat?;
        if stat.pid == pid {
            return Ok(LinuxPidFd::from_observation(&stat)?);
        }
    }
    Err("missing fixture process".into())
}

/// Real admission rejects Strong, exposes fallback and replays the same attempt without exec.
#[test]
fn rootless_admission_exit_and_idempotency() -> TestResult {
    assert!(matches!(
        ScopeRuntime::new(
            ContainmentRequest::RequireStrong,
            LinuxBestEffort::with_discarded_io()?
        ),
        Err(AdmissionError::Unavailable)
    ));
    let mut scope = ScopeRuntime::new(
        ContainmentRequest::PreferStrong,
        LinuxBestEffort::with_discarded_io()?,
    )?;
    assert_eq!(scope.guarantee(), ContainmentGuarantee::BestEffort);
    let directory = tempfile::tempdir()?;
    let run = RunId::new();
    let spec = shell(
        "printf x >> executed; exit 7",
        directory.path(),
        DescendantPolicy::WaitForAll,
    );
    scope.start(run, spec.clone())?;
    until(&mut scope, |scope| {
        scope
            .run(run)
            .is_some_and(|run| matches!(run.cleanup, CleanupState::Complete(_)))
    });
    let expected = RunSnapshot {
        id: run,
        launch: LaunchFact::Started,
        direct: DirectProcessState::Exited(ExitOutcome::Code(7)),
        cleanup: CleanupState::Complete(CleanupEvidence::BestEffortComplete),
    };
    assert_eq!(scope.start(run, spec)?, expected);
    assert_eq!(std::fs::read(directory.path().join("executed"))?, b"x");
    scope.close(StopRequest::Force, Instant::now())?;
    until(&mut scope, |scope| {
        scope.state() == ScopeState::Closed(CleanupEvidence::BestEffortComplete)
    });
    Ok(())
}

/// Force-stop cleans one real process family while another run remains alive in its own session.
#[test]
fn stopping_one_run_preserves_its_neighbor() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut scope = ScopeRuntime::new(
        ContainmentRequest::BestEffort,
        LinuxBestEffort::with_discarded_io()?,
    )?;
    let a = RunId::new();
    let b = RunId::new();
    scope.start(
        a,
        shell(
            "/bin/sleep 60 & printf '%s' $! > leaf-pid; printf ready > ready; wait",
            directory.path(),
            DescendantPolicy::WaitForAll,
        ),
    )?;
    scope.start(
        b,
        shell(
            "exec /bin/sleep 60",
            directory.path(),
            DescendantPolicy::WaitForAll,
        ),
    )?;
    until(&mut scope, |_| directory.path().join("ready").exists());
    let leaf = recorded_process(directory.path())?;
    scope.stop_run(a, StopRequest::Force, Instant::now())?;
    until(&mut scope, |scope| {
        scope.run(a).is_some_and(|run| {
            run.cleanup == CleanupState::Complete(CleanupEvidence::BestEffortComplete)
        })
    });
    assert!(leaf.has_exited()?);
    assert_eq!(
        scope.run(b),
        Some(RunSnapshot {
            id: b,
            launch: LaunchFact::Started,
            direct: DirectProcessState::Running,
            cleanup: CleanupState::Pending
        })
    );
    scope.close(StopRequest::Force, Instant::now())?;
    until(&mut scope, |scope| {
        scope.state() == ScopeState::Closed(CleanupEvidence::BestEffortComplete)
    });
    Ok(())
}

/// A launcher's exit is visible before cleanup and does not release its session identity early.
#[test]
fn direct_exit_keeps_background_descendants_pending_until_stopped() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut scope = ScopeRuntime::new(
        ContainmentRequest::BestEffort,
        LinuxBestEffort::with_discarded_io()?,
    )?;
    let run = RunId::new();
    scope.start(
        run,
        shell(
            "/bin/sleep 60 & printf '%s' $! > leaf-pid; exit 3",
            directory.path(),
            DescendantPolicy::WaitForAll,
        ),
    )?;
    until(&mut scope, |scope| {
        scope
            .run(run)
            .is_some_and(|run| run.direct == DirectProcessState::Exited(ExitOutcome::Code(3)))
    });
    let leaf = recorded_process(directory.path())?;
    assert_eq!(
        scope.run(run),
        Some(RunSnapshot {
            id: run,
            launch: LaunchFact::Started,
            direct: DirectProcessState::Exited(ExitOutcome::Code(3)),
            cleanup: CleanupState::Pending
        })
    );
    scope.stop_run(run, StopRequest::Force, Instant::now())?;
    until(&mut scope, |scope| {
        scope.run(run).is_some_and(|run| {
            run.cleanup == CleanupState::Complete(CleanupEvidence::BestEffortComplete)
        })
    });
    assert!(leaf.has_exited()?);
    Ok(())
}

/// The lifecycle kernel's descendant policy drives the real rootless adapter, not just a fake.
#[test]
fn direct_exit_policy_cleans_background_descendants() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut scope = ScopeRuntime::new(
        ContainmentRequest::BestEffort,
        LinuxBestEffort::with_discarded_io()?,
    )?;
    let run = RunId::new();
    scope.start(
        run,
        shell(
            "/bin/sleep 60 & exit 0",
            directory.path(),
            DescendantPolicy::Cleanup {
                grace: Duration::ZERO,
            },
        ),
    )?;
    until(&mut scope, |scope| {
        scope.run(run).is_some_and(|run| {
            run.cleanup == CleanupState::Complete(CleanupEvidence::BestEffortComplete)
        })
    });
    assert_eq!(
        scope.run(run),
        Some(RunSnapshot {
            id: run,
            launch: LaunchFact::Started,
            direct: DirectProcessState::Exited(ExitOutcome::Code(0)),
            cleanup: CleanupState::Complete(CleanupEvidence::BestEffortComplete)
        })
    );
    Ok(())
}

/// A real exec failure is terminal without creating a live tracked process.
#[test]
fn missing_executable_is_not_started() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut scope = ScopeRuntime::new(
        ContainmentRequest::BestEffort,
        LinuxBestEffort::with_discarded_io()?,
    )?;
    let spec = RunSpec::new(
        directory.path().join("missing"),
        directory.path(),
        DescendantPolicy::WaitForAll,
    );
    let result = scope.start(RunId::new(), spec)?;
    assert!(matches!(result.launch, LaunchFact::NotStarted(_)));
    assert_eq!(
        (result.direct, result.cleanup),
        (
            DirectProcessState::NotStarted,
            CleanupState::Complete(CleanupEvidence::BestEffortComplete)
        )
    );
    Ok(())
}

/// A TERM-ignoring process remains pending until the caller escalates to force.
#[test]
fn notification_does_not_manufacture_cleanup_and_can_escalate() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut scope = ScopeRuntime::new(
        ContainmentRequest::BestEffort,
        LinuxBestEffort::with_discarded_io()?,
    )?;
    let run = RunId::new();
    scope.start(
        run,
        shell(
            "trap '' TERM; printf ready > ready; while :; do /bin/sleep 60; done",
            directory.path(),
            DescendantPolicy::WaitForAll,
        ),
    )?;
    until(&mut scope, |_| directory.path().join("ready").exists());
    let now = Instant::now();
    scope.stop_run(
        run,
        StopRequest::NotifyThenWait {
            timeout: Duration::from_secs(/*secs*/ 60),
        },
        now,
    )?;
    assert_eq!(scope.reconcile(now), Vec::new());
    assert_eq!(
        scope.run(run),
        Some(RunSnapshot {
            id: run,
            launch: LaunchFact::Started,
            direct: DirectProcessState::Running,
            cleanup: CleanupState::Pending
        })
    );
    scope.stop_run(run, StopRequest::Force, Instant::now())?;
    until(&mut scope, |scope| {
        scope.run(run).is_some_and(|run| {
            run.cleanup == CleanupState::Complete(CleanupEvidence::BestEffortComplete)
        })
    });
    Ok(())
}

/// Dropping the in-memory owner initiates best-effort termination without blocking its destructor.
#[test]
fn dropping_scope_stops_its_owned_direct_child() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut scope = ScopeRuntime::new(
        ContainmentRequest::BestEffort,
        LinuxBestEffort::with_discarded_io()?,
    )?;
    scope.start(
        RunId::new(),
        shell(
            "printf '%s' $$ > pid; exec /bin/sleep 60",
            directory.path(),
            DescendantPolicy::WaitForAll,
        ),
    )?;
    until(&mut scope, |_| {
        std::fs::read_to_string(directory.path().join("pid"))
            .is_ok_and(|text| text.parse::<u32>().is_ok())
    });
    let pid = std::fs::read_to_string(directory.path().join("pid"))?.parse::<u32>()?;
    let stat = linux_process_snapshot()?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .find(|stat| stat.pid == pid)
        .ok_or("missing child")?;
    let handle = LinuxPidFd::from_observation(&stat)?;
    drop(scope);
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 5);
    while !handle.has_exited()? {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(/*millis*/ 5));
    }
    Ok(())
}

/// A pinned member cannot escape its stop request merely by creating another session.
#[test]
fn tracked_member_is_stopped_after_setsid() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut scope = ScopeRuntime::new(
        ContainmentRequest::BestEffort,
        LinuxBestEffort::with_discarded_io()?,
    )?;
    let mut spec = RunSpec::new(
        std::env::current_exe()?,
        directory.path(),
        DescendantPolicy::WaitForAll,
    );
    spec.args = [
        "--exact".into(),
        "rootless_fixture".into(),
        "--nocapture".into(),
    ]
    .into();
    spec.env
        .insert("ORA_ROOTLESS_TEST_ROLE".into(), "parent".into());
    let run = RunId::new();
    scope.start(run, spec)?;
    until(&mut scope, |_| directory.path().join("leaf-ready").exists());
    let leaf = recorded_process(directory.path())?;
    // This successful scan runs after readiness, ensuring the leaf is pinned before detaching.
    assert_eq!(scope.reconcile(Instant::now()), Vec::new());
    std::fs::write(directory.path().join("detach"), b"go")?;
    until(&mut scope, |_| directory.path().join("detached").exists());
    scope.stop_run(run, StopRequest::Force, Instant::now())?;
    until(&mut scope, |scope| {
        scope.run(run).is_some_and(|run| {
            run.cleanup == CleanupState::Complete(CleanupEvidence::BestEffortComplete)
        })
    });
    assert!(leaf.has_exited()?);
    Ok(())
}

/// Runs only in explicitly configured child test processes; the ordinary test invocation is inert.
#[test]
fn rootless_fixture() -> TestResult {
    match std::env::var("ORA_ROOTLESS_TEST_ROLE").as_deref() {
        Ok("parent") => {
            let mut child = std::process::Command::new(std::env::current_exe()?)
                .args(["--exact", "rootless_fixture", "--nocapture"])
                .env("ORA_ROOTLESS_TEST_ROLE", "leaf")
                .spawn()?;
            child.wait()?;
        }
        Ok("leaf") => {
            std::fs::write("leaf-pid", std::process::id().to_string())?;
            std::fs::write("leaf-ready", b"ready")?;
            let deadline = Instant::now() + Duration::from_secs(/*secs*/ 20);
            while !Path::new("detach").exists() {
                assert!(Instant::now() < deadline, "detach handshake timed out");
                std::thread::sleep(Duration::from_millis(/*millis*/ 5));
            }
            // SAFETY: changes the session only of this dedicated fixture process.
            assert!(unsafe { libc::setsid() } > 0);
            std::fs::write("detached", b"ready")?;
            while Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(/*millis*/ 5));
            }
        }
        Ok(_) | Err(_) => {}
    }
    Ok(())
}
