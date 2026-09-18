#![cfg(target_os = "linux")]

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::{net::UnixStream, process::CommandExt};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use ora_utils::fs::LinuxFileLock;
use ora_utils::process::{
    LinuxPidFd, ProcessSignal, configure_linux_detached_child, linux_process_snapshot,
};
use pretty_assertions::assert_eq;

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct ChildGuard(Child);

impl Drop for ChildGuard {
    /// Test failures must not leave direct fixture children running or unreaped.
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct ProcessGuard(LinuxPidFd);

impl Drop for ProcessGuard {
    /// An orphaned holder is targeted through its pinned identity, never its numeric PID.
    fn drop(&mut self) {
        let _ = self.0.signal(ProcessSignal::Kill);
    }
}

/// Opens an existing stable inode without truncation; fixture creation is separate from locking.
fn acquire(path: &Path) -> io::Result<LinuxFileLock> {
    LinuxFileLock::try_acquire(OpenOptions::new().read(true).write(true).open(path)?)
}

/// Concurrent forks can briefly duplicate a descriptor before their exec closes it. Wait for
/// actual acquisition, retaining the acquired lock instead of probing then opening a second time.
fn acquire_after_close(path: &Path) -> io::Result<LinuxFileLock> {
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 10);
    loop {
        match acquire(path) {
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(/*millis*/ 2));
            }
            result => return result,
        }
    }
}

/// Waits for a concrete synchronization fact, not for an assumed scheduling delay.
fn until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 10);
    while !ready() {
        assert!(Instant::now() < deadline, "lock fixture timed out");
        std::thread::sleep(Duration::from_millis(/*millis*/ 2));
    }
}

/// A clone keeps exclusion after the original owner drops, without changing the lock file.
#[test]
fn duplicates_retain_lock_until_last_close() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("lock");
    std::fs::write(&path, b"retained contents")?;
    let lock = acquire(&path)?;
    assert!(matches!(acquire(&path), Err(error) if error.kind() == io::ErrorKind::WouldBlock));
    let duplicate = lock.try_clone()?;
    drop(lock);
    assert!(matches!(acquire(&path), Err(error) if error.kind() == io::ErrorKind::WouldBlock));
    let file = duplicate.into_file();
    // SAFETY: the owned descriptor is live; F_GETFD does not mutate it.
    assert!(unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC != 0);
    assert!(matches!(acquire(&path), Err(error) if error.kind() == io::ErrorKind::WouldBlock));
    drop(file);
    let _recovered = acquire_after_close(&path)?;
    assert_eq!(std::fs::read(&path)?, b"retained contents");
    assert!(
        matches!(LinuxFileLock::try_acquire(File::open(directory.path())?), Err(error) if error.kind() == io::ErrorKind::InvalidInput)
    );
    Ok(())
}

/// An unrelated exec must not inherit the default close-on-exec descriptor.
#[test]
fn ordinary_exec_does_not_retain_lock() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("lock");
    std::fs::write(&path, b"")?;
    let lock = acquire(&path)?;
    let mut child = ChildGuard(Command::new("/bin/sleep").arg("30").spawn()?);
    drop(lock);
    let _recovered = acquire_after_close(&path)?;
    assert!(child.0.try_wait()?.is_none());
    Ok(())
}

/// Detached exec strips even accidentally inheritable descriptors outside explicit stdio mappings.
#[test]
fn detached_exec_closes_unintended_inheritable_lock() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("lock");
    File::create(&path)?;
    let file = acquire(&path)?.into_file();
    let mut command = Command::new("/bin/sleep");
    command.arg("30");
    let descriptor = file.as_raw_fd();
    // SAFETY: the file stays live through spawn. Change only the fork child's descriptor flags,
    // so concurrently spawned test children cannot inherit a non-CLOEXEC parent descriptor.
    unsafe {
        command.pre_exec(move || {
            if libc::fcntl(descriptor, libc::F_SETFD, 0) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    configure_linux_detached_child(&mut command);
    let mut child = ChildGuard(command.spawn()?);
    drop(file);
    let _recovered = acquire_after_close(&path)?;
    assert!(child.0.try_wait()?.is_none());
    Ok(())
}

/// A pre-exec barrier proves why CLOEXEC does not eliminate temporary fork inheritance.
#[test]
fn fork_reference_is_released_at_exec_not_parent_close() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("lock");
    std::fs::write(&path, b"")?;
    let lock = acquire(&path)?;
    let (mut parent, child) = UnixStream::pair()?;
    parent.set_read_timeout(Some(Duration::from_secs(/*secs*/ 5)))?;
    let worker = std::thread::spawn(move || {
        let mut command = Command::new("/bin/sleep");
        command.arg("30");
        // SAFETY: the post-fork closure uses only read/write/poll and raw error construction. The
        // captured stream pins its descriptor and the one-byte stack buffers remain live.
        unsafe {
            command.pre_exec(move || {
                let mut byte = [0_u8];
                let mut poll = libc::pollfd {
                    fd: child.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                if libc::write(child.as_raw_fd(), b"r".as_ptr().cast(), /*count*/ 1) != 1
                    || libc::poll(&mut poll, /*nfds*/ 1, /*timeout*/ 5000) != 1
                    || libc::read(
                        child.as_raw_fd(),
                        byte.as_mut_ptr().cast(),
                        /*count*/ 1,
                    ) != 1
                {
                    return Err(io::Error::from_raw_os_error(libc::EIO));
                }
                Ok(())
            });
        }
        command.spawn().map(ChildGuard)
    });
    let mut ready = [0_u8];
    parent.read_exact(&mut ready)?;
    assert_eq!(ready, *b"r");
    drop(lock);
    let while_forked = acquire(&path);
    // Release before asserting so a failed assertion cannot strand the pre-exec fixture.
    parent.write_all(b"g")?;
    let mut child = worker.join().map_err(|_| "spawn worker panicked")??;
    assert!(matches!(while_forked, Err(error) if error.kind() == io::ErrorKind::WouldBlock));
    let _recovered = acquire_after_close(&path)?;
    assert!(child.0.try_wait()?.is_none());
    Ok(())
}

/// A real exec holder retains the same lock after an external SIGKILL of its launching process.
#[test]
fn inherited_lock_survives_launcher_kill() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("lock");
    std::fs::write(&path, b"unchanged")?;
    let mut launcher = ChildGuard(
        Command::new(std::env::current_exe()?)
            .args(["--exact", "lock_fixture", "--nocapture"])
            .env("ORA_LOCK_FIXTURE", "launcher")
            .env("ORA_LOCK_DIRECTORY", directory.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()?,
    );
    until(|| directory.path().join("holder-ready").exists());
    let pid = std::fs::read_to_string(directory.path().join("holder-pid"))?.parse::<u32>()?;
    let mut pinned = None;
    for observation in linux_process_snapshot()? {
        let observation = observation?;
        if observation.pid == pid {
            pinned = Some(ProcessGuard(LinuxPidFd::from_observation(&observation)?));
            break;
        }
    }
    let holder = pinned.ok_or("holder identity missing")?;
    assert!(matches!(acquire(&path), Err(error) if error.kind() == io::ErrorKind::WouldBlock));
    launcher.0.kill()?;
    launcher.0.wait()?;
    assert!(!holder.0.has_exited()?);
    assert!(matches!(acquire(&path), Err(error) if error.kind() == io::ErrorKind::WouldBlock));
    holder.0.signal(ProcessSignal::Kill)?;
    until(|| holder.0.has_exited().unwrap_or(false));
    // Exit notification and final descriptor release need not be observed in the same instant.
    let _recovered = acquire_after_close(&path)?;
    assert_eq!(std::fs::read(&path)?, b"unchanged");
    Ok(())
}

/// Child-only fixture roles use explicit environment injection, never mutate the test runner.
#[test]
fn lock_fixture() -> TestResult {
    let Some(role) = std::env::var_os("ORA_LOCK_FIXTURE") else {
        return Ok(());
    };
    let directory = std::path::PathBuf::from(
        std::env::var_os("ORA_LOCK_DIRECTORY").ok_or("missing directory")?,
    );
    if role == "launcher" {
        let lock = acquire(&directory.join("lock"))?;
        let child = Command::new(std::env::current_exe()?)
            .args(["--exact", "lock_fixture", "--nocapture"])
            .env("ORA_LOCK_FIXTURE", "holder")
            .env("ORA_LOCK_DIRECTORY", &directory)
            .stdin(Stdio::from(lock.try_clone()?.into_file()))
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()?;
        let _child = ChildGuard(child);
        std::thread::sleep(Duration::from_secs(/*secs*/ 20));
    } else if role == "holder" {
        // Application scope/path binding is separate; adoption checks the inherited description.
        let inherited = File::from(std::io::stdin().as_fd().try_clone_to_owned()?);
        let _lock = LinuxFileLock::adopt_inherited(inherited)?;
        std::fs::write(directory.join("holder-pid"), std::process::id().to_string())?;
        std::fs::write(directory.join("holder-ready"), b"ready")?;
        std::thread::sleep(Duration::from_secs(/*secs*/ 20));
    } else {
        return Err("unknown fixture role".into());
    }
    Ok(())
}

/// Merely reopening the same inode does not acquire the inherited holder's qualification.
#[test]
fn adoption_rejects_unlocked_reopened_and_shared_descriptions() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("lock");
    File::create(&path)?;
    assert!(LinuxFileLock::adopt_inherited(File::open(&path)?).is_err());
    let original = acquire(&path)?;
    assert!(LinuxFileLock::adopt_inherited(File::open(&path)?).is_err());
    let adopted = LinuxFileLock::adopt_inherited(original.try_clone()?.into_file())?;
    drop(original);
    assert!(acquire(&path).is_err());
    drop(adopted);
    let shared = acquire_after_close(&path)?.into_file();
    // SAFETY: flock borrows the owned descriptor; a shared lock must never be promoted by adoption.
    assert_eq!(
        unsafe { libc::flock(shared.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) },
        0
    );
    assert!(LinuxFileLock::adopt_inherited(shared.try_clone()?).is_err());
    drop(shared);
    acquire_after_close(&path)?;
    Ok(())
}
