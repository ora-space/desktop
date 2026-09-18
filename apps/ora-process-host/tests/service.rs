#![cfg(target_os = "linux")]

use ora_process_client::ProcessHost;
use ora_process_protocol::{
    CleanupEvidence, CleanupState, DescendantPolicy, DirectProcessState, ExitOutcome,
    GuardianHostDisconnect, HostOperation as Operation, HostReply as Reply, HostRunIntent,
    LaunchFact, RunId, RunSnapshot, RunSpec, ScopeId, ScopeState,
};
use ora_utils::process::{LinuxPidFd, ProcessSignal, linux_process_snapshot};
use pretty_assertions::assert_eq;
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use tokio::io::AsyncWriteExt;

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct Fixture {
    directory: tempfile::TempDir,
    guardian: PathBuf,
    root: PathBuf,
}

impl Fixture {
    /// Test deployment is explicit and private; production host never derives paths from HOME.
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let parent = PathBuf::from(std::env::var_os("HOME").ok_or("fixture HOME missing")?)
            .canonicalize()?;
        let directory = tempfile::Builder::new()
            .prefix("h")
            .permissions(fs::Permissions::from_mode(/*mode*/ 0o700))
            .tempdir_in(parent)?;
        let guardian = directory.path().join("guardian");
        // Workspace tests build both apps. For a targeted run, build ora-process-guardian first.
        let guardian_binary = Path::new(env!("CARGO_BIN_EXE_ora-process-host"))
            .with_file_name("ora-process-guardian");
        fs::copy(guardian_binary, &guardian)?;
        fs::set_permissions(&guardian, fs::Permissions::from_mode(/*mode*/ 0o700))?;
        let root = directory.path().join("s");
        Ok(Self {
            directory,
            guardian,
            root,
        })
    }

    /// Each fresh client connects through the production local transport, with no shared caller state.
    fn client(&self) -> ProcessHost {
        // SAFETY: geteuid reads only the fixture process identity.
        ProcessHost::new(self.root.clone(), unsafe { libc::geteuid() })
    }

    /// Starts the actual host executable with an intentionally unrelated HOME and working directory.
    fn launch(&self, mode: &str) -> Result<ChildGuard, std::io::Error> {
        Ok(ChildGuard(
            Command::new(env!("CARGO_BIN_EXE_ora-process-host"))
                .arg(mode)
                .arg(&self.root)
                .arg(&self.guardian)
                .env("HOME", self.directory.path().join("not-state"))
                .current_dir(self.directory.path())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()?,
        ))
    }
}

impl Drop for Fixture {
    /// Teardown targets only this test's unique executable paths through revalidated pidfds.
    fn drop(&mut self) {
        if let Ok(snapshot) = linux_process_snapshot() {
            for stat in snapshot.flatten() {
                if fs::read_link(Path::new("/proc").join(stat.pid.to_string()).join("exe"))
                    .ok()
                    .is_some_and(|path| {
                        path == self.guardian || path == self.directory.path().join("sleep")
                    })
                    && let Ok(handle) = LinuxPidFd::from_observation(&stat)
                {
                    let _ = handle.signal(ProcessSignal::Kill);
                }
            }
        }
    }
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    /// Failed assertions cannot leave the real host process or its child handle behind.
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Ready is a production reply, not a socket-file existence check or arbitrary startup delay.
async fn ready(
    fixture: &Fixture,
    child: &mut ChildGuard,
) -> Result<ora_process_protocol::HostBinding, Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 10);
    loop {
        if let Ok(Reply::Ready(binding)) = fixture.client().execute(Operation::Inspect).await {
            return Ok(binding);
        }
        assert!(child.0.try_wait()?.is_none(), "host exited before Ready");
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(/*millis*/ 5)).await;
    }
}

/// Polls a requested condition through host projections without driving reconciliation in the client.
async fn observed(
    client: &ProcessHost,
    run: RunId,
    predicate: impl Fn(&RunSnapshot) -> bool,
) -> Result<RunSnapshot, Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 10);
    loop {
        if let Reply::Run(view) = client.execute(Operation::QueryRun { run }).await?
            && let Some(snapshot) = view.last_observed
            && predicate(&snapshot)
        {
            return Ok(snapshot);
        }
        assert!(
            Instant::now() < deadline,
            "host did not publish required observation"
        );
        tokio::time::sleep(Duration::from_millis(/*millis*/ 5)).await;
    }
}

/// A real host SIGKILL leaves the original guardian alive; recovery stops it without replacing identity.
#[tokio::test]
async fn host_kill_reconnect_stop_and_close_use_original_attempt() -> TestResult {
    let fixture = Fixture::new()?;
    let mut child = fixture.launch("create")?;
    let original_binding = ready(&fixture, &mut child).await?;
    let lock_inode = fs::metadata(fixture.root.join("host.lock"))?.ino();
    let client = fixture.client();
    let scope = ScopeId::new();
    assert!(matches!(
        client.execute(Operation::CreateScope { scope }).await?,
        Reply::Scope(_)
    ));
    let sleeper = fixture.directory.path().join("sleep");
    fs::copy("/bin/sleep", &sleeper)?;
    let mut spec = RunSpec::new(
        sleeper,
        fixture.directory.path(),
        DescendantPolicy::WaitForAll,
    );
    spec.args.push("60".into());
    let intent = HostRunIntent {
        scope,
        run: RunId::new(),
        spec,
        host_disconnect: GuardianHostDisconnect::KeepRunning,
    };
    assert!(matches!(
        client
            .execute(Operation::Start {
                intent: intent.clone()
            })
            .await?,
        Reply::Run(_)
    ));
    observed(&client, intent.run, |snapshot| {
        snapshot.direct == DirectProcessState::Running
    })
    .await?;
    let scope_journal = fixture
        .root
        .join("scopes")
        .join(scope.to_string())
        .join("guardian.sqlite");
    let guardian_inode = fs::metadata(&scope_journal)?.ino();
    child.0.kill()?;
    child.0.wait()?;
    let mut recovered = fixture.launch("recover")?;
    let binding = ready(&fixture, &mut recovered).await?;
    assert_eq!(binding.epoch.get(), original_binding.epoch.get() + 1);
    assert_eq!(
        fs::metadata(fixture.root.join("host.lock"))?.ino(),
        lock_inode
    );
    assert_eq!(fs::metadata(scope_journal)?.ino(), guardian_inode);
    let client = fixture.client();
    assert!(matches!(
        client
            .execute(Operation::Start {
                intent: intent.clone()
            })
            .await?,
        Reply::Run(_)
    ));
    assert!(matches!(
        client.execute(Operation::Stop { run: intent.run }).await?,
        Reply::Run(_)
    ));
    assert_eq!(
        observed(&client, intent.run, |snapshot| matches!(
            snapshot.cleanup,
            CleanupState::Complete(_)
        ))
        .await?,
        RunSnapshot {
            id: intent.run,
            launch: LaunchFact::Started,
            direct: DirectProcessState::Exited(ExitOutcome::Signal(libc::SIGKILL)),
            cleanup: CleanupState::Complete(CleanupEvidence::BestEffortComplete)
        }
    );
    assert!(matches!(
        client.execute(Operation::Close { scope }).await?,
        Reply::Scope(_)
    ));
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 10);
    loop {
        if let Reply::Scope(view) = client.execute(Operation::QueryScope { scope }).await?
            && view.last_observed == Some(ScopeState::Closed(CleanupEvidence::BestEffortComplete))
        {
            break;
        }
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(/*millis*/ 5)).await;
    }
    assert!(!fixture.directory.path().join("not-state").exists());
    Ok(())
}

/// Disconnecting before the Start reply does not cancel execution or permit duplicate side effects.
#[tokio::test]
async fn lost_reply_executes_once_and_stalled_output_does_not_block_control() -> TestResult {
    let fixture = Fixture::new()?;
    let mut child = fixture.launch("create")?;
    ready(&fixture, &mut child).await?;
    let client = fixture.client();
    let scope = ScopeId::new();
    client.execute(Operation::CreateScope { scope }).await?;
    let mut spec = RunSpec::new(
        "/bin/sh",
        fixture.directory.path(),
        DescendantPolicy::WaitForAll,
    );
    spec.args = ["-c", "printf x >> count; printf output"]
        .map(Into::into)
        .to_vec();
    spec.output = ora_process_protocol::OutputPolicy::Capture {
        stdout_limit: 4,
        stderr_limit: 4,
    };
    let intent = HostRunIntent {
        scope,
        run: RunId::new(),
        spec,
        host_disconnect: GuardianHostDisconnect::KeepRunning,
    };
    let mut stream = tokio::net::UnixStream::connect(fixture.root.join("host.sock")).await?;
    stream
        .write_all(&ora_process_protocol::encode_guardian_frame(
            &ora_process_protocol::HostRequest {
                version: ora_process_protocol::HOST_WIRE_VERSION,
                operation: Operation::Start {
                    intent: intent.clone(),
                },
            },
        )?)
        .await?;
    drop(stream);
    let snapshot = observed(&client, intent.run, |snapshot| {
        matches!(snapshot.cleanup, CleanupState::Complete(_))
    })
    .await?;
    assert_eq!(
        snapshot.direct,
        DirectProcessState::Exited(ExitOutcome::Code(0))
    );
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 10);
    loop {
        let output = client
            .execute(Operation::Output {
                run: intent.run,
                stream: ora_process_protocol::OutputStream::Stdout,
                offset: 0,
                max_bytes: 4096,
            })
            .await?;
        if matches!(&output, Reply::Output { output, .. } if output.state == ora_process_protocol::OutputState::Eof)
        {
            assert_eq!(
                output,
                Reply::Output {
                    run: intent.run,
                    stream: ora_process_protocol::OutputStream::Stdout,
                    offset: 0,
                    output: ora_process_protocol::OutputRead {
                        bytes: b"outp".to_vec(),
                        retained: 4,
                        truncated: true,
                        state: ora_process_protocol::OutputState::Eof
                    }
                }
            );
            break;
        }
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(/*millis*/ 5)).await;
    }
    let mut stalled = Vec::new();
    for _ in 0..16 {
        let mut stream = tokio::net::UnixStream::connect(fixture.root.join("host-io.sock")).await?;
        stream.write_all(&[0]).await?;
        stalled.push(stream);
    }
    assert!(matches!(
        tokio::time::timeout(
            Duration::from_secs(/*secs*/ 2),
            client.execute(Operation::Start { intent })
        )
        .await??,
        Reply::Run(_)
    ));
    assert_eq!(fs::read(fixture.directory.path().join("count"))?, b"x");
    Ok(())
}

/// A contender cannot replace live endpoints; recovery preserves foreign files at recognized names.
#[tokio::test]
async fn endpoint_ownership_and_unknown_file_preservation() -> TestResult {
    let fixture = Fixture::new()?;
    let mut child = fixture.launch("create")?;
    ready(&fixture, &mut child).await?;
    let original_inode = fs::metadata(fixture.root.join("host.sock"))?.ino();
    let mut contender = fixture.launch("recover")?;
    let status = contender.0.wait()?;
    assert!(!status.success());
    assert_eq!(
        fs::metadata(fixture.root.join("host.sock"))?.ino(),
        original_inode
    );
    assert!(matches!(
        fixture.client().execute(Operation::Inspect).await?,
        Reply::Ready(_)
    ));
    child.0.kill()?;
    child.0.wait()?;
    let endpoint = fixture.root.join("host.sock");
    fs::rename(&endpoint, fixture.directory.path().join("old-socket"))?;
    fs::write(&endpoint, b"preserve user file")?;
    fs::set_permissions(&endpoint, fs::Permissions::from_mode(/*mode*/ 0o600))?;
    let database = fs::read(fixture.root.join("host.sqlite"))?;
    let mut rejected = fixture.launch("recover")?;
    assert!(!rejected.0.wait()?.success());
    assert_eq!(fs::read(&endpoint)?, b"preserve user file");
    assert_eq!(fs::read(fixture.root.join("host.sqlite"))?, database);
    Ok(())
}

/// Peer mismatch and incompatible framing fail before any new responsibility is admitted.
#[tokio::test]
async fn incompatible_requests_and_wrong_peer_are_rejected() -> TestResult {
    use tokio::io::AsyncReadExt;
    let fixture = Fixture::new()?;
    let mut child = fixture.launch("create")?;
    ready(&fixture, &mut child).await?;
    // SAFETY: geteuid only queries the fixture identity; the client expects a deliberately different UID.
    let incorrect = ProcessHost::new(
        fixture.root.clone(),
        unsafe { libc::geteuid() }.wrapping_add(1),
    );
    assert_eq!(
        incorrect
            .execute(Operation::Inspect)
            .await
            .err()
            .ok_or("wrong UID accepted")?
            .kind(),
        std::io::ErrorKind::PermissionDenied
    );
    let scope = ScopeId::new();
    let mut stream = tokio::net::UnixStream::connect(fixture.root.join("host.sock")).await?;
    stream
        .write_all(&ora_process_protocol::encode_guardian_frame(
            &ora_process_protocol::HostRequest {
                version: ora_process_protocol::HOST_WIRE_VERSION + 1,
                operation: Operation::CreateScope { scope },
            },
        )?)
        .await?;
    let length = stream.read_u32().await? as usize;
    assert!(length <= ora_process_protocol::GUARDIAN_MAX_FRAME);
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes).await?;
    assert_eq!(
        ora_process_protocol::decode_guardian_payload::<Reply>(&bytes)?,
        Reply::Rejected(ora_process_protocol::HostRejection::IncompatibleVersion)
    );
    assert_eq!(
        fixture
            .client()
            .execute(Operation::QueryScope { scope })
            .await?,
        Reply::Rejected(ora_process_protocol::HostRejection::UnknownIdentity)
    );
    let mut oversized = tokio::net::UnixStream::connect(fixture.root.join("host.sock")).await?;
    oversized
        .write_all(&((ora_process_protocol::GUARDIAN_MAX_FRAME + 1) as u32).to_be_bytes())
        .await?;
    assert_eq!(oversized.read(&mut [0]).await?, 0);
    assert!(matches!(
        fixture.client().execute(Operation::Inspect).await?,
        Reply::Ready(_)
    ));
    Ok(())
}
