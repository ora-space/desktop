use std::future::Future;
use std::io;
use std::process::ExitStatus;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::ProcessSpec;

/// Spawns OS child processes from a fully described process specification.
///
/// Upper layers should depend on this trait with static dispatch so tests can inject a fake
/// spawner without starting real child processes.
pub trait ProcessSpawner {
    /// Process handle type returned by this spawner.
    type Process: ManagedProcess;

    /// Spawns one child process and returns a handle for lifecycle and stdio access.
    fn spawn(&self, spec: ProcessSpec) -> io::Result<Self::Process>;
}

/// Owns one child process lifecycle and exposes its raw async stdio pipes.
///
/// Implementations should keep protocol parsing, buffering, health checks, and business meaning in
/// upper layers. This trait only models process lifecycle and pipe ownership.
pub trait ManagedProcess {
    /// Stdin pipe type exposed by this process implementation.
    type Stdin: AsyncWrite + Unpin + Send + 'static;
    /// Stdout pipe type exposed by this process implementation.
    type Stdout: AsyncRead + Unpin + Send + 'static;
    /// Stderr pipe type exposed by this process implementation.
    type Stderr: AsyncRead + Unpin + Send + 'static;

    /// Returns the platform process identifier when the backend exposes one.
    fn id(&self) -> Option<u32>;

    /// Moves the stdin pipe out of the process handle, returning `None` after it has been taken.
    fn take_stdin(&mut self) -> Option<Self::Stdin>;

    /// Moves the stdout pipe out of the process handle, returning `None` after it has been taken.
    fn take_stdout(&mut self) -> Option<Self::Stdout>;

    /// Moves the stderr pipe out of the process handle, returning `None` after it has been taken.
    fn take_stderr(&mut self) -> Option<Self::Stderr>;

    /// Checks whether the child has exited without waiting for it to finish.
    fn try_wait(&self) -> io::Result<Option<ExitStatus>>;

    /// Waits until the child exits and returns its platform exit status.
    fn wait(&self) -> impl Future<Output = io::Result<ExitStatus>> + Send + '_;

    /// Forcefully terminates the child process.
    fn kill(&self) -> impl Future<Output = io::Result<()>> + Send + '_;
}

/// A stable direct-process exit projection independent of platform-specific status objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessExit {
    pub exit_code: Option<i32>,
    pub success: bool,
}

impl From<ExitStatus> for ProcessExit {
    fn from(status: ExitStatus) -> Self {
        Self {
            exit_code: status.code(),
            success: status.success(),
        }
    }
}

/// Stable failures for the process-tree containment and cleanup boundary.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProcessTreeError {
    #[error("the platform cannot guarantee pre-execution process-tree containment")]
    TreeKillUnavailable,
    #[error("process tree stdio capabilities were already transferred")]
    CapabilitiesAlreadyTransferred,
    #[error("failed to terminate the process tree: {message}")]
    TerminationFailed { message: String },
    #[error("failed while observing the direct process: {message}")]
    DirectProcessFailed { message: String },
    #[error("failed while observing tree emptiness: {message}")]
    TreeObservationFailed { message: String },
    #[error("process-tree cleanup exceeded its deadline")]
    TreeCleanupTimeout,
}

/// The three owned byte-stream endpoints transferred exactly once to generation I/O tasks.
#[derive(Debug)]
pub struct PluginStdio<Stdin, Stdout, Stderr> {
    pub stdin: Stdin,
    pub stdout: Stdout,
    pub stderr: Stderr,
}

/// Separates one tree owner into capabilities that can be driven concurrently.
#[derive(Debug)]
pub struct ProcessTreeParts<Stdin, Stdout, Stderr, Controller, DirectExit, TreeEmpty> {
    pub stdio: PluginStdio<Stdin, Stdout, Stderr>,
    pub controller: Controller,
    pub direct_exit: DirectExit,
    pub tree_empty: TreeEmpty,
}

/// Concrete capability bundle produced by one [`ManagedProcessTree`] implementation.
pub type ManagedProcessTreeParts<Tree> = ProcessTreeParts<
    <Tree as ManagedProcessTree>::Stdin,
    <Tree as ManagedProcessTree>::Stdout,
    <Tree as ManagedProcessTree>::Stderr,
    <Tree as ManagedProcessTree>::Controller,
    <Tree as ManagedProcessTree>::DirectExit,
    <Tree as ManagedProcessTree>::TreeEmpty,
>;

/// Requests idempotent termination while exit and tree-empty watchers retain their handles.
pub trait ProcessTreeController: Clone + Send + Sync + 'static {
    /// Terminates every process currently assigned to this generation's managed tree.
    fn terminate_tree(&self) -> Result<(), ProcessTreeError>;
}

/// Owns one generation's direct process, complete hierarchy, and async stdio pipes.
pub trait ManagedProcessTree: Sized {
    type Stdin: AsyncWrite + Unpin + Send + 'static;
    type Stdout: AsyncRead + Unpin + Send + 'static;
    type Stderr: AsyncRead + Unpin + Send + 'static;
    type Controller: ProcessTreeController;
    type DirectExit: Future<Output = Result<ProcessExit, ProcessTreeError>> + Send + 'static;
    type TreeEmpty: Future<Output = Result<(), ProcessTreeError>> + Send + 'static;

    /// Returns the direct process identifier retained independently from descendant accounting.
    fn direct_process_id(&self) -> u32;

    /// Transfers stdio, termination, direct-exit, and tree-empty capabilities exactly once.
    fn into_parts(self) -> Result<ManagedProcessTreeParts<Self>, ProcessTreeError>;
}

/// Creates a contained process tree before any untrusted plugin code can execute.
pub trait ProcessTreeSpawner {
    type ProcessTree: ManagedProcessTree;

    /// Spawns one atomically contained process tree or fails without running the child.
    fn spawn_tree(&self, spec: ProcessSpec) -> io::Result<Self::ProcessTree>;
}
