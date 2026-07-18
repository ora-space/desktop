use std::io;
use std::process::ExitStatus;
use std::time::Duration;

#[cfg(windows)]
use std::os::windows::io::{FromRawHandle, OwnedHandle};
#[cfg(windows)]
use windows_sys::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
#[cfg(windows)]
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
};

use ora_process::{ManagedProcess, ProcessSpawner, ProcessSpec, ProcessStdio, TokioProcessSpawner};
#[cfg(windows)]
use ora_process::{ManagedProcessTree, ProcessTreeSpawner, WindowsJobProcessTreeSpawner};
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
    assert!(!spec.should_kill_on_drop());
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

#[tokio::test]
async fn wait_closes_unowned_stdin_so_stdin_readers_exit() {
    let spawner = TokioProcessSpawner::new();
    let process = spawner
        .spawn(stdin_echo_command())
        .unwrap_or_else(|error| panic!("expected process spawn to succeed: {error}"));

    // Deliberately do NOT take_stdin. A stdin-driven child (cat/more) must still
    // exit because wait() closes the unowned write end, mirroring tokio's native
    // Child::wait. Without the fix this hangs until the timeout elapses.
    let exit = tokio::time::timeout(Duration::from_secs(5), process.wait())
        .await
        .unwrap_or_else(|_| panic!("expected wait to return after closing stdin, but it hung"));
    let exit = exit.unwrap_or_else(|error| panic!("expected process wait to succeed: {error}"));

    assert!(exit.success());
}

/// Verifies that Bun can consume and produce byte streams through the exact Windows Job pipes.
#[cfg(windows)]
#[tokio::test]
#[ignore = "run after `task prepare-plugin-runtime` to use the verified local Bun cache"]
async fn bun_stdio_round_trips_through_windows_job_pipes() {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .unwrap_or_else(|| panic!("expected workspace root"));
    let runtime_assets = workspace.join("runtime-assets").join("prepared");
    let bun = runtime_assets.join("bun.exe");
    let script_root = tempfile::tempdir()
        .unwrap_or_else(|error| panic!("expected temporary script root: {error}"));
    let script = script_root.path().join("stdio.js");
    std::fs::write(
        &script,
        r#"process.stderr.write("bun-stderr-ready\n");
for await (const chunk of process.stdin) {
  process.stdout.write(chunk);
}
"#,
    )
    .unwrap_or_else(|error| panic!("expected Bun script fixture: {error}"));

    let mut bunfig_argument = std::ffi::OsString::from("--config=");
    bunfig_argument.push(runtime_assets.join("empty-bunfig.toml"));
    let mut spec = ProcessSpec::new(&bun)
        .args([
            bunfig_argument,
            std::ffi::OsString::from("--no-env-file"),
            std::ffi::OsString::from("run"),
            std::ffi::OsString::from("--no-install"),
            script.into_os_string(),
        ])
        .cwd(script_root.path())
        .clear_and_allowlist_environment();
    for key in ["SystemRoot", "WINDIR", "TEMP", "TMP"] {
        spec = spec.env(
            key,
            std::env::var_os(key)
                .unwrap_or_else(|| panic!("expected required Windows environment {key}")),
        );
    }

    let tree = WindowsJobProcessTreeSpawner::new()
        .spawn_tree(spec)
        .unwrap_or_else(|error| panic!("expected contained Bun spawn: {error}"));
    let parts = tree
        .into_parts()
        .unwrap_or_else(|error| panic!("expected process-tree capabilities: {error}"));
    let mut stdin = parts.stdio.stdin;
    let mut stdout = parts.stdio.stdout;
    let mut stderr = parts.stdio.stderr;
    stdin
        .write_all(b"bun-stdin\n")
        .await
        .unwrap_or_else(|error| panic!("expected Bun stdin write: {error}"));
    stdin
        .shutdown()
        .await
        .unwrap_or_else(|error| panic!("expected Bun stdin shutdown: {error}"));
    drop(stdin);

    let result = tokio::time::timeout(Duration::from_secs(10), async move {
        let mut stdout_bytes = Vec::new();
        let mut stderr_bytes = Vec::new();
        let (stdout_result, stderr_result, direct_exit, tree_empty) = tokio::join!(
            stdout.read_to_end(&mut stdout_bytes),
            stderr.read_to_end(&mut stderr_bytes),
            parts.direct_exit,
            parts.tree_empty,
        );
        (
            stdout_result.map_err(|error| error.to_string()),
            stderr_result.map_err(|error| error.to_string()),
            direct_exit,
            tree_empty,
            String::from_utf8_lossy(&stdout_bytes).into_owned(),
            String::from_utf8_lossy(&stderr_bytes).into_owned(),
        )
    })
    .await
    .unwrap_or_else(|_| panic!("expected contained Bun to finish"));

    assert_eq!(
        result,
        (
            Ok(10),
            Ok(17),
            Ok(ora_process::ProcessExit {
                exit_code: Some(0),
                success: true,
            }),
            Ok(()),
            "bun-stdin\n".to_owned(),
            "bun-stderr-ready\n".to_owned(),
        )
    );
}

/// Verifies that Windows kills the direct plugin and its descendants when the Host disappears.
#[cfg(windows)]
#[test]
#[ignore = "run in the real Windows process-tree E2E gate"]
fn abrupt_host_exit_closes_the_job_and_kills_descendants() {
    use std::io::{BufRead, Write};
    use std::process::{Command, Stdio};

    let mut helper = Command::new(env!("CARGO_BIN_EXE_windows_job_host_fixture"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("expected Host fixture to start: {error}"));
    let mut reported_processes = String::new();
    std::io::BufReader::new(
        helper
            .stdout
            .take()
            .unwrap_or_else(|| panic!("expected Host fixture stdout")),
    )
    .read_line(&mut reported_processes)
    .unwrap_or_else(|error| panic!("expected Host fixture process IDs: {error}"));
    let process_ids = reported_processes
        .split_whitespace()
        .map(|value| {
            value
                .parse::<u32>()
                .unwrap_or_else(|error| panic!("expected numeric process ID `{value}`: {error}"))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        process_ids.len(),
        2,
        "fixture output: {reported_processes:?}"
    );
    let direct_process = open_process_for_wait(process_ids[0]);
    let descendant_process = open_process_for_wait(process_ids[1]);

    helper
        .stdin
        .take()
        .unwrap_or_else(|| panic!("expected Host fixture stdin"))
        .write_all(b"exit now\n")
        .unwrap_or_else(|error| panic!("expected Host fixture acknowledgement: {error}"));
    let status = helper
        .wait()
        .unwrap_or_else(|error| panic!("expected Host fixture exit: {error}"));

    assert_eq!(status.code(), Some(23));
    assert_process_exits(&direct_process, process_ids[0]);
    assert_process_exits(&descendant_process, process_ids[1]);
}

/// Opens a stable synchronization handle before the fixture is allowed to terminate.
#[cfg(windows)]
fn open_process_for_wait(process_id: u32) -> OwnedHandle {
    // SAFETY: OpenProcess owns no borrowed inputs; a successful handle is transferred to RAII.
    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, process_id) };
    assert!(
        !handle.is_null(),
        "failed to open process {process_id}: {}",
        io::Error::last_os_error()
    );
    // SAFETY: the successful OpenProcess return is one uniquely owned live handle.
    unsafe { OwnedHandle::from_raw_handle(handle.cast()) }
}

/// Waits a bounded interval for a process handle to become signaled after Job closure.
#[cfg(windows)]
fn assert_process_exits(process: &OwnedHandle, process_id: u32) {
    use std::os::windows::io::AsRawHandle;

    // SAFETY: the borrowed RAII handle remains valid throughout this bounded wait.
    let wait = unsafe { WaitForSingleObject(process.as_raw_handle() as HANDLE, 10_000) };
    assert_eq!(
        wait, WAIT_OBJECT_0,
        "process {process_id} survived Host exit"
    );
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
