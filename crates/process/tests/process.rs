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
        .expect("expected wait to return after closing stdin, but it hung");
    let exit = exit.unwrap_or_else(|error| panic!("expected process wait to succeed: {error}"));

    assert!(exit.success());
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

// ---------------------------------------------------------------------------
// Tree-wide termination
// ---------------------------------------------------------------------------

// Verifies the tree-kill contract from `ManagedProcess::kill`: when a spawned child starts a
// grandchild (for example a shell launching a long-running tool), killing the managed process must
// also terminate that grandchild. On Unix the grandchild inherits the child's process group, so a
// group-wide `kill(-pgid, SIGKILL)` reaches it; on Windows the grandchild inherits membership in
// the child's Job Object, so `TerminateJobObject` reaches it.
//
// Unix variant probes liveness with `libc::kill(0)`; the Windows variant uses
// `OpenProcess` + `GetExitCodeProcess` against `STILL_ACTIVE`.
#[cfg(unix)]
#[tokio::test]
async fn kill_terminates_descendants_started_by_the_child() {
    use std::time::Duration;

    let marker_dir = tempfile::tempdir()
        .unwrap_or_else(|error| panic!("expected tempdir for pid marker: {error}"));
    let marker_path = marker_dir.path().join("grandchild.pid");
    let script = format!(
        "sh -c 'sleep 30' & echo $! > {marker}; exec sleep 30",
        marker = escape_shell_path(&marker_path)
    );
    let spawner = TokioProcessSpawner::new();
    let process = spawner
        .spawn(ProcessSpec::new("sh").args(["-c", script.as_str()]))
        .unwrap_or_else(|error| panic!("expected process spawn to succeed: {error}"));

    let grandchild_pid = wait_for_marker_pid(&marker_path).await;
    assert_eq!(
        unsafe { libc::kill(grandchild_pid, 0) },
        0,
        "grandchild should be alive before kill"
    );

    process
        .kill()
        .await
        .unwrap_or_else(|error| panic!("expected process kill to succeed: {error}"));
    // start_kill contract: kill returned even while the tree is still tearing down. Wait for the
    // direct child so OS-driven SIGKILL delivery to the descendants has propagated.
    let exit = process
        .wait()
        .await
        .unwrap_or_else(|error| panic!("expected wait after kill to succeed: {error}"));
    assert!(!exit.success());

    // The grandchild was started in the same process group as the killed child (job control is
    // off by default for non-interactive sh), so SIGKILL to the group should reach it as well.
    // Poll for a short window: the grandchild becomes a reparented zombie until pid 1 reaps it,
    // during which `kill -0` keeps returning success even though the process is already dead.
    let mut reaped = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        if unsafe { libc::kill(grandchild_pid, 0) } != 0 {
            reaped = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        reaped,
        "grandchild should have been terminated as part of tree-wide kill"
    );
}

#[cfg(windows)]
#[tokio::test]
async fn kill_terminates_descendants_started_by_the_child() {
    use std::time::Duration;

    let marker_dir = tempfile::tempdir()
        .unwrap_or_else(|error| panic!("expected tempdir for pid marker: {error}"));
    let marker_path = marker_dir.path().join("grandchild.pid");
    // powershell starts a detached ping (the grandchild), records its pid to the marker file, and
    // then blocks in `Start-Sleep` so the powershell process itself stays alive until the kill
    // arrives. `Start-Process` calls CreateProcess under the hood, so the grandchild inherits
    // membership in the powershell process's Job Object.
    let script = format!(
        "$p = Start-Process -FilePath ping -ArgumentList '-n','30','127.0.0.1' -PassThru -WindowStyle Hidden; \
         Out-File -FilePath '{marker}' -InputObject $p.Id -Encoding ASCII; \
         Start-Sleep -Seconds 30",
        marker = escape_powershell_path(&marker_path)
    );
    let spawner = TokioProcessSpawner::new();
    let process = spawner
        .spawn(ProcessSpec::new("powershell").args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script.as_str(),
        ]))
        .unwrap_or_else(|error| panic!("expected process spawn to succeed: {error}"));

    let grandchild_pid = wait_for_marker_pid(&marker_path).await;
    assert!(
        process_alive(grandchild_pid),
        "grandchild should be alive before kill"
    );

    process
        .kill()
        .await
        .unwrap_or_else(|error| panic!("expected process kill to succeed: {error}"));
    // start_kill contract: kill returned even while the tree is still tearing down. Wait for the
    // direct child so the Job Object termination has propagated to the descendants.
    let exit = process
        .wait()
        .await
        .unwrap_or_else(|error| panic!("expected wait after kill to succeed: {error}"));
    assert!(!exit.success());

    // `TerminateJobObject` kills every process in the job asynchronously. Poll for a short window
    // until the grandchild's exit code is no longer `STILL_ACTIVE`.
    let mut reaped = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if !process_alive(grandchild_pid) {
            reaped = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        reaped,
        "grandchild should have been terminated as part of tree-wide kill"
    );
}

#[cfg(unix)]
async fn wait_for_marker_pid(marker_path: &std::path::Path) -> i32 {
    let pid = wait_for_marker_pid_contents(marker_path).await;
    pid.parse::<i32>()
        .unwrap_or_else(|error| panic!("grandchild marker held non-i32 pid {pid:?}: {error}"))
}

#[cfg(windows)]
async fn wait_for_marker_pid(marker_path: &std::path::Path) -> u32 {
    let pid = wait_for_marker_pid_contents(marker_path).await;
    pid.parse::<u32>()
        .unwrap_or_else(|error| panic!("grandchild marker held non-u32 pid {pid:?}: {error}"))
}

/// Polls for the numeric pid string written by the spawned grandchild. Returning the raw string lets
/// the platform-specific callers parse into `i32` (Unix) or `u32` (Windows) without tripping the
/// `expect_used` / `unwrap_used` clippy lints enforced workspace-wide.
async fn wait_for_marker_pid_contents(marker_path: &std::path::Path) -> String {
    use std::time::Duration;

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(contents) = std::fs::read_to_string(marker_path) {
            let trimmed = contents.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "grandchild pid marker was never written"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(unix)]
fn escape_shell_path(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\'', r"'\''")
}

// PowerShell single-quoted strings treat `'` as an escaped quote when doubled, so embedding the
// marker path requires doubling every `'` and wrapping the whole result in `'...'`.
#[cfg(windows)]
fn escape_powershell_path(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\'', "''")
}

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    // A null handle means the process is gone (or we don't have access). The grandchild was
    // started by the test itself, so access is never the issue here.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return false;
    }

    let mut exit_code: u32 = 0;
    let ok = unsafe { GetExitCodeProcess(handle, &mut exit_code) };
    let _ = unsafe { CloseHandle(handle) };
    if ok == 0 {
        return false;
    }
    // STILL_ACTIVE (0x103 = 259) is what GetExitCodeProcess reports until the process actually
    // terminates; any other value means the OS has observed the exit.
    exit_code == STILL_ACTIVE as u32
}
