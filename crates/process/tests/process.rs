use std::io;
use std::process::ExitStatus;
use std::time::Duration;

use ora_process::{ManagedProcess, ProcessSpawner, ProcessSpec, ProcessStdio, TokioProcessSpawner};
use pretty_assertions::assert_eq;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn process_spec_preserves_command_options_and_defaults() {
    let cwd = std::path::PathBuf::from("worktree");
    let spec = ProcessSpec::new("bun")
        .arg("run")
        .args(["main.ts", "--verbose"])
        .cwd(cwd.clone())
        .env("ORA_ENV", "test")
        .stdin(ProcessStdio::Inherit)
        .stderr(ProcessStdio::Null)
        .keep_alive_on_drop();

    assert_eq!(spec.program(), std::ffi::OsStr::new("bun"));
    assert_eq!(
        spec.args_iter().collect::<Vec<_>>(),
        vec![
            std::ffi::OsStr::new("run"),
            std::ffi::OsStr::new("main.ts"),
            std::ffi::OsStr::new("--verbose"),
        ]
    );
    assert_eq!(spec.cwd_path(), Some(cwd.as_path()));
    assert_eq!(
        spec.envs().collect::<Vec<_>>(),
        vec![(
            std::ffi::OsStr::new("ORA_ENV"),
            std::ffi::OsStr::new("test")
        )]
    );
    assert_eq!(spec.stdin_policy(), ProcessStdio::Inherit);
    assert_eq!(spec.stdout_policy(), ProcessStdio::Piped);
    assert_eq!(spec.stderr_policy(), ProcessStdio::Null);
    assert_eq!(spec.should_kill_on_drop(), false);
}

#[test]
fn process_spawner_trait_allows_fake_processes() {
    let spawner = FakeSpawner;
    let process = spawn_with(&spawner, ProcessSpec::new("fake"))
        .unwrap_or_else(|error| panic!("expected fake process spawn to succeed: {error}"));

    assert_eq!(process.id(), Some(42));
}

#[tokio::test]
async fn spawns_process_from_spec_and_reads_stdout_and_stderr() {
    let spawner = TokioProcessSpawner::new();
    let mut process = spawner
        .spawn(shell_command(
            "echo process-stdout && echo process-stderr 1>&2",
        ))
        .unwrap_or_else(|error| panic!("expected process spawn to succeed: {error}"));
    let mut stdout = process
        .take_stdout()
        .unwrap_or_else(|| panic!("expected stdout pipe"));
    let mut stderr = process
        .take_stderr()
        .unwrap_or_else(|| panic!("expected stderr pipe"));

    let mut output = String::new();
    stdout
        .read_to_string(&mut output)
        .await
        .unwrap_or_else(|error| panic!("expected stdout read to succeed: {error}"));
    let mut error_output = String::new();
    stderr
        .read_to_string(&mut error_output)
        .await
        .unwrap_or_else(|error| panic!("expected stderr read to succeed: {error}"));
    let exit = process
        .wait()
        .await
        .unwrap_or_else(|error| panic!("expected process wait to succeed: {error}"));

    assert!(exit.success());
    assert!(output.contains("process-stdout"));
    assert!(error_output.contains("process-stderr"));
}

#[tokio::test]
async fn applies_cwd_and_env_from_process_spec() {
    let worktree = tempfile::tempdir().unwrap_or_else(|error| panic!("expected tempdir: {error}"));
    let spawner = TokioProcessSpawner::new();
    let mut process = spawner
        .spawn(
            cwd_and_env_command()
                .cwd(worktree.path())
                .env("ORA_PROCESS_TEST_VALUE", "process-env"),
        )
        .unwrap_or_else(|error| panic!("expected process spawn to succeed: {error}"));
    let mut stdout = process
        .take_stdout()
        .unwrap_or_else(|| panic!("expected stdout pipe"));

    let mut output = String::new();
    stdout
        .read_to_string(&mut output)
        .await
        .unwrap_or_else(|error| panic!("expected stdout read to succeed: {error}"));
    let exit = process
        .wait()
        .await
        .unwrap_or_else(|error| panic!("expected process wait to succeed: {error}"));

    assert!(exit.success());
    assert!(output.contains(&worktree.path().display().to_string()));
    assert!(output.contains("process-env"));
}

#[tokio::test]
async fn exposes_stdin_as_an_owned_pipe() {
    let spawner = TokioProcessSpawner::new();
    let mut process = spawner
        .spawn(stdin_echo_command())
        .unwrap_or_else(|error| panic!("expected process spawn to succeed: {error}"));
    let mut stdin = process
        .take_stdin()
        .unwrap_or_else(|| panic!("expected stdin pipe"));
    let mut stdout = process
        .take_stdout()
        .unwrap_or_else(|| panic!("expected stdout pipe"));

    assert!(process.take_stdin().is_none());
    assert!(process.take_stdout().is_none());

    stdin
        .write_all(b"process-stdin\n")
        .await
        .unwrap_or_else(|error| panic!("expected stdin write to succeed: {error}"));
    drop(stdin);

    let mut output = String::new();
    stdout
        .read_to_string(&mut output)
        .await
        .unwrap_or_else(|error| panic!("expected stdout read to succeed: {error}"));
    let exit = process
        .wait()
        .await
        .unwrap_or_else(|error| panic!("expected process wait to succeed: {error}"));

    assert!(exit.success());
    assert!(output.contains("process-stdin"));
}

#[tokio::test]
async fn can_wait_and_kill_without_exclusive_process_access() {
    let spawner = TokioProcessSpawner::new();
    let process = spawner
        .spawn(long_running_command())
        .unwrap_or_else(|error| panic!("expected process spawn to succeed: {error}"));

    assert!(
        process
            .try_wait()
            .unwrap_or_else(|error| panic!("expected try_wait to succeed: {error}"))
            .is_none()
    );

    let wait = process.wait();
    let kill = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        process.kill().await
    };
    let (exit, kill_result) = tokio::join!(wait, kill);

    kill_result.unwrap_or_else(|error| panic!("expected process kill to succeed: {error}"));
    let exit = exit.unwrap_or_else(|error| panic!("expected wait after kill to succeed: {error}"));
    assert!(!exit.success());
}

fn spawn_with<S: ProcessSpawner>(spawner: &S, spec: ProcessSpec) -> io::Result<S::Process> {
    spawner.spawn(spec)
}

struct FakeSpawner;

impl ProcessSpawner for FakeSpawner {
    type Process = FakeProcess;

    fn spawn(&self, _spec: ProcessSpec) -> io::Result<Self::Process> {
        Ok(FakeProcess)
    }
}

struct FakeProcess;

impl ManagedProcess for FakeProcess {
    type Stdin = tokio::io::DuplexStream;
    type Stdout = tokio::io::DuplexStream;
    type Stderr = tokio::io::DuplexStream;

    fn id(&self) -> Option<u32> {
        Some(42)
    }

    fn take_stdin(&mut self) -> Option<Self::Stdin> {
        None
    }

    fn take_stdout(&mut self) -> Option<Self::Stdout> {
        None
    }

    fn take_stderr(&mut self) -> Option<Self::Stderr> {
        None
    }

    fn try_wait(&self) -> io::Result<Option<ExitStatus>> {
        Ok(None)
    }

    async fn wait(&self) -> io::Result<ExitStatus> {
        Err(io::Error::other("fake process does not exit"))
    }

    async fn kill(&self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(windows)]
fn shell_command(script: &'static str) -> ProcessSpec {
    ProcessSpec::new("cmd.exe").args(["/C", script])
}

#[cfg(not(windows))]
fn shell_command(script: &'static str) -> ProcessSpec {
    ProcessSpec::new("sh").args(["-c", script])
}

#[cfg(windows)]
fn cwd_and_env_command() -> ProcessSpec {
    shell_command("cd && echo %ORA_PROCESS_TEST_VALUE%")
}

#[cfg(not(windows))]
fn cwd_and_env_command() -> ProcessSpec {
    shell_command("pwd; printf '%s\\n' \"$ORA_PROCESS_TEST_VALUE\"")
}

#[cfg(windows)]
fn stdin_echo_command() -> ProcessSpec {
    shell_command("more")
}

#[cfg(not(windows))]
fn stdin_echo_command() -> ProcessSpec {
    ProcessSpec::new("cat")
}

#[cfg(windows)]
fn long_running_command() -> ProcessSpec {
    shell_command("ping -n 6 127.0.0.1 > nul")
}

#[cfg(not(windows))]
fn long_running_command() -> ProcessSpec {
    shell_command("sleep 5")
}
