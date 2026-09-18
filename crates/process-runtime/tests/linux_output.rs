#![cfg(target_os = "linux")]

use std::path::Path;
use std::time::{Duration, Instant};

use ora_process_protocol::{
    CleanupState, ContainmentRequest, DescendantPolicy, DirectProcessState, ExitOutcome,
    LaunchFact, OutputPolicy, OutputRead, OutputState, OutputStream, RunId, RunSpec, StopRequest,
};
use ora_process_runtime::{LinuxBestEffort, ScopeRuntime, StartError};
use pretty_assertions::assert_eq;

type TestResult = Result<(), Box<dyn std::error::Error>>;
type Scope = ScopeRuntime<LinuxBestEffort>;

/// Uses explicit independent byte limits and no inherited environment.
fn shell(script: &str, cwd: &Path, stdout_limit: usize, stderr_limit: usize) -> RunSpec {
    let mut spec = RunSpec::new("/bin/sh", cwd, DescendantPolicy::WaitForAll);
    spec.args = vec!["-c".into(), script.into()];
    spec.output = OutputPolicy::Capture {
        stdout_limit,
        stderr_limit,
    };
    spec
}

/// Drives ownership until actual facts satisfy the assertion, never treats a delay as evidence.
fn until(scope: &mut Scope, mut ready: impl FnMut(&Scope) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 10);
    loop {
        let failures = scope.reconcile(Instant::now());
        if ready(scope) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out: {failures:?}");
        std::thread::sleep(Duration::from_millis(/*millis*/ 5));
    }
}

/// Requires both EOFs separately from completed cleanup.
fn finished(scope: &Scope, run: RunId) -> bool {
    matches!(
        scope.run(run).map(|snapshot| snapshot.cleanup),
        Some(CleanupState::Complete(_))
    ) && [OutputStream::Stdout, OutputStream::Stderr]
        .into_iter()
        .all(|stream| {
            scope
                .read_output(run, stream, /*offset*/ 0, /*max_bytes*/ 0)
                .is_ok_and(|read| read.state == OutputState::Eof)
        })
}

/// Each pipe preserves binary ordering and exact-limit completeness; replay cannot change limits.
#[test]
fn exact_limits_replay_and_partial_reads() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut scope = Scope::new(
        ContainmentRequest::BestEffort,
        LinuxBestEffort::with_bounded_output()?,
    )?;
    let run = RunId::new();
    let spec = shell(
        "printf 'a\\000bc'; printf 'err' >&2",
        directory.path(),
        /*stdout_limit*/ 4,
        /*stderr_limit*/ 3,
    );
    scope.start(run, spec.clone())?;
    until(&mut scope, |scope| finished(scope, run));
    let expected = OutputRead {
        bytes: b"a\0bc".to_vec(),
        retained: 4,
        truncated: false,
        state: OutputState::Eof,
    };
    assert_eq!(
        scope.read_output(
            run,
            OutputStream::Stdout,
            /*offset*/ 0,
            /*max_bytes*/ 100
        )?,
        expected
    );
    assert_eq!(
        scope
            .read_output(
                run,
                OutputStream::Stdout,
                /*offset*/ 1,
                /*max_bytes*/ 2
            )?
            .bytes,
        b"\0b"
    );
    assert_eq!(
        scope.read_output(
            run,
            OutputStream::Stderr,
            /*offset*/ 0,
            /*max_bytes*/ 100
        )?,
        OutputRead {
            bytes: b"err".to_vec(),
            retained: 3,
            truncated: false,
            state: OutputState::Eof,
        }
    );
    assert_eq!(
        scope.start(run, spec.clone())?,
        scope.run(run).ok_or("missing run")?
    );
    let mut changed = spec;
    changed.output = OutputPolicy::Capture {
        stdout_limit: 5,
        stderr_limit: 3,
    };
    assert_eq!(
        scope.start(run, changed),
        Err(StartError::ConflictingRun(run))
    );
    assert!(
        scope
            .read_output(
                run,
                OutputStream::Stdout,
                /*offset*/ 5,
                /*max_bytes*/ 1
            )
            .is_err()
    );
    Ok(())
}

/// Both readers drain beyond pipe capacity without any output consumer or reconciliation.
#[test]
fn readers_progress_without_consumer() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut scope = Scope::new(
        ContainmentRequest::BestEffort,
        LinuxBestEffort::with_bounded_output()?,
    )?;
    let run = RunId::new();
    scope.start(run, shell("/usr/bin/head -c 131072 /dev/zero & /usr/bin/head -c 131072 /dev/zero >&2 & wait; : > done", directory.path(), /*stdout_limit*/ 131072, /*stderr_limit*/ 131072))?;
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 10);
    while !directory.path().join("done").exists() {
        assert!(Instant::now() < deadline, "reader failed to drain");
        std::thread::sleep(Duration::from_millis(/*millis*/ 5));
    }
    until(&mut scope, |scope| finished(scope, run));
    for stream in [OutputStream::Stdout, OutputStream::Stderr] {
        assert_eq!(
            scope.read_output(run, stream, /*offset*/ 0, /*max_bytes*/ 131072)?,
            OutputRead {
                bytes: vec![0; 131072],
                retained: 131072,
                truncated: false,
                state: OutputState::Eof,
            }
        );
    }
    Ok(())
}

/// Either channel can trigger forced cleanup without stopping a neighboring run.
#[test]
fn overflow_stops_only_its_run() -> TestResult {
    let directory = tempfile::tempdir()?;
    for (redirect, stream) in [("", OutputStream::Stdout), (">&2", OutputStream::Stderr)] {
        let mut scope = Scope::new(
            ContainmentRequest::BestEffort,
            LinuxBestEffort::with_bounded_output()?,
        )?;
        let run = RunId::new();
        let peer = RunId::new();
        scope.start(
            peer,
            shell(
                "exec /bin/sleep 30",
                directory.path(),
                /*stdout_limit*/ 0,
                /*stderr_limit*/ 0,
            ),
        )?;
        scope.start(
            run,
            shell(
                &format!("trap '' TERM; while :; do printf abcdefgh {redirect}; done"),
                directory.path(),
                /*stdout_limit*/ 17,
                /*stderr_limit*/ 17,
            ),
        )?;
        until(&mut scope, |scope| finished(scope, run));
        assert_eq!(
            scope.read_output(run, stream, /*offset*/ 0, /*max_bytes*/ 100)?,
            OutputRead {
                bytes: b"abcdefghabcdefgha".to_vec(),
                retained: 17,
                truncated: true,
                state: OutputState::Eof,
            }
        );
        assert_eq!(
            scope.run(run).ok_or("missing run")?.direct,
            DirectProcessState::Exited(ExitOutcome::Signal(libc::SIGKILL))
        );
        assert_eq!(
            scope.run(peer).ok_or("missing peer")?.direct,
            DirectProcessState::Running
        );
        scope.stop_run(peer, StopRequest::Force, Instant::now())?;
        until(&mut scope, |scope| finished(scope, peer));
    }
    Ok(())
}

/// A surviving descendant keeps pipes open after direct exit, and control still stops the run.
#[test]
fn descendant_pipe_outlives_direct_exit() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut scope = Scope::new(
        ContainmentRequest::BestEffort,
        LinuxBestEffort::with_bounded_output()?,
    )?;
    let run = RunId::new();
    scope.start(
        run,
        shell(
            "/bin/sleep 30 & exit 7",
            directory.path(),
            /*stdout_limit*/ 0,
            /*stderr_limit*/ 0,
        ),
    )?;
    until(&mut scope, |scope| {
        scope.run(run).is_some_and(|snapshot| {
            snapshot.direct == DirectProcessState::Exited(ExitOutcome::Code(7))
        })
    });
    assert_eq!(
        scope.read_output(
            run,
            OutputStream::Stdout,
            /*offset*/ 0,
            /*max_bytes*/ 1
        )?,
        OutputRead {
            bytes: Vec::new(),
            retained: 0,
            truncated: false,
            state: OutputState::Open,
        }
    );
    assert_eq!(
        scope.run(run).ok_or("missing run")?.cleanup,
        CleanupState::Pending
    );
    scope.stop_run(run, StopRequest::Force, Instant::now())?;
    until(&mut scope, |scope| finished(scope, run));
    Ok(())
}

/// EOF alone cannot complete a still-running process or its cleanup.
#[test]
fn eof_can_precede_process_exit() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut scope = Scope::new(
        ContainmentRequest::BestEffort,
        LinuxBestEffort::with_bounded_output()?,
    )?;
    let run = RunId::new();
    scope.start(
        run,
        shell(
            "exec 1>&- 2>&-; exec /bin/sleep 30",
            directory.path(),
            /*stdout_limit*/ 0,
            /*stderr_limit*/ 0,
        ),
    )?;
    until(&mut scope, |scope| {
        scope
            .read_output(
                run,
                OutputStream::Stdout,
                /*offset*/ 0,
                /*max_bytes*/ 0,
            )
            .is_ok_and(|read| read.state == OutputState::Eof)
    });
    assert_eq!(
        scope.run(run).ok_or("missing run")?.direct,
        DirectProcessState::Running
    );
    assert_eq!(
        scope.run(run).ok_or("missing run")?.cleanup,
        CleanupState::Pending
    );
    scope.stop_run(run, StopRequest::Force, Instant::now())?;
    until(&mut scope, |scope| finished(scope, run));
    Ok(())
}

/// Discard-only construction rejects capture before execution, rather than silently losing bytes.
#[test]
fn unsupported_capture_never_executes() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut scope = Scope::new(
        ContainmentRequest::BestEffort,
        LinuxBestEffort::with_discarded_io()?,
    )?;
    let run = RunId::new();
    assert!(matches!(
        scope
            .start(
                run,
                shell(
                    ": > executed",
                    directory.path(),
                    /*stdout_limit*/ 1,
                    /*stderr_limit*/ 1
                )
            )?
            .launch,
        LaunchFact::NotStarted(_)
    ));
    assert!(!directory.path().join("executed").exists());
    assert!(
        scope
            .read_output(
                run,
                OutputStream::Stdout,
                /*offset*/ 0,
                /*max_bytes*/ 1
            )
            .is_err()
    );
    Ok(())
}
