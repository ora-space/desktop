use super::*;
use ora_process_client::{GuardianManagement, GuardianRuns};
use ora_process_protocol::{
    CleanupEvidence, CleanupState, DescendantPolicy, DirectProcessState, ExitOutcome,
    GuardianHostDisconnect, GuardianHostSession, GuardianRunOperation as Operation,
    GuardianRunRejection as Rejection, GuardianRunReply, GuardianRunRequest,
    GuardianRunResult as Reply, LaunchFact, OutputPolicy, OutputRead, OutputState, OutputStream,
    RunId, RunSnapshot, RunSpec, ScopeState, decode_guardian_payload,
};
use pretty_assertions::assert_eq;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[path = "runs/admission.rs"]
mod admission;

/// Uses the public management API to get a non-secret binding for the production Run client.
pub(super) async fn bind(
    access: &GuardianAccess,
    host: ora_process_protocol::HostBinding,
) -> Result<GuardianHostSession, Box<dyn std::error::Error>> {
    // SAFETY: geteuid only reads the current OS identity.
    let manager = GuardianManagement::new(access.clone(), unsafe { libc::geteuid() });
    match manager.bind(host).await? {
        ora_process_protocol::GuardianManagementReply::Bound { session, .. } => Ok(session),
        other => Err(format!("expected bound session: {other:?}").into()),
    }
}

/// Polls terminal cleanup instead of assuming process exit means all cleanup has finished.
pub(super) async fn completed(
    client: &GuardianRuns,
    run: RunId,
) -> Result<RunSnapshot, Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 10);
    loop {
        if let Reply::Run(snapshot) = client.execute(Operation::Query { run }).await?
            && matches!(snapshot.cleanup, CleanupState::Complete(_))
        {
            return Ok(snapshot);
        }
        assert!(Instant::now() < deadline, "Run did not complete");
        tokio::time::sleep(Duration::from_millis(/*millis*/ 5)).await;
    }
}

/// Uses a fixture-specific executable so teardown can identify only this test's surviving workload.
pub(super) fn sleeper(fixture: &Fixture) -> Result<RunSpec, Box<dyn std::error::Error>> {
    let program = fixture.directory.path().join("sleep");
    fs::copy("/bin/sleep", &program)?;
    let mut spec = RunSpec::new(
        program,
        fixture.directory.path(),
        DescendantPolicy::WaitForAll,
    );
    spec.args.push("60".into());
    Ok(spec)
}

/// Verifies real exec, exact-ID replay, journaled exit, volatile stream facts and missing-run refusal.
#[tokio::test]
async fn run_exit_output_and_exact_replay() -> TestResult {
    let fixture = Fixture::new()?;
    let mut host = HostState::create(&fixture.directory.path().join("s"))?;
    let scope = ScopeId::new();
    host.record_scope_intent(scope)?;
    let access = host.start_guardian(scope, &fixture.executable).await?;
    ready(&access).await?;
    let session = bind(&access, host.binding()).await?;
    // SAFETY: geteuid only reads the current OS identity.
    let client = GuardianRuns::new(access.clone(), unsafe { libc::geteuid() }, session);
    let run = RunId::new();
    let mut spec = RunSpec::new(
        "/bin/sh",
        fixture.directory.path(),
        DescendantPolicy::WaitForAll,
    );
    spec.args = [
        "-c",
        "printf x >> count; printf abcdef; printf ERR >&2; exit 7",
    ]
    .map(Into::into)
    .to_vec();
    spec.output = OutputPolicy::Capture {
        stdout_limit: 4,
        stderr_limit: 4,
    };
    let start = Operation::Start {
        run,
        spec: spec.clone(),
        host_disconnect: GuardianHostDisconnect::KeepRunning,
    };
    assert!(matches!(
        client.execute(start.clone()).await?,
        Reply::Run(_)
    ));
    let terminal = RunSnapshot {
        id: run,
        launch: LaunchFact::Started,
        direct: DirectProcessState::Exited(ExitOutcome::Code(7)),
        cleanup: CleanupState::Complete(CleanupEvidence::BestEffortComplete),
    };
    // Read SQLite only: terminal recording must progress with no client request driving reconcile.
    let journal = rusqlite::Connection::open_with_flags(
        access.scope_dir.join("guardian.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 5);
    loop {
        let bytes: Vec<u8> = journal.query_row(
            "SELECT snapshot FROM guardian_runs WHERE run=?1",
            [run.to_string()],
            |row| row.get(/*idx*/ 0),
        )?;
        let snapshot: RunSnapshot = decode_guardian_payload(&bytes)?;
        if matches!(snapshot.cleanup, CleanupState::Complete(_)) {
            assert_eq!(snapshot, terminal);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "guardian background reconciliation stalled"
        );
        tokio::time::sleep(Duration::from_millis(/*millis*/ 5)).await;
    }
    assert_eq!(client.execute(start).await?, Reply::Run(terminal.clone()));
    assert_eq!(fs::read(fixture.directory.path().join("count"))?, b"x");
    for (stream, bytes, truncated) in [
        (OutputStream::Stdout, b"abcd".to_vec(), true),
        (OutputStream::Stderr, b"ERR".to_vec(), false),
    ] {
        let deadline = Instant::now() + Duration::from_secs(/*secs*/ 5);
        loop {
            let reply = client
                .execute(Operation::Output {
                    run,
                    stream,
                    offset: 0,
                    max_bytes: 4096,
                })
                .await?;
            if matches!(&reply, Reply::Output { output, .. } if output.state == OutputState::Eof) {
                assert_eq!(
                    reply,
                    Reply::Output {
                        run,
                        stream,
                        offset: 0,
                        output: OutputRead {
                            retained: bytes.len(),
                            bytes,
                            truncated,
                            state: OutputState::Eof
                        }
                    }
                );
                break;
            }
            assert!(Instant::now() < deadline, "output never reached EOF");
            tokio::time::sleep(Duration::from_millis(/*millis*/ 5)).await;
        }
    }
    spec.args.push("changed".into());
    assert_eq!(
        client
            .execute(Operation::Start {
                run,
                spec,
                host_disconnect: GuardianHostDisconnect::KeepRunning
            })
            .await?,
        Reply::Rejected(Rejection::ConflictingRun)
    );
    let missing = RunId::new();
    assert_eq!(
        client.execute(Operation::Query { run: missing }).await?,
        Reply::Rejected(Rejection::UnknownRun)
    );
    assert_eq!(
        client.execute(Operation::Stop { run: missing }).await?,
        Reply::Rejected(Rejection::UnknownRun)
    );
    assert_eq!(
        client
            .execute(Operation::Output {
                run,
                stream: OutputStream::Stdout,
                offset: 0,
                max_bytes: 4097
            })
            .await?,
        Reply::Rejected(Rejection::LimitExceeded)
    );
    let bytes: Vec<u8> = journal.query_row(
        "SELECT snapshot FROM guardian_runs WHERE run=?1",
        [run.to_string()],
        |row| row.get(/*idx*/ 0),
    )?;
    assert_eq!(decode_guardian_payload::<RunSnapshot>(&bytes)?, terminal);
    Ok(())
}

/// Lost Start replies do not repeat exec; a replacement host can stop and seal the original scope.
#[tokio::test]
async fn lost_reply_takeover_force_stop_and_close() -> TestResult {
    let fixture = Fixture::new()?;
    let root = fixture.directory.path().join("s");
    let mut host = HostState::create(&root)?;
    let scope = ScopeId::new();
    host.record_scope_intent(scope)?;
    let access = host.start_guardian(scope, &fixture.executable).await?;
    ready(&access).await?;
    let session = bind(&access, host.binding()).await?;
    // SAFETY: geteuid only reads the current OS identity.
    let owner = unsafe { libc::geteuid() };
    let old = GuardianRuns::new(access.clone(), owner, session.clone());
    let run = RunId::new();
    // A Run operation cannot use a valid host binding on the wrong physical channel.
    let mut wrong_channel =
        tokio::net::UnixStream::connect(access.scope_dir.join("io.sock")).await?;
    wrong_channel
        .write_all(&encode_guardian_frame(&GuardianRunRequest {
            version: GUARDIAN_WIRE_VERSION,
            intent: access.intent.clone(),
            channel: GuardianChannel::Io,
            session: session.clone(),
            operation: Operation::Stop { run },
        })?)
        .await?;
    let length = wrong_channel.read_u32().await? as usize;
    assert!(length <= ora_process_protocol::GUARDIAN_MAX_FRAME);
    let mut bytes = vec![0; length];
    wrong_channel.read_exact(&mut bytes).await?;
    assert_eq!(
        decode_guardian_payload::<GuardianRunReply>(&bytes)?,
        GuardianRunReply {
            intent: access.intent.clone(),
            session: session.clone(),
            result: Reply::Rejected(Rejection::Management(
                ora_process_protocol::GuardianManagementRejection::WrongChannel
            )),
        }
    );
    let start = Operation::Start {
        run,
        spec: sleeper(&fixture)?,
        host_disconnect: GuardianHostDisconnect::KeepRunning,
    };
    let mut lost = tokio::net::UnixStream::connect(access.scope_dir.join("control.sock")).await?;
    lost.write_all(&encode_guardian_frame(&GuardianRunRequest {
        version: GUARDIAN_WIRE_VERSION,
        intent: access.intent.clone(),
        channel: GuardianChannel::Control,
        session: session.clone(),
        operation: start.clone(),
    })?)
    .await?;
    // Wait for durable acceptance, never consuming the Start response.
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 5);
    loop {
        if matches!(old.execute(Operation::Query { run }).await?, Reply::Run(_)) {
            break;
        }
        assert!(Instant::now() < deadline, "Start was never accepted");
        tokio::time::sleep(Duration::from_millis(/*millis*/ 5)).await;
    }
    drop(lost);
    drop(host);
    let recovered = recover(&root).await?;
    let current = bind(&access, recovered.binding()).await?;
    let client = GuardianRuns::new(access.clone(), owner, current);
    let running = Reply::Run(RunSnapshot {
        id: run,
        launch: LaunchFact::Started,
        direct: DirectProcessState::Running,
        cleanup: CleanupState::Pending,
    });
    assert_eq!(client.execute(start.clone()).await?, running);
    assert_eq!(
        old.execute(Operation::Stop { run }).await?,
        Reply::Rejected(Rejection::Management(
            ora_process_protocol::GuardianManagementRejection::StaleSession
        ))
    );
    assert_eq!(client.execute(Operation::Query { run }).await?, running);
    assert!(matches!(
        client.execute(Operation::Stop { run }).await?,
        Reply::Run(_)
    ));
    let stopped = RunSnapshot {
        id: run,
        launch: LaunchFact::Started,
        direct: DirectProcessState::Exited(ExitOutcome::Signal(libc::SIGKILL)),
        cleanup: CleanupState::Complete(CleanupEvidence::BestEffortComplete),
    };
    assert_eq!(completed(&client, run).await?, stopped);
    let other = RunId::new();
    let mut second = start.clone();
    if let Operation::Start { run, .. } = &mut second {
        *run = other;
    }
    assert!(matches!(client.execute(second).await?, Reply::Run(_)));
    client.execute(Operation::Close).await?;
    completed(&client, other).await?;
    assert_eq!(
        client.execute(Operation::Scope).await?,
        Reply::Scope(ScopeState::Closed(CleanupEvidence::BestEffortComplete))
    );
    assert_eq!(client.execute(start).await?, Reply::Run(stopped));
    assert_eq!(
        client
            .execute(Operation::Start {
                run: RunId::new(),
                spec: sleeper(&fixture)?,
                host_disconnect: GuardianHostDisconnect::KeepRunning
            })
            .await?,
        Reply::Rejected(Rejection::ScopeClosed)
    );
    Ok(())
}

/// Guardian death leaves historical facts, not authority to respawn or signal a reused numeric PID.
#[tokio::test]
async fn guardian_kill_preserves_attempt_and_workload_does_not_hold_scope_lock() -> TestResult {
    let fixture = Fixture::new()?;
    let mut host = HostState::create(&fixture.directory.path().join("s"))?;
    let scope = ScopeId::new();
    host.record_scope_intent(scope)?;
    let access = host.start_guardian(scope, &fixture.executable).await?;
    ready(&access).await?;
    // SAFETY: geteuid only reads the current identity.
    let client = GuardianRuns::new(
        access.clone(),
        unsafe { libc::geteuid() },
        bind(&access, host.binding()).await?,
    );
    let run = RunId::new();
    let expected = Reply::Run(RunSnapshot {
        id: run,
        launch: LaunchFact::Started,
        direct: DirectProcessState::Running,
        cleanup: CleanupState::Pending,
    });
    assert_eq!(
        client
            .execute(Operation::Start {
                run,
                spec: sleeper(&fixture)?,
                host_disconnect: GuardianHostDisconnect::KeepRunning
            })
            .await?,
        expected
    );
    let guardian = guardian_pid(&access).await?;
    let stat = linux_process_snapshot()?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .find(|stat| stat.pid == guardian)
        .ok_or("guardian not found")?;
    let handle = LinuxPidFd::from_observation(&stat)?;
    handle.signal(ProcessSignal::Kill)?;
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 5);
    loop {
        match LinuxFileLock::try_acquire(fs::File::open(access.scope_dir.join("guardian.lock"))?) {
            Ok(lock) => {
                drop(lock);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "workload retained guardian lock");
                tokio::time::sleep(Duration::from_millis(/*millis*/ 5)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
    assert!(client.execute(Operation::Query { run }).await.is_err());
    assert!(
        host.start_guardian(scope, &fixture.executable)
            .await
            .is_err()
    );
    let journal = rusqlite::Connection::open_with_flags(
        access.scope_dir.join("guardian.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let bytes: Vec<u8> = journal.query_row(
        "SELECT snapshot FROM guardian_runs WHERE run=?1",
        [run.to_string()],
        |row| row.get(/*idx*/ 0),
    )?;
    assert_eq!(Reply::Run(decode_guardian_payload(&bytes)?), expected);
    let survivor = linux_process_snapshot()?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .find(|stat| {
            fs::read_link(Path::new("/proc").join(stat.pid.to_string()).join("exe")).ok()
                == Some(fixture.directory.path().join("sleep"))
        })
        .ok_or("expected rootless survivor")?;
    assert!(!LinuxPidFd::from_observation(&survivor)?.has_exited()?);
    Ok(())
}

/// Storage failure before acceptance cannot exec; failure after exec retains one uncertain attempt.
#[tokio::test]
async fn run_journal_failures_never_retry_an_accepted_exec() -> TestResult {
    let fixture = Fixture::new()?;
    let mut host = HostState::create(&fixture.directory.path().join("s"))?;
    let scope = ScopeId::new();
    host.record_scope_intent(scope)?;
    let access = host.start_guardian(scope, &fixture.executable).await?;
    ready(&access).await?;
    // SAFETY: geteuid only reads the current OS identity.
    let client = GuardianRuns::new(
        access.clone(),
        unsafe { libc::geteuid() },
        bind(&access, host.binding()).await?,
    );
    let journal = rusqlite::Connection::open(access.scope_dir.join("guardian.sqlite"))?;
    journal.execute_batch("CREATE TRIGGER reject_insert BEFORE INSERT ON guardian_runs BEGIN SELECT RAISE(ABORT, 'fixture'); END;")?;
    let run = RunId::new();
    let mut spec = RunSpec::new(
        "/bin/sh",
        fixture.directory.path(),
        DescendantPolicy::WaitForAll,
    );
    spec.args = ["-c", "printf x >> count"].map(Into::into).to_vec();
    let start = Operation::Start {
        run,
        spec,
        host_disconnect: GuardianHostDisconnect::KeepRunning,
    };
    assert_eq!(
        client.execute(start.clone()).await?,
        Reply::Rejected(Rejection::StorageUnavailable)
    );
    assert!(!fixture.directory.path().join("count").exists());
    journal.execute_batch("DROP TRIGGER reject_insert; CREATE TRIGGER reject_snapshot BEFORE UPDATE OF snapshot ON guardian_runs BEGIN SELECT RAISE(ABORT, 'fixture'); END;")?;
    assert_eq!(
        client.execute(start.clone()).await?,
        Reply::Rejected(Rejection::StorageUnavailable)
    );
    assert_eq!(
        client.execute(start.clone()).await?,
        Reply::Rejected(Rejection::StorageUnavailable)
    );
    let bytes: Vec<u8> = journal.query_row(
        "SELECT snapshot FROM guardian_runs WHERE run=?1",
        [run.to_string()],
        |row| row.get(/*idx*/ 0),
    )?;
    assert_eq!(
        decode_guardian_payload::<RunSnapshot>(&bytes)?,
        RunSnapshot {
            id: run,
            launch: LaunchFact::Unknown("accepted; execution not yet observed".into()),
            direct: DirectProcessState::Unknown,
            cleanup: CleanupState::Pending
        }
    );
    journal.execute_batch("DROP TRIGGER reject_snapshot;")?;
    let terminal = completed(&client, run).await?;
    assert_eq!(client.execute(start).await?, Reply::Run(terminal));
    assert_eq!(fs::read(fixture.directory.path().join("count"))?, b"x");
    Ok(())
}
