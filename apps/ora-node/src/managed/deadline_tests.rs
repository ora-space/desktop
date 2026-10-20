//! Native deadline evidence uses the production client, host, guardian and Git executable.
use super::*;
use crate::revision::RestoreGitRunner;
use ora_utils::process::{LinuxPidFd, ProcessSignal, linux_process_snapshot};
use pretty_assertions::assert_eq;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Child, Command, Stdio},
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct Fixture {
    directory: tempfile::TempDir,
    host: Child,
}
impl Fixture {
    /// Builds require `cargo build -p ora-process-host -p ora-process-guardian` first.
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let parent = PathBuf::from(std::env::var_os("HOME").ok_or("fixture HOME missing")?)
            .canonicalize()?;
        let directory = tempfile::Builder::new()
            .prefix("nd")
            .permissions(fs::Permissions::from_mode(/*mode*/ 0o700))
            .tempdir_in(parent)?;
        let binaries = std::env::current_exe()?
            .parent()
            .and_then(Path::parent)
            .ok_or("missing executable directory")?
            .to_path_buf();
        let guardian = directory.path().join("guardian");
        fs::copy(binaries.join("ora-process-guardian"), &guardian)?;
        fs::set_permissions(&guardian, fs::Permissions::from_mode(/*mode*/ 0o700))?;
        let host = Command::new(binaries.join("ora-process-host"))
            .arg("create")
            .arg(directory.path().join("s"))
            .arg(&guardian)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()?;
        Ok(Self { directory, host })
    }

    /// The environment is explicit and contains no credentials or process-wide mutations.
    fn config(&self) -> ProcessConfig {
        ProcessConfig {
            workload_uid: None,
            host_directory: self.directory.path().join("s"),
            // SAFETY: geteuid only reads this test's identity.
            expected_uid: unsafe { libc::geteuid() },
            git_program: "/usr/bin/git".into(),
            environment: BTreeMap::new(),
            command_timeout_ms: 60_000,
            network_timeout_ms: 600_000,
            cleanup_timeout_ms: 10_000,
            shutdown_grace_ms: 1000,
        }
    }
}
impl Drop for Fixture {
    /// Only this fixture's pinned guardian executables and directly owned host are stopped.
    fn drop(&mut self) {
        let _ = self.host.kill();
        let _ = self.host.wait();
        if let Ok(processes) = linux_process_snapshot() {
            for process in processes.flatten() {
                if fs::read_link(Path::new("/proc").join(process.pid.to_string()).join("exe"))
                    .ok()
                    .as_ref()
                    == Some(&self.directory.path().join("guardian"))
                    && let Ok(handle) = LinuxPidFd::from_observation(&process)
                {
                    let _ = handle.signal(ProcessSignal::Kill);
                }
            }
        }
    }
}

/// An in-flight exchange cannot consume its ordinary five-second IPC timeout after restore expiry.
#[tokio::test]
async fn restore_cutoff_bounds_a_stalled_host_exchange() -> TestResult {
    let directory = tempfile::tempdir()?;
    let listener = tokio::net::UnixListener::bind(directory.path().join("host.sock"))?;
    let server = tokio::spawn(async move {
        let (_connection, _) = listener.accept().await?;
        std::future::pending::<std::io::Result<()>>().await
    });
    let config = ProcessConfig {
        workload_uid: None,
        host_directory: directory.path().to_path_buf(),
        // SAFETY: geteuid only reads this test's identity.
        expected_uid: unsafe { libc::geteuid() },
        git_program: "/usr/bin/git".into(),
        environment: BTreeMap::new(),
        command_timeout_ms: 60_000,
        network_timeout_ms: 600_000,
        cleanup_timeout_ms: 10_000,
        shutdown_grace_ms: 1000,
    };
    let client = ProcessHost::new(config.host_directory.clone(), config.expected_uid);
    let intent = HostRunIntent {
        scope: ScopeId::new(),
        run: RunId::new(),
        spec: RunSpec::new(
            "/usr/bin/git",
            directory.path(),
            DescendantPolicy::WaitForAll,
        ),
        host_disconnect: GuardianHostDisconnect::KeepRunning,
    };
    let mut config = config;
    config.command_timeout_ms = 100;
    for (kind, restore_budget) in [
        (GitIntent::Network, Duration::from_millis(/*millis*/ 100)),
        (GitIntent::ReadOnly, Duration::from_secs(/*secs*/ 10)),
    ] {
        let started = Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(/*secs*/ 1),
            transport::execute_before(
                &client,
                &intent,
                kind,
                &config,
                &Shutdown::default(),
                started + restore_budget,
            ),
        )
        .await?;
        assert_eq!(
            result.err().ok_or("expected an execution timeout")?.kind(),
            std::io::ErrorKind::TimedOut
        );
    }
    server.abort();
    Ok(())
}

/// Real Git is interrupted at the restore cap; returning still waits for original closure evidence.
#[test]
fn restore_cutoff_closes_real_git_with_a_longer_network_budget() -> TestResult {
    let fixture = Fixture::new()?;
    let config = fixture.config();
    let client = ProcessHost::new(config.host_directory.clone(), config.expected_uid);
    let runner = CloneHost::new(config, Shutdown::default(), RunLifetime::Independent)?;
    runner.runtime.block_on(async {
        let deadline = Instant::now() + Duration::from_secs(/*secs*/ 15);
        while !matches!(
            client.execute(HostOperation::Inspect).await,
            Ok(HostReply::Ready(_))
        ) {
            assert!(Instant::now() < deadline, "host did not become ready");
            tokio::time::sleep(Duration::from_millis(/*millis*/ 10)).await;
        }
    });
    let command = GitCommand::new(
        fixture.directory.path().to_path_buf(),
        [
            "-c",
            "alias.pause=!printf started > started; exec sleep 60",
            "pause",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        gitlancer::GitEnv::automation_defaults()
            .with_variable("GIT_CONFIG_NOSYSTEM", "1")
            .with_variable("GIT_CONFIG_GLOBAL", "/dev/null"),
        GitIntent::Network,
    );
    let started = Instant::now();
    let result = runner.run_bounded_before(
        &command,
        /*stdout*/ 1024,
        /*stderr*/ 1024,
        started + Duration::from_secs(/*secs*/ 15),
    );
    assert!(
        matches!(result, Err(GitExecError::OutputReadFailed { source, .. })
        if source.kind() == std::io::ErrorKind::TimedOut)
    );
    assert!(
        fixture.directory.path().join("started").is_file(),
        "Git never reached its live wait"
    );
    assert!(started.elapsed() < Duration::from_secs(/*secs*/ 30));
    let database = rusqlite::Connection::open_with_flags(
        fixture.directory.path().join("s").join("host.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let (scope, run): (String, String) =
        database.query_row("SELECT scope,run FROM run_intents", [], |row| {
            Ok((row.get(/*idx*/ 0)?, row.get(/*idx*/ 1)?))
        })?;
    let scope = runner
        .runtime
        .block_on(client.execute(HostOperation::QueryScope {
            scope: scope.parse()?,
        }))?;
    let run = runner
        .runtime
        .block_on(client.execute(HostOperation::QueryRun { run: run.parse()? }))?;
    assert!(
        matches!(scope, HostReply::Scope(view) if matches!(view.last_observed, Some(ScopeState::Closed(_))))
    );
    assert!(
        matches!(run, HostReply::Run(view) if view.last_observed.as_ref().is_some_and(|snapshot|
        matches!(snapshot.direct, DirectProcessState::Exited(_)) && matches!(snapshot.cleanup, CleanupState::Complete(_))))
    );
    Ok(())
}
