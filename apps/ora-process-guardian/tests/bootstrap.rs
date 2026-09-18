#![cfg(target_os = "linux")]

#[path = "bootstrap/host.rs"]
mod host;
#[path = "bootstrap/management.rs"]
mod management;
#[path = "bootstrap/runs.rs"]
mod runs;

use ora_process_client::GuardianProbe;
use ora_process_protocol::{
    GUARDIAN_WIRE_VERSION, GuardianAccess, GuardianBootstrap, GuardianChannel, ScopeId,
    encode_guardian_frame,
};
use ora_process_runtime::{HostState, ProcessStateError};
use ora_utils::fs::LinuxFileLock;
use ora_utils::process::{
    LinuxPidFd, ProcessSignal, configure_linux_detached_child, linux_process_snapshot,
};
use pretty_assertions::assert_eq;
use std::fs;
use std::io::Write;
use std::os::fd::OwnedFd;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Observes actual lock availability after concurrent test forks finish their close-on-exec step.
async fn recover(root: &Path) -> Result<HostState, ProcessStateError> {
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 10);
    loop {
        match HostState::recover(root) {
            Err(ProcessStateError::Io(error))
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                tokio::time::sleep(Duration::from_millis(/*millis*/ 5)).await;
            }
            result => return result,
        }
    }
}

/// Exercises rejection through the real app's inherited descriptor/bootstrap boundary.
#[test]
fn invalid_bootstrap_preserves_scope_and_never_publishes_endpoints() -> TestResult {
    let fixture = Fixture::new()?;
    for case in [
        "unlocked",
        "reopened",
        "wrong-inode",
        "old-journal",
        "version",
    ] {
        let mut host = HostState::create(&fixture.directory.path().join(case))?;
        let scope = ScopeId::new();
        let intent = host.record_scope_intent(scope)?;
        let scope_dir = fixture.directory.path().join(scope.to_string());
        fs::create_dir(&scope_dir)?;
        fs::set_permissions(&scope_dir, fs::Permissions::from_mode(/*mode*/ 0o700))?;
        let lock_path = scope_dir.join("guardian.lock");
        let file = fs::File::create(&lock_path)?;
        fs::set_permissions(&lock_path, fs::Permissions::from_mode(/*mode*/ 0o600))?;
        let held = if case == "unlocked" {
            None
        } else {
            Some(LinuxFileLock::try_acquire(file)?)
        };
        let inherited = match case {
            "unlocked" | "reopened" => fs::File::open(&lock_path)?,
            "wrong-inode" => LinuxFileLock::try_acquire(fs::File::create(
                fixture.directory.path().join("other.lock"),
            )?)?
            .into_file(),
            "old-journal" | "version" => held
                .as_ref()
                .ok_or("missing held lock")?
                .try_clone()?
                .into_file(),
            _ => unreachable!(),
        };
        if case == "old-journal" {
            fs::write(
                scope_dir.join("guardian.sqlite"),
                b"preserve original journal",
            )?;
        }
        let (mut parent, child) = std::os::unix::net::UnixStream::pair()?;
        let mut command = Command::new(&fixture.executable);
        command
            .arg("--bootstrap")
            .env_clear()
            .stdin(Stdio::from(inherited))
            .stdout(Stdio::from(OwnedFd::from(child)))
            .stderr(Stdio::null());
        configure_linux_detached_child(&mut command);
        let mut child = ChildGuard(command.spawn()?);
        drop(command);
        let frame = encode_guardian_frame(&GuardianBootstrap {
            version: if case == "version" {
                GUARDIAN_WIRE_VERSION + 1
            } else {
                GUARDIAN_WIRE_VERSION
            },
            access: GuardianAccess {
                scope_dir: scope_dir.clone(),
                intent,
            },
        })?;
        // Invalid qualification may close the socket before the parent finishes delivery.
        let _ = parent.write_all(&frame);
        drop(parent);
        let deadline = Instant::now() + Duration::from_secs(/*secs*/ 10);
        let status = loop {
            if let Some(status) = child.0.try_wait()? {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "invalid bootstrap did not terminate"
            );
            std::thread::sleep(Duration::from_millis(/*millis*/ 5));
        };
        assert!(!status.success(), "invalid case {case} was accepted");
        assert!(!scope_dir.join("control.sock").exists());
        if case == "old-journal" {
            assert_eq!(
                fs::read(scope_dir.join("guardian.sqlite"))?,
                b"preserve original journal"
            );
        } else {
            assert!(!scope_dir.join("guardian.sqlite").exists());
        }
    }
    Ok(())
}

struct Fixture {
    directory: tempfile::TempDir,
    executable: PathBuf,
}

impl Fixture {
    /// Copies the real app into a private trusted deployment, independent of checkout permissions.
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let parent = PathBuf::from(std::env::var_os("HOME").ok_or("missing fixture HOME")?)
            .canonicalize()?;
        let directory = tempfile::Builder::new()
            .prefix("g")
            .permissions(fs::Permissions::from_mode(/*mode*/ 0o700))
            .tempdir_in(parent)?;
        let executable = directory.path().join("guardian");
        fs::copy(env!("CARGO_BIN_EXE_ora-process-guardian"), &executable)?;
        fs::set_permissions(&executable, fs::Permissions::from_mode(/*mode*/ 0o700))?;
        Ok(Self {
            directory,
            executable,
        })
    }
}

impl Drop for Fixture {
    /// Cleans only processes executing this fixture's unique binary, through revalidated pidfds.
    fn drop(&mut self) {
        if let Ok(snapshot) = linux_process_snapshot() {
            for stat in snapshot.flatten() {
                let executable = Path::new("/proc").join(stat.pid.to_string()).join("exe");
                if fs::read_link(executable).ok().is_some_and(|path| {
                    path == self.executable || path == self.directory.path().join("sleep")
                }) && let Ok(handle) = LinuxPidFd::from_observation(&stat)
                {
                    let _ = handle.signal(ProcessSignal::Kill);
                }
            }
        }
    }
}

/// Waits for authenticated readiness, not for a guessed startup delay.
async fn ready(access: &GuardianAccess) -> Result<GuardianProbe, Box<dyn std::error::Error>> {
    // SAFETY: geteuid queries the test process's current identity.
    let probe = GuardianProbe::new(access.clone(), unsafe { libc::geteuid() });
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 10);
    loop {
        if probe.ready(GuardianChannel::Control).await.is_ok() {
            return Ok(probe);
        }
        if Instant::now() >= deadline {
            return Err("guardian failed to become Ready".into());
        }
        tokio::time::sleep(Duration::from_millis(/*millis*/ 5)).await;
    }
}

/// Identifies the live socket owner for test assertions; this does not expose numeric signal authority.
async fn guardian_pid(access: &GuardianAccess) -> Result<u32, Box<dyn std::error::Error>> {
    Ok(
        tokio::net::UnixStream::connect(access.scope_dir.join("control.sock"))
            .await?
            .peer_cred()?
            .pid()
            .ok_or("missing peer pid")?
            .try_into()?,
    )
}

/// The real app owns its journal, lock and three authenticated readiness endpoints in a new session.
#[tokio::test]
async fn guardian_commits_before_ready_and_rejects_unauthorized_probes() -> TestResult {
    let fixture = Fixture::new()?;
    let root = fixture.directory.path().join("s");
    let mut host = HostState::create(&root)?;
    let scope = ScopeId::new();
    let intent = host.record_scope_intent(scope)?;
    let access = host.start_guardian(scope, &fixture.executable).await?;
    let probe = ready(&access).await?;
    let first = probe.ready(GuardianChannel::Control).await?;
    assert_eq!(first.intent, intent);
    for channel in [GuardianChannel::Events, GuardianChannel::Io] {
        let mut expected = first.clone();
        expected.channel = channel;
        assert_eq!(probe.ready(channel).await?, expected);
    }
    let pid = guardian_pid(&access).await?;
    let stat = linux_process_snapshot()?
        .filter_map(Result::ok)
        .find(|stat| stat.pid == pid)
        .ok_or("missing guardian process")?;
    assert_eq!(stat.session, pid);
    assert!(
        LinuxFileLock::try_acquire(fs::File::open(access.scope_dir.join("guardian.lock"))?)
            .is_err()
    );
    let journal = rusqlite::Connection::open_with_flags(
        access.scope_dir.join("guardian.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let record = journal.query_row(
        "SELECT scope, guardian, phase FROM guardian_bootstrap",
        [],
        |row| {
            Ok((
                row.get::<_, String>(/*idx*/ 0)?,
                row.get::<_, String>(/*idx*/ 1)?,
                row.get::<_, String>(/*idx*/ 2)?,
            ))
        },
    )?;
    assert_eq!(
        record,
        (
            scope.to_string(),
            intent.guardian.to_string(),
            "initialized".to_owned()
        )
    );
    let mut wrong = access.clone();
    wrong.intent.guardian = ora_process_protocol::GuardianInstanceId::new();
    // SAFETY: geteuid only reads the current identity.
    let owner = unsafe { libc::geteuid() };
    assert!(
        GuardianProbe::new(wrong, owner)
            .ready(GuardianChannel::Control)
            .await
            .is_err()
    );
    assert!(
        GuardianProbe::new(access.clone(), owner.wrapping_add(1))
            .ready(GuardianChannel::Control)
            .await
            .is_err()
    );
    assert!(
        host.start_guardian(scope, &fixture.executable)
            .await
            .is_err()
    );
    assert_eq!(host.guardian_access(scope)?, Some(access));
    Ok(())
}

/// Broken executable errors remain consumed attempts across restart, never automatic retries.
#[tokio::test]
async fn failed_exec_is_not_retried_after_recovery() -> TestResult {
    let fixture = Fixture::new()?;
    let root = fixture.directory.path().join("s");
    let broken = fixture.directory.path().join("broken");
    fs::write(&broken, b"not executable")?;
    fs::set_permissions(&broken, fs::Permissions::from_mode(/*mode*/ 0o600))?;
    let mut host = HostState::create(&root)?;
    let scope = ScopeId::new();
    host.record_scope_intent(scope)?;
    assert!(host.start_guardian(scope, &broken).await.is_err());
    let original = host
        .guardian_access(scope)?
        .ok_or("missing uncertain attempt")?;
    drop(host);
    let mut host = recover(&root).await?;
    assert!(
        host.start_guardian(scope, &fixture.executable)
            .await
            .is_err()
    );
    assert_eq!(host.guardian_access(scope)?, Some(original.clone()));
    assert!(!original.scope_dir.join("guardian.sqlite").exists());
    Ok(())
}

/// Saturating the I/O handshake workers cannot consume the control channel's worker slots.
#[tokio::test]
async fn stalled_io_probes_do_not_block_control() -> TestResult {
    let fixture = Fixture::new()?;
    let mut host = HostState::create(&fixture.directory.path().join("s"))?;
    let scope = ScopeId::new();
    host.record_scope_intent(scope)?;
    let access = host.start_guardian(scope, &fixture.executable).await?;
    let probe = ready(&access).await?;
    let mut stalled = Vec::new();
    for _ in 0..20 {
        stalled.push(tokio::net::UnixStream::connect(access.scope_dir.join("io.sock")).await?);
    }
    tokio::time::timeout(
        Duration::from_secs(/*secs*/ 2),
        probe.ready(GuardianChannel::Control),
    )
    .await??;
    drop(stalled);
    Ok(())
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    /// Reaps the explicitly owned launcher even if a crash assertion fails.
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Killing the launcher does not kill the real guardian or change the original discovery identity.
#[tokio::test]
async fn launcher_kill_preserves_original_guardian_and_lost_ready_is_queryable() -> TestResult {
    let fixture = Fixture::new()?;
    let root = fixture.directory.path().join("s");
    let marker = fixture.directory.path().join("ready");
    let scope = ScopeId::new();
    let run = ora_process_protocol::RunId::new();
    let workload = runs::sleeper(&fixture)?;
    let mut launcher = ChildGuard(
        Command::new(std::env::current_exe()?)
            .args(["--exact", "launcher_fixture", "--nocapture"])
            .env("ORA_GUARDIAN_FIXTURE_ROOT", &root)
            .env("ORA_GUARDIAN_FIXTURE_PROGRAM", &fixture.executable)
            .env("ORA_GUARDIAN_FIXTURE_MARKER", &marker)
            .env("ORA_GUARDIAN_FIXTURE_SCOPE", scope.to_string())
            .env("ORA_GUARDIAN_FIXTURE_WORKLOAD", &workload.program)
            .env("ORA_GUARDIAN_FIXTURE_RUN", run.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()?,
    );
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 10);
    while !marker.exists() {
        assert!(
            launcher.0.try_wait()?.is_none(),
            "launcher exited before Ready"
        );
        assert!(
            Instant::now() < deadline,
            "launcher failed to observe Ready"
        );
        tokio::time::sleep(Duration::from_millis(/*millis*/ 5)).await;
    }
    let pid: u32 = fs::read_to_string(&marker)?.parse()?;
    let inode = fs::metadata(
        root.join("scopes")
            .join(scope.to_string())
            .join("guardian.lock"),
    )?
    .ino();
    launcher.0.kill()?;
    launcher.0.wait()?;
    let mut recovered = recover(&root).await?;
    // Discover the original Run and Scope through the host ledger, not a surviving caller handle.
    let intents = recovered.run_intents()?;
    let [intent] = intents.as_slice() else {
        return Err("expected one durable Run intent".into());
    };
    let mut expected_spec = workload;
    expected_spec.cwd = root.clone();
    assert_eq!(
        intent,
        &ora_process_protocol::HostRunIntent {
            scope,
            run,
            spec: expected_spec,
            host_disconnect: ora_process_protocol::GuardianHostDisconnect::KeepRunning,
        }
    );
    let run = intent.run;
    let access = recovered
        .guardian_access(intent.scope)?
        .ok_or("lost guardian responsibility")?;
    ready(&access).await?;
    assert_eq!(guardian_pid(&access).await?, pid);
    // The accepted workload survives external host SIGKILL, not just a dropped client socket.
    let client = ora_process_client::GuardianRuns::new(
        access.clone(),
        unsafe { libc::geteuid() },
        runs::bind(&access, recovered.binding()).await?,
    );
    assert_eq!(
        client
            .execute(ora_process_protocol::GuardianRunOperation::Query { run })
            .await?,
        ora_process_protocol::GuardianRunResult::Run(ora_process_protocol::RunSnapshot {
            id: run,
            launch: ora_process_protocol::LaunchFact::Started,
            direct: ora_process_protocol::DirectProcessState::Running,
            cleanup: ora_process_protocol::CleanupState::Pending,
        })
    );
    client
        .execute(ora_process_protocol::GuardianRunOperation::Stop { run })
        .await?;
    runs::completed(&client, run).await?;
    // Actual external launcher death advances only host authority, never the guardian identity.
    let manager =
        ora_process_client::GuardianManagement::new(access.clone(), unsafe { libc::geteuid() });
    assert!(
        matches!(manager.bind(recovered.binding()).await?, ora_process_protocol::GuardianManagementReply::Bound { session, .. } if session.host == recovered.binding())
    );
    assert_eq!(
        manager.bind(access.intent.created_by).await?,
        ora_process_protocol::GuardianManagementReply::Rejected(
            ora_process_protocol::GuardianManagementRejection::StaleHost
        )
    );
    assert_eq!(
        fs::metadata(access.scope_dir.join("guardian.lock"))?.ino(),
        inode
    );
    assert!(
        recovered
            .start_guardian(scope, &fixture.executable)
            .await
            .is_err()
    );
    Ok(())
}

/// The fixture is a launcher using production interfaces, not the future production host app.
#[tokio::test]
async fn launcher_fixture() -> TestResult {
    let Some(root) = std::env::var_os("ORA_GUARDIAN_FIXTURE_ROOT") else {
        return Ok(());
    };
    let executable =
        PathBuf::from(std::env::var_os("ORA_GUARDIAN_FIXTURE_PROGRAM").ok_or("missing program")?);
    let marker =
        PathBuf::from(std::env::var_os("ORA_GUARDIAN_FIXTURE_MARKER").ok_or("missing marker")?);
    let scope = std::env::var("ORA_GUARDIAN_FIXTURE_SCOPE")?.parse()?;
    let mut host = HostState::create(Path::new(&root))?;
    host.record_scope_intent(scope)?;
    let access = host.start_guardian(scope, &executable).await?;
    ready(&access).await?;
    if let Some(workload) = std::env::var_os("ORA_GUARDIAN_FIXTURE_WORKLOAD") {
        let run = std::env::var("ORA_GUARDIAN_FIXTURE_RUN")?.parse()?;
        let mut spec = ora_process_protocol::RunSpec::new(
            workload,
            Path::new(&root),
            ora_process_protocol::DescendantPolicy::WaitForAll,
        );
        spec.args.push("60".into());
        let intent = host.record_run_intent(ora_process_protocol::HostRunIntent {
            scope,
            run,
            spec,
            host_disconnect: ora_process_protocol::GuardianHostDisconnect::KeepRunning,
        })?;
        // SAFETY: geteuid only queries the fixture process's identity.
        let client = ora_process_client::GuardianRuns::new(
            access.clone(),
            unsafe { libc::geteuid() },
            runs::bind(&access, host.binding()).await?,
        );
        assert!(matches!(
            client.execute(intent.start_operation()).await?,
            ora_process_protocol::GuardianRunResult::Run(_)
        ));
    }
    fs::write(
        marker.with_extension("pending"),
        guardian_pid(&access).await?.to_string(),
    )?;
    fs::rename(marker.with_extension("pending"), marker)?;
    tokio::time::sleep(Duration::from_secs(/*secs*/ 20)).await;
    Ok(())
}
