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
//!   `TerminateJobObject`. Graceful drop disarms the kill-on-close limit first, so only an
//!   ungraceful termination (process crash where `Drop` cannot run) relies on handle-close
//!   cleanup to kill every process still in the job.
//!
//! [`ProcessTree::kill`] mirrors the `start_kill` contract used by [`crate::ManagedProcess::kill`]:
//! it delivers the termination request to the OS and returns without waiting for any process to
//! actually exit. Callers that need the final exit status must still reap the direct child via
//! [`crate::ManagedProcess::wait`].

use std::collections::BTreeSet;
use std::io;

use tokio::process::{Child, Command};

/// Owns the OS resources required to terminate an entire process tree rooted at one spawned
/// child process.
///
/// Created from a freshly-spawned child and held by the lifecycle task so every kill path
/// (explicit `kill()`, `kill_on_drop`, and lifecycle task teardown) goes through one entry point.
/// Dropping this handle is an *ordinary* release: on Windows it disarms
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` before closing the Job Object handle, so releasing this
/// handle never terminates the tree on its own. Tree-wide termination only ever happens through
/// the explicit [`ProcessTree::kill`] path (`TerminateJobObject`), which is unaffected by
/// disarming since it acts immediately rather than on handle close.
pub(crate) struct ProcessTree {
    #[cfg(unix)]
    pgid: i32,
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
}

/// Tracks every process tree registered with the external reaper.
///
/// Unix stores independent process-group identifiers, while Windows enrolls direct children in
/// one aggregate Job Object whose membership follows process lifetime automatically. Dropping
/// this value is a fail-safe cleanup path for malformed IPC or other sidecar failures.
pub(crate) struct ReaperTargets {
    registered: BTreeSet<u32>,
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
// SAFETY: The aggregate Job Object handle is likewise owned exclusively by `ReaperTargets` and only
// used through synchronous win32 calls. The reaper shares the value behind a `Mutex`, so only
// `Send` is needed for one thread at a time to use and drop it.
#[cfg(windows)]
unsafe impl Send for ReaperTargets {}

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
    /// child to it after the external reaper has assigned its shared outer Job. There is a small
    /// race window between spawn and assignment where the child could fork a subprocess that
    /// escapes the private Job; the shared reaper Job still contains that descendant. Avoiding
    /// the private-Job race entirely would require `CREATE_SUSPENDED` plumbing that the Tokio
    /// `Command` type does not expose.
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

impl ReaperTargets {
    /// Creates the platform containment needed to own all subsequently registered trees.
    pub(crate) fn new() -> io::Result<Self> {
        Ok(Self {
            registered: BTreeSet::new(),
            #[cfg(windows)]
            job: create_kill_on_close_job()?,
        })
    }

    /// Adds one newly-spawned direct child, treating a process that already exited as registered.
    pub(crate) fn register(&mut self, process_id: u32) -> io::Result<()> {
        if self.registered.contains(&process_id) {
            return Ok(());
        }

        #[cfg(unix)]
        if !process_group_exists(process_id)? {
            return Ok(());
        }

        #[cfg(windows)]
        if !assign_child_to_job_if_running(self.job, process_id)? {
            return Ok(());
        }

        self.registered.insert(process_id);
        Ok(())
    }

    /// Forgets a direct child after its owner observed exit, preventing identifier-reuse hazards.
    pub(crate) fn unregister(&mut self, process_id: u32) -> io::Result<()> {
        #[cfg(unix)]
        {
            // A shell can exit while a background descendant remains in its inherited process
            // group. Retain that group for final cleanup until the complete tree is gone.
            if !process_group_exists(process_id)? {
                self.registered.remove(&process_id);
            }
        }

        #[cfg(not(unix))]
        {
            // Windows Job membership, rather than this identifier set, retains descendants.
            self.registered.remove(&process_id);
        }
        Ok(())
    }

    /// Forcefully terminates every tree that remains registered at parent shutdown.
    pub(crate) fn kill_all(&mut self) -> io::Result<()> {
        #[cfg(unix)]
        {
            let mut first_error = None;
            for process_id in std::mem::take(&mut self.registered) {
                if let Err(error) = kill_process_group(process_id as i32)
                    && first_error.is_none()
                {
                    first_error = Some(error);
                }
            }
            match first_error {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }

        #[cfg(windows)]
        {
            self.registered.clear();
            terminate_job(self.job)
        }

        #[cfg(not(any(unix, windows)))]
        {
            self.registered.clear();
            Ok(())
        }
    }
}

impl Drop for ReaperTargets {
    fn drop(&mut self) {
        let _ = self.kill_all();
        #[cfg(windows)]
        close_handle(self.job);
    }
}

#[cfg(windows)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        // This runs both on the direct child's normal exit and whenever the lifecycle task
        // itself is torn down without an explicit kill (for example Tokio runtime shutdown),
        // including for `keep_alive_on_drop` processes. Neither case should terminate whatever
        // is still running in the job, so disarm JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE first: without
        // this, closing the handle would kill every descendant still in the job even though
        // nobody asked for that. Any real termination already happened synchronously through
        // `kill()` (`TerminateJobObject`) before this Drop runs, so disarming here is safe.
        let _ = disarm_kill_on_close(self.job);
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

/// Reports whether a process group still contains a direct child or any inherited descendants.
#[cfg(unix)]
fn process_group_exists(process_id: u32) -> io::Result<bool> {
    let Ok(pgid) = i32::try_from(process_id) else {
        return Ok(false);
    };
    let result = unsafe { libc::kill(-pgid, 0) };
    if result == 0 {
        return Ok(true);
    }

    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ESRCH) => Ok(false),
        Some(libc::EPERM) => Ok(true),
        _ => Err(error),
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

/// Clears the Job Object's limit flags so a subsequent `CloseHandle` no longer terminates
/// whatever is still running in the job. Used for the ordinary (non-killing) release path in
/// [`Drop for ProcessTree`](ProcessTree), never for the explicit `kill()` path.
#[cfg(windows)]
fn disarm_kill_on_close(job: windows_sys::Win32::Foundation::HANDLE) -> io::Result<()> {
    use windows_sys::Win32::System::JobObjects::{
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };

    // Zeroed LimitFlags clears JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE (the only limit this job was
    // ever configured with; see `create_kill_on_close_job`).
    let info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };

    let ok = unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
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

/// Assigns a child when it is still alive, making registration idempotent for short-lived
/// processes that exit between spawn and the reaper's synchronous enrollment request.
#[cfg(windows)]
fn assign_child_to_job_if_running(
    job: windows_sys::Win32::Foundation::HANDLE,
    child_pid: u32,
) -> io::Result<bool> {
    use windows_sys::Win32::Foundation::ERROR_INVALID_PARAMETER;
    use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };

    let child_handle = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, child_pid) };
    if child_handle.is_null() {
        let error = io::Error::last_os_error();
        return if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
            Ok(false)
        } else {
            Err(error)
        };
    }

    let ok = unsafe { AssignProcessToJobObject(job, child_handle) };
    close_handle(child_handle);
    if ok != 0 {
        return Ok(true);
    }

    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
        Ok(false)
    } else {
        Err(error)
    }
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
