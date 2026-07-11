//! Platform-specific process-tree termination for spawned child processes.
//!
//! Tree-wide termination is required because a child (for example a shell) may itself spawn nested
//! processes. Killing only the direct child leaves those descendants orphaned and running. This
//! module owns the OS resources and primitives used to request termination of the entire tree
//! rooted at one spawned child:
//!
//! - On Unix the child is placed in its own process group (set via `Command::process_group(0)`);
//!   the entire group is signalled with `kill(-pgid, SIGKILL)`.
//! - On Windows the child is assigned to a Job Object created with
//!   `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`; the whole job is terminated with
//!   `TerminateJobObject`, and closing the job handle kills every process still in the job.
//!
//! [`ProcessTree::kill`] mirrors the `start_kill` contract used by [`crate::ManagedProcess::kill`]:
//! it delivers the termination request to the OS and returns without waiting for any process to
//! actually exit. Callers that need the final exit status must still reap the direct child via
//! [`crate::ManagedProcess::wait`].

use std::io;

use tokio::process::{Child, Command};

/// Owns the OS resources required to terminate an entire process tree rooted at one spawned
/// child process.
///
/// Created from a freshly-spawned child and held by the lifecycle task so every kill path
/// (explicit `kill()`, `kill_on_drop`, and lifecycle task teardown) goes through one entry point.
/// Dropping this handle releases those resources; on Windows it also triggers OS-level cleanup of
/// the tree via the Job Object's `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` limit.
pub(crate) struct ProcessTree {
    #[cfg(unix)]
    pgid: i32,
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
}

// SAFETY: On Windows the Job Object handle is owned exclusively by this struct and only ever
// touched through the synchronous win32 APIs used here (no shared interior mutability), so moving
// it across threads and dropping it on whichever thread the lifecycle task lands on is safe.
// On Unix the only field is a plain `i32` process group id, which is intrinsically Send + Sync.
#[cfg(windows)]
unsafe impl Send for ProcessTree {}
#[cfg(windows)]
unsafe impl Sync for ProcessTree {}

impl ProcessTree {
    /// Applies platform-specific spawn configuration so the spawned child becomes the root of a
    /// manageable process tree.
    ///
    /// On Unix this places the child in its own process group; on Windows the Job Object is
    /// created after spawn, so nothing is configured here.
    pub(crate) fn configure_command(command: &mut Command) {
        #[cfg(unix)]
        {
            // A process group of 0 makes the child a process group leader with pgid == child pid.
            // Descendants inherit the same pgid unless they explicitly leave it, which is rare and
            // outside our control. This is the standard mechanism Rust's std documentation points
            // to for tree-wide termination on Unix.
            command.process_group(0);
        }

        #[cfg(not(unix))]
        {
            let _ = command;
        }
    }

    /// Builds a process-tree handle from a freshly-spawned child.
    ///
    /// On Windows this creates the Job Object with `KILL_ON_JOB_CLOSE` and assigns the running
    /// child to it. There is a small race window between spawn and assignment where the child
    /// could fork a subprocess that escapes the job; for Ora's agent runtimes this race is
    /// acceptable and unavoidable without `CREATE_SUSPENDED` plumbing that the tokio `Command`
    /// type does not expose.
    pub(crate) fn from_spawned(child: &Child) -> io::Result<Self> {
        #[cfg(unix)]
        {
            let pid = child
                .id()
                .ok_or_else(|| io::Error::other("spawned child has no platform pid"))?
                as i32;
            Ok(Self { pgid: pid })
        }

        #[cfg(windows)]
        {
            let pid = child
                .id()
                .ok_or_else(|| io::Error::other("spawned child has no platform pid"))?;
            let job = create_kill_on_close_job()?;
            // If assignment fails we must release the just-created job handle, otherwise it would
            // leak and immediately kill the freshly-spawned child via KILL_ON_JOB_CLOSE.
            if let Err(error) = assign_child_to_job(job, pid) {
                close_handle(job);
                return Err(error);
            }
            Ok(Self { job })
        }
    }

    /// Delivers a tree-wide termination request to the OS without waiting for any process to
    /// exit (a `start_kill` contract: the request has been submitted, not necessarily reaped).
    ///
    /// Returns `Ok(())` when the request was accepted by the OS or when the tree is already gone
    /// (for example ESRCH on Unix when the process group no longer exists). Returns `Err` only when
    /// the OS refused the request for a reason callers should surface (for example EPERM).
    pub(crate) fn kill(&self) -> io::Result<()> {
        #[cfg(unix)]
        {
            kill_process_group(self.pgid)
        }

        #[cfg(windows)]
        {
            terminate_job(self.job)
        }
    }
}

#[cfg(windows)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        // CloseHandle releasing our reference to the Job Object. Because the job was created with
        // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, the OS kills every process still in the job once the
        // last handle is closed. Even for keep_alive_on_drop processes this is the right thing to
        // do during runtime teardown: thoroughly cleaning up the tree instead of orphaning it.
        close_handle(self.job);
    }
}

// ---------------------------------------------------------------------------
// Unix implementation: process-group signalling.
// ---------------------------------------------------------------------------

#[cfg(unix)]
fn kill_process_group(pgid: i32) -> io::Result<()> {
    // A negative target delivers the signal to the entire process group identified by |pgid|.
    // This relies on the child being its own group leader (see configure_command); the pid we
    // captured at spawn equals the group id, so -pgid targets the whole tree.
    let result = unsafe { libc::kill(-pgid, libc::SIGKILL) };
    if result == 0 {
        return Ok(());
    }

    let error = io::Error::last_os_error();
    // ESRCH means the group is already gone, which is equivalent to "termination request
    // delivered" from the caller's perspective. Any other failure should be surfaced.
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error)
    }
}

// ---------------------------------------------------------------------------
// Windows implementation: Job Object with kill-on-close semantics.
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn create_kill_on_close_job() -> io::Result<windows_sys::Win32::Foundation::HANDLE> {
    use windows_sys::Win32::System::JobObjects::{
        CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
    };

    // CreateJobObjectW returns a null handle on failure (it does not use INVALID_HANDLE_VALUE).
    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job.is_null() {
        return Err(io::Error::last_os_error());
    }

    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

    let ok = unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if ok == 0 {
        let error = io::Error::last_os_error();
        close_handle(job);
        return Err(error);
    }

    Ok(job)
}

#[cfg(windows)]
fn assign_child_to_job(
    job: windows_sys::Win32::Foundation::HANDLE,
    child_pid: u32,
) -> io::Result<()> {
    use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };

    let child_handle = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, child_pid) };
    if child_handle.is_null() {
        return Err(io::Error::last_os_error());
    }

    // AssignProcessToJobObject can return ERROR_ACCESS_DENIED on systems without nested-job
    // support, but Windows 8+ supports nested jobs, so success is the expected path.
    let ok = unsafe { AssignProcessToJobObject(job, child_handle) };
    close_handle(child_handle);
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}

#[cfg(windows)]
fn terminate_job(job: windows_sys::Win32::Foundation::HANDLE) -> io::Result<()> {
    use windows_sys::Win32::System::JobObjects::TerminateJobObject;

    let ok = unsafe { TerminateJobObject(job, 1) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(windows)]
fn close_handle(handle: windows_sys::Win32::Foundation::HANDLE) {
    let _ = unsafe { windows_sys::Win32::Foundation::CloseHandle(handle) };
}
