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

    /// Requests forceful termination of the entire process tree rooted at the spawned child,
    /// including any descendant processes the child may have started (for example a shell that
    /// launched further tools).
    ///
    /// This is a `start_kill` contract: the future resolves once the termination request has been
    /// submitted to the OS, **not** once the tree has fully exited. Callers that need the final
    /// exit status of the direct child must still await [`Self::wait`]. The future returns `Ok`
    /// when the OS accepted the request or the tree was already gone; it returns `Err` only when
    /// the OS refused the request for a reason the caller should surface (for example
    /// permission loss).
    fn kill(&self) -> impl Future<Output = io::Result<()>> + Send + '_;
}
