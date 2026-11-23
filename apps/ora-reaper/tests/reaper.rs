use pretty_assertions::assert_eq;
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(windows)]
use ora_process::{ManagedProcess, ProcessSpawner, ProcessSpec, TokioProcessSpawner};

const READY_FRAME: &[u8; 4] = b"ORA1";
const REGISTER: u8 = 1;
const SHUTDOWN: u8 = 3;

/// Verifies the shared reaper Job can contain multiple independently managed process trees.
#[cfg(windows)]
#[tokio::test]
async fn registers_multiple_managed_process_trees() {
    ora_process::initialize_reaper(env!("CARGO_BIN_EXE_ora-reaper"))
        .unwrap_or_else(|error| panic!("initialize process reaper: {error}"));
    let spawner = TokioProcessSpawner::new();
    let first = spawner
        .spawn(managed_long_running_target())
        .unwrap_or_else(|error| panic!("spawn first managed target: {error}"));
    let second = spawner
        .spawn(managed_long_running_target())
        .unwrap_or_else(|error| panic!("spawn second managed target: {error}"));

    first
        .kill()
        .await
        .unwrap_or_else(|error| panic!("kill first managed target: {error}"));
    second
        .kill()
        .await
        .unwrap_or_else(|error| panic!("kill second managed target: {error}"));
    first
        .wait()
        .await
        .unwrap_or_else(|error| panic!("wait for first managed target: {error}"));
    second
        .wait()
        .await
        .unwrap_or_else(|error| panic!("wait for second managed target: {error}"));
    ora_process::shutdown_reaper()
        .unwrap_or_else(|error| panic!("shut down process reaper: {error}"));
}

/// Verifies parent EOF makes the sidecar terminate a registered live process.
#[test]
fn parent_eof_reaps_registered_process() {
    let mut reaper = spawn_reaper();
    assert_ready(&mut reaper);
    let mut target = spawn_long_running_target();
    let target_id = target.id();
    register(&mut reaper, target_id);

    drop(reaper.stdin.take());
    wait_for_successful_exit(&mut reaper, "reaper");
    wait_for_exit(&mut target, "registered target");
}

/// Verifies explicit shutdown acknowledges cleanup even when a registration already exited.
#[test]
fn shutdown_is_idempotent_for_an_exited_process() {
    let mut reaper = spawn_reaper();
    assert_ready(&mut reaper);
    let mut target = spawn_short_lived_target();
    let target_id = target.id();
    register(&mut reaper, target_id);
    let status = target
        .wait()
        .unwrap_or_else(|error| panic!("wait for short-lived target: {error}"));
    assert!(status.success());

    request(&mut reaper, SHUTDOWN, 0);
    wait_for_successful_exit(&mut reaper, "reaper");
}

/// Verifies each termination signal makes the sidecar terminate a registered live process group
/// before it exits successfully, instead of dying with the group still running.
#[cfg(unix)]
#[test]
fn termination_signals_reap_registered_process() {
    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        let mut reaper = spawn_reaper();
        assert_ready(&mut reaper);
        let mut target = spawn_long_running_target();
        register(&mut reaper, target.id());

        send_signal(&reaper, signal);
        let reaper_status = wait_for_status(&mut reaper, "reaper");
        let target_signal = wait_for_signal_exit(&mut target, "registered target");

        assert_eq!(
            (signal, reaper_status.code(), target_signal),
            (signal, Some(0), Some(libc::SIGKILL))
        );
    }
}

/// Verifies a burst of termination signals neither aborts the cleanup the first one started nor
/// changes its successful exit.
#[cfg(unix)]
#[test]
fn repeated_termination_signals_complete_one_cleanup() {
    let mut reaper = spawn_reaper();
    assert_ready(&mut reaper);
    let mut targets: Vec<Child> = (0..3).map(|_| spawn_long_running_target()).collect();
    for target in &targets {
        register(&mut reaper, target.id());
    }

    for signal in [
        libc::SIGTERM,
        libc::SIGTERM,
        libc::SIGHUP,
        libc::SIGINT,
        libc::SIGTERM,
    ] {
        send_signal(&reaper, signal);
    }
    let reaper_status = wait_for_status(&mut reaper, "reaper");
    let target_signals: Vec<Option<i32>> = targets
        .iter_mut()
        .map(|target| wait_for_signal_exit(target, "registered target"))
        .collect();

    assert_eq!(
        (reaper_status.code(), target_signals),
        (Some(0), vec![Some(libc::SIGKILL); 3])
    );
}

/// Verifies a signal racing an explicit shutdown still ends in exactly one successful cleanup,
/// whichever of the two runs it.
#[cfg(unix)]
#[test]
fn termination_signal_during_shutdown_completes_cleanup() {
    let mut reaper = spawn_reaper();
    assert_ready(&mut reaper);
    let mut target = spawn_long_running_target();
    register(&mut reaper, target.id());

    let stdin = reaper
        .stdin
        .as_mut()
        .unwrap_or_else(|| panic!("reaper stdin unavailable"));
    stdin
        .write_all(&[SHUTDOWN, 0, 0, 0, 0])
        .and_then(|()| stdin.flush())
        .unwrap_or_else(|error| panic!("write shutdown request: {error}"));
    send_signal(&reaper, libc::SIGTERM);
    let reaper_status = wait_for_status(&mut reaper, "reaper");
    let target_signal = wait_for_signal_exit(&mut target, "registered target");

    assert_eq!(
        (reaper_status.code(), target_signal),
        (Some(0), Some(libc::SIGKILL))
    );
}

/// Verifies the case of a killed parent whose cleanup owner signals the sidecar right away: the
/// signal racing EOF cleanup cannot make the sidecar exit before its registered group is killed.
#[cfg(unix)]
#[test]
fn termination_signal_after_parent_eof_completes_cleanup() {
    let mut reaper = spawn_reaper();
    assert_ready(&mut reaper);
    let mut target = spawn_long_running_target();
    register(&mut reaper, target.id());

    drop(reaper.stdin.take());
    send_signal(&reaper, libc::SIGTERM);
    let reaper_status = wait_for_status(&mut reaper, "reaper");
    let target_signal = wait_for_signal_exit(&mut target, "registered target");

    assert_eq!(
        (reaper_status.code(), target_signal),
        (Some(0), Some(libc::SIGKILL))
    );
}

/// Delivers one signal to the sidecar alone, as a cleanup owner signalling it by PID would.
#[cfg(unix)]
fn send_signal(reaper: &Child, signal: i32) {
    let process_id = libc::pid_t::try_from(reaper.id())
        .unwrap_or_else(|error| panic!("reaper pid out of range: {error}"));
    // SAFETY: `kill` only reads its integer arguments.
    let result = unsafe { libc::kill(process_id, signal) };
    assert_eq!(
        result,
        0,
        "signal reaper: {}",
        std::io::Error::last_os_error()
    );
}

/// Polls one process and returns how it ended, failing instead of hanging past the deadline.
#[cfg(unix)]
fn wait_for_status(child: &mut Child, label: &str) -> std::process::ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(status) = child
            .try_wait()
            .unwrap_or_else(|error| panic!("poll child exit: {error}"))
        {
            return status;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!("{label} did not exit before its deadline");
}

/// Waits for a target and reports the signal that ended it.
#[cfg(unix)]
fn wait_for_signal_exit(child: &mut Child, label: &str) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    wait_for_status(child, label).signal()
}

/// Starts the compiled sidecar with the private protocol attached to pipes.
fn spawn_reaper() -> Child {
    Command::new(env!("CARGO_BIN_EXE_ora-reaper"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("spawn ora-reaper: {error}"))
}

/// Checks the version marker before sending requests to the sidecar.
fn assert_ready(reaper: &mut Child) {
    let mut ready = [0_u8; READY_FRAME.len()];
    reaper
        .stdout
        .as_mut()
        .unwrap_or_else(|| panic!("reaper stdout unavailable"))
        .read_exact(&mut ready)
        .unwrap_or_else(|error| panic!("read readiness frame: {error}"));
    assert_eq!(&ready, READY_FRAME);
}

/// Registers one PID and verifies the synchronous acknowledgement.
fn register(reaper: &mut Child, process_id: u32) {
    request(reaper, REGISTER, process_id);
}

/// Exchanges one fixed-width operation with the sidecar.
fn request(reaper: &mut Child, operation: u8, process_id: u32) {
    let stdin = reaper
        .stdin
        .as_mut()
        .unwrap_or_else(|| panic!("reaper stdin unavailable"));
    stdin
        .write_all(&[operation])
        .unwrap_or_else(|error| panic!("write operation: {error}"));
    stdin
        .write_all(&process_id.to_le_bytes())
        .unwrap_or_else(|error| panic!("write process id: {error}"));
    stdin
        .flush()
        .unwrap_or_else(|error| panic!("flush request: {error}"));

    let mut response = [0_u8; 6];
    reaper
        .stdout
        .as_mut()
        .unwrap_or_else(|| panic!("reaper stdout unavailable"))
        .read_exact(&mut response)
        .unwrap_or_else(|error| panic!("read acknowledgement: {error}"));
    assert_eq!(response[0], 0);
    assert_eq!(response[1], operation);
    let mut acknowledged_process_id = [0_u8; size_of::<u32>()];
    acknowledged_process_id.copy_from_slice(&response[2..]);
    assert_eq!(u32::from_le_bytes(acknowledged_process_id), process_id);
}

/// Starts a target that remains alive until the reaper terminates it.
#[cfg(unix)]
fn spawn_long_running_target() -> Child {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new("sh");
    command.args(["-c", "exec sleep 30"]).process_group(0);
    command
        .spawn()
        .unwrap_or_else(|error| panic!("spawn long-running target: {error}"))
}

/// Builds the same long-running command through Ora's managed-process API.
#[cfg(windows)]
fn managed_long_running_target() -> ProcessSpec {
    ProcessSpec::new("powershell").args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "Start-Sleep -Seconds 30",
    ])
}

/// Starts a target that remains alive until the reaper terminates it.
#[cfg(windows)]
fn spawn_long_running_target() -> Child {
    Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Start-Sleep -Seconds 30",
        ])
        .spawn()
        .unwrap_or_else(|error| panic!("spawn long-running target: {error}"))
}

/// Starts a successful target that remains alive long enough to register before it exits.
#[cfg(unix)]
fn spawn_short_lived_target() -> Child {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new("sh");
    command.args(["-c", "sleep 0.1"]).process_group(0);
    command
        .spawn()
        .unwrap_or_else(|error| panic!("spawn short-lived target: {error}"))
}

/// Starts a successful target that remains alive long enough to register before it exits.
#[cfg(windows)]
fn spawn_short_lived_target() -> Child {
    Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Start-Sleep -Milliseconds 100",
        ])
        .spawn()
        .unwrap_or_else(|error| panic!("spawn short-lived target: {error}"))
}

/// Polls process exit so a broken reaper fails the test instead of hanging indefinitely.
fn wait_for_exit(child: &mut Child, label: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if child
            .try_wait()
            .unwrap_or_else(|error| panic!("poll child exit: {error}"))
            .is_some()
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let _ = child.kill();
    panic!("{label} did not exit before its deadline");
}

/// Polls one expected-success process and checks its final exit status.
fn wait_for_successful_exit(child: &mut Child, label: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(status) = child
            .try_wait()
            .unwrap_or_else(|error| panic!("poll child exit: {error}"))
        {
            assert!(status.success(), "{label} exited with {status}");
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let _ = child.kill();
    panic!("{label} did not exit before its deadline");
}
