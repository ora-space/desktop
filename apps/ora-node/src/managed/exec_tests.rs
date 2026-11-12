//! Directly run Git against the real executable: deadlines, leftovers and what a result waits for.
use super::*;
use pretty_assertions::assert_eq;
use std::{fs, os::unix::fs::PermissionsExt};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// A private Node home with its lock directory, and a directory for commands to run in.
pub(super) struct Fixture {
    pub(super) directory: tempfile::TempDir,
    pub(super) locks: AttemptLocks,
}

impl Fixture {
    pub(super) fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let directory = tempfile::Builder::new()
            .prefix("nd")
            .permissions(fs::Permissions::from_mode(/*mode*/ 0o700))
            .tempdir()?;
        let locks = AttemptLocks::open(directory.path())?;
        Ok(Self { directory, locks })
    }

    /// The environment is explicit and contains no credentials or process-wide mutations.
    pub(super) fn config(&self) -> ProcessConfig {
        ProcessConfig {
            workload_uid: None,
            host_directory: None,
            expected_uid: None,
            git_program: "/usr/bin/git".into(),
            environment: BTreeMap::new(),
            command_timeout_ms: 60_000,
            network_timeout_ms: 600_000,
            cleanup_timeout_ms: 10_000,
            shutdown_grace_ms: 1000,
        }
    }

    /// An executor over this fixture's locks.
    pub(super) fn git(&self, config: ProcessConfig) -> std::io::Result<DirectGit> {
        DirectGit::new(config, Shutdown::default(), self.locks.clone())
    }

    /// Real Git running `script` through a shell alias, so the command under test is Git itself
    /// with whatever process tree the script builds below it.
    pub(super) fn script(&self, script: &str, intent: GitIntent) -> GitCommand {
        GitCommand::new(
            self.directory.path().to_path_buf(),
            vec![
                "-c".to_owned(),
                format!("alias.script=!{script}"),
                "script".to_owned(),
            ],
            gitlancer::GitEnv::automation_defaults()
                .with_variable("GIT_CONFIG_NOSYSTEM", "1")
                .with_variable("GIT_CONFIG_GLOBAL", "/dev/null")
                .with_variable("PATH", "/usr/bin:/bin"),
            intent,
        )
    }
}

/// Every command runs with background gc and maintenance switched off, and its children see the
/// same settings, so nothing detaches from a fetch holding the attempt lock.
#[test]
fn every_command_disables_background_maintenance() -> TestResult {
    let fixture = Fixture::new()?;
    let git = fixture.git(fixture.config())?;
    let command = fixture.script(
        "git config --get gc.auto; git config --get maintenance.auto",
        GitIntent::Mutating,
    );

    let output = git.run(&command)?;

    assert_eq!(
        (output.stdout.as_str(), fixture.locks.entries()?),
        ("0\nfalse\n", vec![])
    );
    Ok(())
}

/// An expired deadline ends the whole process group, and the caller only gets the timeout back
/// once the attempt lock proved the group gone: by then the lock is already retired.
#[test]
fn a_deadline_ends_the_group_before_the_result_returns() -> TestResult {
    let fixture = Fixture::new()?;
    let mut config = fixture.config();
    config.command_timeout_ms = 300;
    let git = fixture.git(config)?;
    let command = fixture.script(
        "printf started > started; sleep 60 & exec sleep 60",
        GitIntent::Mutating,
    );

    let started = Instant::now();
    let result = git.run(&command);

    assert!(
        matches!(result, Err(GitExecError::OutputReadFailed { source, .. })
        if source.kind() == std::io::ErrorKind::TimedOut)
    );
    assert_eq!(
        (
            fixture.directory.path().join("started").is_file(),
            started.elapsed() < Duration::from_secs(/*secs*/ 10),
            fixture.locks.entries()?,
        ),
        (true, true, vec![])
    );
    Ok(())
}

/// Git exiting 0 is not success while something of its tree may still write. What it left in its
/// group is ended and the result waits for the lock, so a clean exit returns promptly and clean.
#[test]
fn a_clean_exit_ends_what_git_left_in_its_group() -> TestResult {
    let fixture = Fixture::new()?;
    let git = fixture.git(fixture.config())?;
    let command = fixture.script("sleep 60 >/dev/null 2>&1 & echo done", GitIntent::Mutating);

    let started = Instant::now();
    let output = git.run(&command)?;

    assert_eq!(
        (
            output.stdout.as_str(),
            started.elapsed() < Duration::from_secs(/*secs*/ 10),
            fixture.locks.entries()?,
        ),
        ("done\n", true, vec![])
    );
    Ok(())
}

/// A descendant that left the group outlives the kill and keeps the lock. The script pauses so
/// the descendant has left before Git exits and its group is ended. The exit code 0 is then not
/// reported: the command fails as unverified, its lock stays, and no further unrecorded
/// command starts until the descendant is gone.
#[test]
fn a_descendant_outside_the_group_blocks_the_next_command() -> TestResult {
    let fixture = Fixture::new()?;
    let mut config = fixture.config();
    config.cleanup_timeout_ms = 300;
    let git = fixture.git(config)?;
    let escaping = fixture.script(
        "setsid sleep 2 >/dev/null 2>&1 & sleep 0.3; exit 0",
        GitIntent::Mutating,
    );
    let next = fixture.script("echo next", GitIntent::Mutating);

    let unverified = git.run(&escaping);
    let leftovers = git.leftover_directories();
    let blocked = git.run(&next);
    std::thread::sleep(Duration::from_secs(/*secs*/ 3));
    let after = git.run(&next)?;

    assert!(matches!(
        unverified,
        Err(GitExecError::OutputReadFailed { .. })
    ));
    assert!(matches!(
        blocked,
        Err(GitExecError::OutputReadFailed { .. })
    ));
    assert_eq!(
        (leftovers, after.stdout.as_str(), fixture.locks.entries()?),
        (
            vec![fixture.directory.path().to_path_buf()],
            "next\n",
            vec![]
        )
    );
    Ok(())
}

/// Output beyond the caller's limit ends the command instead of handing a parser a prefix.
#[test]
fn output_beyond_the_limit_fails_the_command() -> TestResult {
    let fixture = Fixture::new()?;
    let git = fixture.git(fixture.config())?;
    let command = fixture.script("head -c 5000 /dev/zero; exec sleep 60", GitIntent::ReadOnly);

    let started = Instant::now();
    let result = git.run_bounded(&command, /*stdout*/ 1024, /*stderr*/ 1024);

    assert!(matches!(result, Err(GitExecError::OutputReadFailed { .. })));
    assert!(started.elapsed() < Duration::from_secs(/*secs*/ 10));
    Ok(())
}

/// A recorded attempt is settled by how Git ended, and by nothing else: its own exit code, the
/// signal that cut it at its deadline, or no exit status at all when it never started.
#[test]
fn a_dispatched_attempt_settles_by_how_git_ended() -> TestResult {
    let fixture = Fixture::new()?;
    let mut config = fixture.config();
    config.network_timeout_ms = 300;
    let git = fixture.git(config.clone())?;
    config.git_program = "/nonexistent/git".into();
    let missing = fixture.git(config)?;
    let mut settlements = Vec::new();
    for (executor, script) in [
        (&git, "exit 3"),
        (&git, "exec sleep 60"),
        (&missing, "exit 0"),
    ] {
        let attempt = ProcessAttempt {
            execution: ExecutionId::new("execution"),
            id: AttemptId::generate(),
        };
        let command = fixture.script(script, GitIntent::Mutating);
        fixture
            .locks
            .create(&attempt.id, &command.cwd, LockDurability::Recorded)?;
        settlements.push(executor.dispatch(&attempt, &command));
        // Recovery reads the same evidence from the lock the attempt left behind.
        settlements.push(executor.settle(&attempt));
    }

    assert_eq!(
        settlements,
        vec![
            AttemptSettlement::Exited(3),
            AttemptSettlement::Stopped,
            AttemptSettlement::Terminated(libc::SIGKILL),
            AttemptSettlement::Stopped,
            AttemptSettlement::Stopped,
            AttemptSettlement::Stopped,
        ]
    );
    Ok(())
}

/// Opt-in proof on a root host: Git dropped to the workload user still holds its attempt lock
/// through the inherited descriptor, while that user cannot open the lock file to hold an
/// attempt Unknown on its own.
#[test]
#[ignore = "requires root; run the built test binary with `sudo -n <binary> --ignored workload_git`"]
fn workload_git_inherits_a_lock_it_cannot_open() -> TestResult {
    let fixture = Fixture::new()?;
    let mut config = fixture.config();
    config.workload_uid = Some(65534);
    let git = fixture.git(config.clone())?;
    let attempt = AttemptId::generate();
    let lock = fixture
        .directory
        .path()
        .join("git-attempts")
        .join(format!("{attempt}.lock"));
    // The private fixture directory is closed to the workload user, so the command runs from
    // the root directory. It reports its user, whether it can read the lock file by name, and
    // then stays alive long enough for the lock to be probed.
    let mut command = fixture.script(
        &format!(
            "id -u; cat '{}' >/dev/null 2>&1; echo $?; sleep 1",
            lock.display()
        ),
        GitIntent::Mutating,
    );
    command.cwd = "/".into();
    fixture
        .locks
        .create(&attempt, &command.cwd, LockDurability::Recorded)?;

    let (finished, held_while_running) = std::thread::scope(|scope| {
        let running = scope.spawn(|| {
            git.runtime.block_on(exec::run(
                &config,
                &fixture.locks,
                &Shutdown::default(),
                Invocation {
                    command: &command,
                    environment: repository::clone_environment(&command),
                    identity: RunAs::Workload,
                    capture: Capture::Bounded {
                        stdout: 1024,
                        stderr: 1024,
                    },
                    deadline: Instant::now() + Duration::from_secs(/*secs*/ 30),
                    stop_grace: StopGrace::Honored,
                    before_cut: None,
                    lock: Some(&attempt),
                },
            ))
        });
        // The Node's own share of the lock is dropped right after the spawn, so a lock still
        // held half a second into the command is held by the workload's Git alone.
        std::thread::sleep(Duration::from_millis(/*millis*/ 500));
        let held = fixture.locks.probe(&attempt);
        (running.join(), held)
    });
    let finished = finished.map_err(|_| "the command thread panicked")?;

    assert_eq!(
        (
            finished.output?.stdout.as_str(),
            finished.settlement,
            held_while_running,
            fixture.locks.probe(&attempt),
        ),
        (
            "65534\n1\n",
            AttemptSettlement::Exited(0),
            StopEvidence::Running,
            StopEvidence::Stopped,
        )
    );
    Ok(())
}
