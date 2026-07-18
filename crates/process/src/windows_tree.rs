mod command;
mod pipes;
#[cfg(test)]
mod tests;

use std::ffi::c_void;
use std::future::Future;
use std::io;
use std::mem::{size_of, size_of_val};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::pin::Pin;
use std::ptr::{null, null_mut};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use tokio::net::windows::named_pipe::NamedPipeServer;
use tokio::runtime::Handle;
use tokio::sync::oneshot;
use windows_sys::Win32::Foundation::{
    GetLastError, HANDLE, INVALID_HANDLE_VALUE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::IO::{
    CreateIoCompletionPort, GetQueuedCompletionStatus, PostQueuedCompletionStatus,
};
use windows_sys::Win32::System::JobObjects::{
    CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_ASSOCIATE_COMPLETION_PORT,
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectAssociateCompletionPortInformation, JobObjectBasicAccountingInformation,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject,
};
use windows_sys::Win32::System::SystemServices::JOB_OBJECT_MSG_ACTIVE_PROCESS_ZERO;
use windows_sys::Win32::System::Threading::{
    CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, INFINITE, InitializeProcThreadAttributeList,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST, PROCESS_INFORMATION,
    STARTF_USESTDHANDLES, STARTUPINFOEXW, UpdateProcThreadAttribute, WaitForSingleObject,
};

use crate::{
    ManagedProcessTree, PluginStdio, ProcessExit, ProcessSpec, ProcessStdio, ProcessTreeController,
    ProcessTreeError, ProcessTreeParts, ProcessTreeSpawner,
};
use command::{build_command_line, build_environment_block, wide_nul};
use pipes::{
    PipeDirection, PipeSecurityDescriptor, create_pipe_pair, named_pipe_server, pipe_nonce,
};

const TREE_POLL_MILLIS: u32 = 100;
const TREE_WAKE_MESSAGE: u32 = u32::MAX;
const JOB_TERMINATION_EXIT_CODE: u32 = 1;

/// Creates Windows plugin processes with Job Object containment established by CreateProcessW.
///
/// The implementation intentionally has no suspended-process fallback: if the operating system
/// cannot honor `PROC_THREAD_ATTRIBUTE_JOB_LIST`, no untrusted code is allowed to start.
#[derive(Debug, Clone)]
pub struct WindowsJobProcessTreeSpawner {
    tree_cleanup_timeout: Duration,
}

impl Default for WindowsJobProcessTreeSpawner {
    fn default() -> Self {
        Self {
            tree_cleanup_timeout: Duration::from_secs(10),
        }
    }
}

impl WindowsJobProcessTreeSpawner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Overrides the bounded Job emptiness observation deadline used by the cleanup watcher.
    pub fn with_tree_cleanup_timeout(mut self, timeout: Duration) -> Self {
        self.tree_cleanup_timeout = timeout;
        self
    }
}

impl ProcessTreeSpawner for WindowsJobProcessTreeSpawner {
    type ProcessTree = WindowsJobProcessTree;

    fn spawn_tree(&self, spec: ProcessSpec) -> io::Result<Self::ProcessTree> {
        require_plugin_pipes(&spec)?;
        let runtime = Handle::try_current().map_err(|error| {
            io::Error::other(format!(
                "WindowsJobProcessTreeSpawner requires a Tokio runtime with I/O enabled: {error}"
            ))
        })?;

        let mut transaction = SpawnTransaction::new()?;
        transaction.spawn(&spec)?;
        transaction.finish(runtime, self.tree_cleanup_timeout)
    }
}

/// Owns one atomically contained Windows process generation until its capabilities are split.
pub struct WindowsJobProcessTree {
    process_id: u32,
    stdin: NamedPipeServer,
    stdout: NamedPipeServer,
    stderr: NamedPipeServer,
    inner: Arc<WindowsTreeInner>,
    tree_cleanup_timeout: Duration,
}

impl ManagedProcessTree for WindowsJobProcessTree {
    type Stdin = NamedPipeServer;
    type Stdout = NamedPipeServer;
    type Stderr = NamedPipeServer;
    type Controller = WindowsProcessTreeController;
    type DirectExit = WindowsDirectExit;
    type TreeEmpty = WindowsTreeEmpty;

    fn direct_process_id(&self) -> u32 {
        self.process_id
    }

    fn into_parts(
        self,
    ) -> Result<
        ProcessTreeParts<
            Self::Stdin,
            Self::Stdout,
            Self::Stderr,
            Self::Controller,
            Self::DirectExit,
            Self::TreeEmpty,
        >,
        ProcessTreeError,
    > {
        let (direct_tx, direct_rx) = oneshot::channel();
        let (tree_tx, tree_rx) = oneshot::channel();

        let direct_inner = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let result = wait_for_direct_process(&direct_inner);
            direct_inner.mark_direct_reaped();
            let _ = direct_tx.send(result);
        });

        let tree_inner = Arc::clone(&self.inner);
        let cleanup_timeout = self.tree_cleanup_timeout;
        tokio::task::spawn_blocking(move || {
            let _ = tree_tx.send(wait_for_empty_tree(&tree_inner, cleanup_timeout));
        });

        Ok(ProcessTreeParts {
            stdio: PluginStdio {
                stdin: self.stdin,
                stdout: self.stdout,
                stderr: self.stderr,
            },
            controller: WindowsProcessTreeController {
                inner: Arc::clone(&self.inner),
            },
            direct_exit: WindowsDirectExit {
                receiver: direct_rx,
            },
            tree_empty: WindowsTreeEmpty { receiver: tree_rx },
        })
    }
}

/// Cloneable Job termination capability kept independent from exit observation.
#[derive(Clone)]
pub struct WindowsProcessTreeController {
    inner: Arc<WindowsTreeInner>,
}

impl ProcessTreeController for WindowsProcessTreeController {
    fn terminate_tree(&self) -> Result<(), ProcessTreeError> {
        let mut terminated = self
            .inner
            .terminated
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if *terminated {
            return Ok(());
        }

        // SAFETY: the shared RAII inner keeps the Job handle open for this whole call.
        let success = unsafe {
            TerminateJobObject(
                self.inner.job.as_raw_handle() as HANDLE,
                JOB_TERMINATION_EXIT_CODE,
            )
        };
        if success == 0 {
            return Err(ProcessTreeError::TerminationFailed {
                message: io::Error::last_os_error().to_string(),
            });
        }
        *terminated = true;
        Ok(())
    }
}

/// Future resolving when the direct Bun process has been reaped independently of descendants.
pub struct WindowsDirectExit {
    receiver: oneshot::Receiver<Result<ProcessExit, ProcessTreeError>>,
}

impl Future for WindowsDirectExit {
    type Output = Result<ProcessExit, ProcessTreeError>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.receiver).poll(context) {
            Poll::Ready(Ok(result)) => Poll::Ready(result),
            Poll::Ready(Err(_)) => Poll::Ready(Err(ProcessTreeError::DirectProcessFailed {
                message: "direct-process watcher stopped before reporting exit".to_owned(),
            })),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Future resolving only after the direct process is reaped and the Job has no active process.
pub struct WindowsTreeEmpty {
    receiver: oneshot::Receiver<Result<(), ProcessTreeError>>,
}

impl Future for WindowsTreeEmpty {
    type Output = Result<(), ProcessTreeError>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.receiver).poll(context) {
            Poll::Ready(Ok(result)) => Poll::Ready(result),
            Poll::Ready(Err(_)) => Poll::Ready(Err(ProcessTreeError::TreeObservationFailed {
                message: "tree-empty watcher stopped before reporting completion".to_owned(),
            })),
            Poll::Pending => Poll::Pending,
        }
    }
}

struct WindowsTreeInner {
    // The Job must remain alive until both watchers and the controller have released the inner.
    // KILL_ON_JOB_CLOSE then protects the Host-crash and abandoned-generation paths.
    job: OwnedHandle,
    completion_port: OwnedHandle,
    process: OwnedHandle,
    completion_key: usize,
    _completion_key_owner: Box<u8>,
    direct_reaped: std::sync::atomic::AtomicBool,
    terminated: Mutex<bool>,
}

impl WindowsTreeInner {
    /// Records direct-process reaping and wakes a completion-port waiter without relying on a
    /// possibly delayed `ACTIVE_PROCESS_ZERO` Job message.
    fn mark_direct_reaped(&self) {
        self.direct_reaped
            .store(true, std::sync::atomic::Ordering::Release);
        // SAFETY: the completion port remains owned by this inner. A failed wake is harmless
        // because the tree watcher also performs bounded periodic queries.
        unsafe {
            PostQueuedCompletionStatus(
                self.completion_port.as_raw_handle() as HANDLE,
                TREE_WAKE_MESSAGE,
                self.completion_key,
                null(),
            );
        }
    }
}

struct SpawnTransaction {
    job: Option<OwnedHandle>,
    completion_port: Option<OwnedHandle>,
    completion_key: Box<u8>,
    stdin_host: Option<OwnedHandle>,
    stdin_child: Option<OwnedHandle>,
    stdout_host: Option<OwnedHandle>,
    stdout_child: Option<OwnedHandle>,
    stderr_host: Option<OwnedHandle>,
    stderr_child: Option<OwnedHandle>,
    _job_list: Box<[HANDLE; 1]>,
    handle_list: Box<[HANDLE; 3]>,
    attributes: Option<ProcThreadAttributeList>,
    process: Option<OwnedHandle>,
    thread: Option<OwnedHandle>,
    process_id: Option<u32>,
}

impl SpawnTransaction {
    /// Allocates every pre-spawn capability and freezes both CreateProcess attribute payloads.
    fn new() -> io::Result<Self> {
        let job = create_job()?;
        let completion_port = create_completion_port()?;
        let completion_key = Box::new(0_u8);
        let completion_key_value = (&*completion_key as *const u8) as usize;
        associate_completion_port(
            job.as_raw_handle() as HANDLE,
            completion_port.as_raw_handle() as HANDLE,
            completion_key_value,
        )?;

        let security_descriptor = PipeSecurityDescriptor::current_owner_and_system()?;
        let nonce = pipe_nonce()?;
        let (stdin_host, stdin_child) = create_pipe_pair(
            &format!(r"\\.\pipe\ora-plugin-{nonce}-stdin"),
            PipeDirection::HostWrites,
            &security_descriptor,
        )?;
        let (stdout_host, stdout_child) = create_pipe_pair(
            &format!(r"\\.\pipe\ora-plugin-{nonce}-stdout"),
            PipeDirection::HostReads,
            &security_descriptor,
        )?;
        let (stderr_host, stderr_child) = create_pipe_pair(
            &format!(r"\\.\pipe\ora-plugin-{nonce}-stderr"),
            PipeDirection::HostReads,
            &security_descriptor,
        )?;

        let job_list = Box::new([job.as_raw_handle() as HANDLE]);
        let handle_list = Box::new([
            stdin_child.as_raw_handle() as HANDLE,
            stdout_child.as_raw_handle() as HANDLE,
            stderr_child.as_raw_handle() as HANDLE,
        ]);
        let attributes = ProcThreadAttributeList::new(&job_list, &handle_list)?;

        Ok(Self {
            job: Some(job),
            completion_port: Some(completion_port),
            completion_key,
            stdin_host: Some(stdin_host),
            stdin_child: Some(stdin_child),
            stdout_host: Some(stdout_host),
            stdout_child: Some(stdout_child),
            stderr_host: Some(stderr_host),
            stderr_child: Some(stderr_child),
            _job_list: job_list,
            handle_list,
            attributes: Some(attributes),
            process: None,
            thread: None,
            process_id: None,
        })
    }

    /// Calls CreateProcessW once with the Job list and inherited-handle whitelist already bound.
    fn spawn(&mut self, spec: &ProcessSpec) -> io::Result<()> {
        let application = wide_nul(spec.program(), "program")?;
        let mut command_line = build_command_line(spec)?;
        let current_directory = spec
            .cwd_path()
            .map(|path| wide_nul(path.as_os_str(), "working directory"))
            .transpose()?;
        let environment = build_environment_block(spec)?;

        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = self.handle_list[0];
        startup.StartupInfo.hStdOutput = self.handle_list[1];
        startup.StartupInfo.hStdError = self.handle_list[2];
        startup.lpAttributeList = self
            .attributes
            .as_ref()
            .ok_or_else(|| io::Error::other("process attribute list was released too early"))?
            .as_raw();

        let mut process_info = PROCESS_INFORMATION::default();
        // SAFETY: all UTF-16 buffers, attribute payloads, stdio handles, and the attribute-list
        // backing remain stable in this transaction until CreateProcessW returns.
        let success = unsafe {
            CreateProcessW(
                application.as_ptr(),
                command_line.as_mut_ptr(),
                null(),
                null(),
                1,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
                environment.as_ptr().cast(),
                current_directory.as_ref().map_or(null(), Vec::as_ptr),
                &startup.StartupInfo,
                &mut process_info,
            )
        };
        if success == 0 {
            return Err(io::Error::last_os_error());
        }

        self.process = Some(owned_handle(
            process_info.hProcess,
            "CreateProcessW process handle",
        )?);
        self.thread = Some(owned_handle(
            process_info.hThread,
            "CreateProcessW thread handle",
        )?);
        self.process_id = Some(process_info.dwProcessId);
        Ok(())
    }

    /// Releases parent-only spawn resources and transfers the surviving generation capabilities.
    fn finish(
        mut self,
        _runtime: Handle,
        tree_cleanup_timeout: Duration,
    ) -> io::Result<WindowsJobProcessTree> {
        // These exact copies must close before the Host starts awaiting EOF on its pipe ends.
        self.attributes.take();
        self.thread.take();
        self.stdin_child.take();
        self.stdout_child.take();
        self.stderr_child.take();

        let stdin = named_pipe_server(
            self.stdin_host
                .take()
                .ok_or_else(|| io::Error::other("missing Host stdin pipe"))?,
        )?;
        let stdout = named_pipe_server(
            self.stdout_host
                .take()
                .ok_or_else(|| io::Error::other("missing Host stdout pipe"))?,
        )?;
        let stderr = named_pipe_server(
            self.stderr_host
                .take()
                .ok_or_else(|| io::Error::other("missing Host stderr pipe"))?,
        )?;

        let inner = Arc::new(WindowsTreeInner {
            job: self
                .job
                .take()
                .ok_or_else(|| io::Error::other("missing Job handle"))?,
            completion_port: self
                .completion_port
                .take()
                .ok_or_else(|| io::Error::other("missing completion-port handle"))?,
            process: self
                .process
                .take()
                .ok_or_else(|| io::Error::other("missing direct-process handle"))?,
            completion_key: (&*self.completion_key as *const u8) as usize,
            _completion_key_owner: self.completion_key,
            direct_reaped: std::sync::atomic::AtomicBool::new(false),
            terminated: Mutex::new(false),
        });

        Ok(WindowsJobProcessTree {
            process_id: self
                .process_id
                .take()
                .ok_or_else(|| io::Error::other("missing direct-process id"))?,
            stdin,
            stdout,
            stderr,
            inner,
            tree_cleanup_timeout,
        })
    }
}

struct ProcThreadAttributeList {
    backing: Vec<u8>,
}

impl ProcThreadAttributeList {
    /// Creates one address-stable attribute list containing both containment and inheritance data.
    fn new(job_list: &[HANDLE; 1], handle_list: &[HANDLE; 3]) -> io::Result<Self> {
        let mut bytes = 0_usize;
        // SAFETY: the documented sizing call writes only the required byte count.
        unsafe {
            InitializeProcThreadAttributeList(null_mut(), 2, 0, &mut bytes);
        }
        if bytes == 0 {
            return Err(io::Error::last_os_error());
        }

        let mut list = Self {
            backing: vec![0_u8; bytes],
        };
        // SAFETY: the backing allocation is the exact OS-requested size and is never resized.
        let initialized =
            unsafe { InitializeProcThreadAttributeList(list.as_raw(), 2, 0, &mut bytes) };
        if initialized == 0 {
            return Err(io::Error::last_os_error());
        }

        list.update(
            PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
            job_list.as_ptr().cast(),
            size_of::<HANDLE>(),
        )?;
        list.update(
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            handle_list.as_ptr().cast(),
            size_of_val(handle_list),
        )?;
        Ok(list)
    }

    fn as_raw(&self) -> *mut c_void {
        self.backing.as_ptr().cast_mut().cast()
    }

    /// Adds one attribute while its separately boxed payload remains stable in the transaction.
    fn update(&mut self, attribute: usize, value: *const c_void, bytes: usize) -> io::Result<()> {
        // SAFETY: `value` points at a boxed payload that outlives this list and CreateProcessW.
        let success = unsafe {
            UpdateProcThreadAttribute(
                self.as_raw(),
                0,
                attribute,
                value,
                bytes,
                null_mut(),
                null(),
            )
        };
        if success == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

impl Drop for ProcThreadAttributeList {
    fn drop(&mut self) {
        // SAFETY: this list was initialized exactly once and its backing still exists.
        unsafe { DeleteProcThreadAttributeList(self.as_raw()) };
    }
}

/// Creates and configures a kill-on-close Job before any plugin process exists.
fn create_job() -> io::Result<OwnedHandle> {
    // SAFETY: null attributes/name request a private non-inheritable Job.
    let job = owned_handle(
        unsafe { CreateJobObjectW(null(), null()) },
        "CreateJobObjectW",
    )?;
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: the information buffer has the exact class-specific representation and size.
    let success = unsafe {
        SetInformationJobObject(
            job.as_raw_handle() as HANDLE,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if success == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(job)
}

/// Creates the private completion port used only for this generation's Job notifications.
fn create_completion_port() -> io::Result<OwnedHandle> {
    // SAFETY: INVALID_HANDLE_VALUE with a null existing port creates a new completion port.
    owned_handle(
        unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, null_mut(), 0, 1) },
        "CreateIoCompletionPort",
    )
}

/// Associates the Job with its private completion port and stable generation key.
fn associate_completion_port(job: HANDLE, port: HANDLE, key: usize) -> io::Result<()> {
    let association = JOBOBJECT_ASSOCIATE_COMPLETION_PORT {
        CompletionKey: key as *mut c_void,
        CompletionPort: port,
    };
    // SAFETY: the association structure is valid for this information class.
    let success = unsafe {
        SetInformationJobObject(
            job,
            JobObjectAssociateCompletionPortInformation,
            (&association as *const JOBOBJECT_ASSOCIATE_COMPLETION_PORT).cast(),
            size_of::<JOBOBJECT_ASSOCIATE_COMPLETION_PORT>() as u32,
        )
    };
    if success == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Waits for the direct process and records its stable numeric Windows exit code.
fn wait_for_direct_process(inner: &WindowsTreeInner) -> Result<ProcessExit, ProcessTreeError> {
    // SAFETY: the process handle remains owned by the shared inner for the whole blocking wait.
    let wait = unsafe { WaitForSingleObject(inner.process.as_raw_handle() as HANDLE, INFINITE) };
    if wait == WAIT_FAILED {
        return Err(ProcessTreeError::DirectProcessFailed {
            message: io::Error::last_os_error().to_string(),
        });
    }
    if wait != WAIT_OBJECT_0 {
        return Err(ProcessTreeError::DirectProcessFailed {
            message: format!("unexpected process wait result {wait}"),
        });
    }

    let mut exit_code = 0_u32;
    // SAFETY: the signaled process handle is still valid and exit_code is writable.
    if unsafe { GetExitCodeProcess(inner.process.as_raw_handle() as HANDLE, &mut exit_code) } == 0 {
        return Err(ProcessTreeError::DirectProcessFailed {
            message: io::Error::last_os_error().to_string(),
        });
    }
    Ok(ProcessExit {
        exit_code: Some(exit_code as i32),
        success: exit_code == 0,
    })
}

/// Uses completion messages as wakeups but treats accounting queries as the source of truth.
fn wait_for_empty_tree(
    inner: &WindowsTreeInner,
    timeout: Duration,
) -> Result<(), ProcessTreeError> {
    let deadline = Instant::now() + timeout;
    loop {
        if tree_is_fully_reaped(inner)? {
            return Ok(());
        }
        if Instant::now() >= deadline {
            // A final query at the deadline prevents a just-completed tree from becoming a false
            // timeout due to delayed or coalesced completion-port delivery.
            return if tree_is_fully_reaped(inner)? {
                Ok(())
            } else {
                Err(ProcessTreeError::TreeCleanupTimeout)
            };
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        let wait_millis = remaining
            .min(Duration::from_millis(u64::from(TREE_POLL_MILLIS)))
            .as_millis()
            .max(1) as u32;
        let mut message = 0_u32;
        let mut key = 0_usize;
        let mut overlapped = null_mut();
        // SAFETY: all output pointers are valid, and the shared inner owns the completion port.
        let success = unsafe {
            GetQueuedCompletionStatus(
                inner.completion_port.as_raw_handle() as HANDLE,
                &mut message,
                &mut key,
                &mut overlapped,
                wait_millis,
            )
        };
        if success == 0 {
            let error = unsafe { GetLastError() };
            if error != WAIT_TIMEOUT {
                return Err(ProcessTreeError::TreeObservationFailed {
                    message: io::Error::from_raw_os_error(error as i32).to_string(),
                });
            }
        } else if key != inner.completion_key {
            continue;
        } else if !matches!(
            message,
            JOB_OBJECT_MSG_ACTIVE_PROCESS_ZERO | TREE_WAKE_MESSAGE
        ) {
            // Other messages are useful wakeups as process counts change; the accounting query at
            // the top of the loop remains authoritative.
        }
    }
}

/// Requires both independent direct reaping and zero active Job processes.
fn tree_is_fully_reaped(inner: &WindowsTreeInner) -> Result<bool, ProcessTreeError> {
    let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
    // SAFETY: the output buffer exactly matches JobObjectBasicAccountingInformation.
    let success = unsafe {
        QueryInformationJobObject(
            inner.job.as_raw_handle() as HANDLE,
            JobObjectBasicAccountingInformation,
            (&mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
            size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
            null_mut(),
        )
    };
    if success == 0 {
        return Err(ProcessTreeError::TreeObservationFailed {
            message: io::Error::last_os_error().to_string(),
        });
    }
    Ok(accounting.ActiveProcesses == 0
        && inner
            .direct_reaped
            .load(std::sync::atomic::Ordering::Acquire))
}

/// Rejects any launch whose stdio topology would bypass the inherited-handle whitelist.
fn require_plugin_pipes(spec: &ProcessSpec) -> io::Result<()> {
    if spec.stdin_policy() != ProcessStdio::Piped
        || spec.stdout_policy() != ProcessStdio::Piped
        || spec.stderr_policy() != ProcessStdio::Piped
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "contained plugin processes require piped stdin, stdout, and stderr",
        ));
    }
    Ok(())
}

/// Adopts one successful Win32 handle return value into an RAII owner.
fn owned_handle(handle: HANDLE, operation: &str) -> io::Result<OwnedHandle> {
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::new(
            io::Error::last_os_error().kind(),
            format!("{operation}: {}", io::Error::last_os_error()),
        ));
    }
    // SAFETY: the caller transfers one unique live Win32 handle to this function.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle as RawHandle) })
}
