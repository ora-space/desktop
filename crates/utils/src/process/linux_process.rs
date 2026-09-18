use std::fs::File;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::ExitStatusExt;
use std::process::{Child, ExitStatus};

/// Session membership observed through a pinned proc directory; names need not be UTF-8.
#[derive(Debug)]
pub struct LinuxProcessStat {
    pub pid: u32,
    pub session: u32,
    pub start_ticks: u64,
    directory: File,
}

struct StatFields {
    pid: u32,
    session: u32,
    start_ticks: u64,
}

/// Signals accepted by the stable process handle, never by a remembered numeric PID.
#[derive(Clone, Copy)]
pub enum ProcessSignal {
    Terminate,
    Kill,
}

/// A kernel handle keeps signal delivery tied to the original process after PID reuse.
pub struct LinuxPidFd(OwnedFd);

impl LinuxPidFd {
    /// Pins an exclusively owned child that the caller has not reaped.
    /// No other thread or signal handler may reap that child concurrently.
    pub fn for_child(child: &Child) -> io::Result<Self> {
        Self::open(child.id())
    }

    /// Verifies a procfs observation around acquisition using a pinned proc directory.
    pub fn from_observation(expected: &LinuxProcessStat) -> io::Result<Self> {
        let before = read_stat(&expected.directory)?;
        if (before.pid, before.start_ticks) != (expected.pid, expected.start_ticks) {
            return Err(io::Error::from_raw_os_error(libc::ESRCH));
        }
        let handle = Self::open(expected.pid)?;
        // An exited/reused process cannot be substituted by reopening its numeric path here.
        let after = read_stat(&expected.directory)?;
        if (after.pid, after.start_ticks) != (expected.pid, expected.start_ticks) {
            return Err(io::Error::from_raw_os_error(libc::ESRCH));
        }
        Ok(handle)
    }

    /// Checks syscall availability and signal authorization without sending a real signal.
    pub fn probe_current() -> io::Result<()> {
        let handle = Self::open(std::process::id())?;
        handle.send(/*signal*/ 0)?;
        handle.has_exited()?;
        // Self is not our child. ECHILD proves P_PIDFD/WNOWAIT was understood without creating
        // a probe process; unsupported or seccomp-blocked wait operations fail admission.
        match handle.peek_child_exit() {
            Err(error) if error.raw_os_error() == Some(libc::ECHILD) => Ok(()),
            Err(error) => Err(error),
            Ok(_) => Err(io::Error::other("unexpected self wait result")),
        }
    }

    /// Opens only a positive individual PID; flags never select groups or threads.
    fn open(pid: u32) -> io::Result<Self> {
        let pid = i32::try_from(pid).map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))?;
        if pid <= 0 {
            return Err(io::Error::from_raw_os_error(libc::EINVAL));
        }
        // SAFETY: pidfd_open takes scalar arguments and returns a fresh owned descriptor.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0_u32) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the successful syscall's descriptor is transferred exactly once.
        Ok(Self(unsafe { OwnedFd::from_raw_fd(fd as i32) }))
    }

    /// A readable/hung-up pidfd proves exit even when another parent has not reaped the zombie.
    pub fn has_exited(&self) -> io::Result<bool> {
        let mut poll = libc::pollfd {
            fd: self.0.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: poll receives one live descriptor and a valid writable record.
        if unsafe {
            libc::poll(&mut poll, /*nfds*/ 1, /*timeout*/ 0)
        } < 0
        {
            return Err(io::Error::last_os_error());
        }
        if poll.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
            return Err(io::Error::other("invalid pidfd observation"));
        }
        Ok(poll.revents & (libc::POLLIN | libc::POLLHUP) != 0)
    }

    /// Delivers only to the pinned process; an already exited target is an idempotent success.
    pub fn signal(&self, signal: ProcessSignal) -> io::Result<()> {
        self.send(match signal {
            ProcessSignal::Terminate => libc::SIGTERM,
            ProcessSignal::Kill => libc::SIGKILL,
        })
    }

    /// Shares error handling between capability probing and actual process signals.
    fn send(&self, signal: i32) -> io::Result<()> {
        // SAFETY: the descriptor is live, and a null siginfo asks the kernel to construct it.
        if unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.0.as_raw_fd(),
                signal,
                std::ptr::null::<libc::siginfo_t>(),
                0_u32,
            )
        } < 0
        {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error);
            }
        }
        Ok(())
    }

    /// Observes child exit without releasing its numeric identity or blocking the caller.
    pub fn peek_child_exit(&self) -> io::Result<Option<ExitStatus>> {
        self.wait(libc::WEXITED | libc::WNOHANG | libc::WNOWAIT)
    }

    /// Reaps this original child; use only after exit is observed or in a dedicated waiter.
    pub fn reap_child(&self) -> io::Result<ExitStatus> {
        self.wait(libc::WEXITED)?
            .ok_or_else(|| io::Error::other("missing child exit status"))
    }

    /// waitid through the pidfd never mistakes a reused numeric PID for the original child.
    fn wait(&self, options: i32) -> io::Result<Option<ExitStatus>> {
        // SAFETY: zero initializes siginfo_t's integer/pointer fields, including the no-event PID.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // SAFETY: info is writable and the owned fd is a valid nonnegative id for P_PIDFD.
        if unsafe { libc::waitid(libc::P_PIDFD, self.0.as_raw_fd() as u32, &mut info, options) } < 0
        {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: successful waitid initializes the SIGCHLD fields of siginfo_t.
        if unsafe { info.si_pid() } == 0 {
            return Ok(None);
        }
        let status = unsafe { info.si_status() };
        let raw = match info.si_code {
            libc::CLD_EXITED => status << 8,
            libc::CLD_KILLED => status,
            libc::CLD_DUMPED => status | 0x80,
            _ => return Err(io::Error::other("unexpected child wait event")),
        };
        Ok(Some(ExitStatus::from_raw(raw)))
    }
}

/// Pins one proc directory so callers can compare observations around stable handle acquisition.
pub fn linux_process(pid: u32) -> io::Result<LinuxProcessStat> {
    let directory = File::open(std::path::Path::new("/proc").join(pid.to_string()))?;
    let stat = read_stat(&directory)?;
    Ok(LinuxProcessStat {
        pid: stat.pid,
        session: stat.session,
        start_ticks: stat.start_ticks,
        directory,
    })
}

/// Enumerates visible processes; permission and parse failures are errors, never empty evidence.
pub fn linux_process_snapshot() -> io::Result<impl Iterator<Item = io::Result<LinuxProcessStat>>> {
    Ok(std::fs::read_dir("/proc")?.filter_map(|entry| {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => return Some(Err(error)),
        };
        entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())?;
        let observation = File::open(entry.path()).and_then(|directory| {
            let stat = read_stat(&directory)?;
            Ok(LinuxProcessStat {
                pid: stat.pid,
                session: stat.session,
                start_ticks: stat.start_ticks,
                directory,
            })
        });
        match observation {
            Ok(stat) => Some(Ok(stat)),
            Err(error)
                if error.kind() == io::ErrorKind::NotFound
                    || error.raw_os_error() == Some(libc::ESRCH) =>
            {
                None
            }
            Err(error) => Some(Err(error)),
        }
    }))
}

/// Reads through the pinned proc directory so PID reuse cannot redirect an identity recheck.
fn read_stat(directory: &File) -> io::Result<StatFields> {
    // SAFETY: the directory and static NUL-terminated filename remain live throughout openat.
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            c"stat".as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: openat returned a fresh descriptor, now owned by File.
    let file = unsafe { File::from_raw_fd(fd) };
    let mut bytes = Vec::new();
    file.take(/*limit*/ 16385).read_to_end(&mut bytes)?;
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid proc stat");
    if bytes.len() > 16384 {
        return Err(invalid());
    }
    let begin = bytes
        .iter()
        .position(|byte| *byte == b'(')
        .ok_or_else(invalid)?;
    let end = bytes
        .iter()
        .rposition(|byte| *byte == b')')
        .ok_or_else(invalid)?;
    let pid = std::str::from_utf8(&bytes[..begin])
        .map_err(|_| invalid())?
        .trim()
        .parse()
        .map_err(|_| invalid())?;
    let fields: Vec<_> = std::str::from_utf8(&bytes[end + 1..])
        .map_err(|_| invalid())?
        .split_whitespace()
        .collect();
    if fields.len() < 20 {
        return Err(invalid());
    }
    Ok(StatFields {
        pid,
        session: fields[3].parse().map_err(|_| invalid())?,
        start_ticks: fields[19].parse().map_err(|_| invalid())?,
    })
}
