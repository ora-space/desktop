use std::future::Future;
use std::io;
use std::process::ExitStatus;
use std::sync::{Mutex, PoisonError};

use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot, watch};

use crate::{EnvironmentPolicy, ManagedProcess, ProcessSpawner, ProcessSpec};

/// Tokio-backed process spawner for real OS child processes.
#[derive(Debug, Clone, Copy, Default)]
pub struct TokioProcessSpawner;

impl TokioProcessSpawner {
    pub fn new() -> Self {
        Self
    }
}

impl ProcessSpawner for TokioProcessSpawner {
    type Process = TokioManagedProcess;

    fn spawn(&self, spec: ProcessSpec) -> io::Result<Self::Process> {
        let handle = Handle::try_current().map_err(|error| {
            io::Error::other(format!(
                "TokioProcessSpawner requires an active Tokio runtime: {error}"
            ))
        })?;
        let mut command = Command::new(spec.program());
        command.args(spec.args_iter());

        if let Some(cwd) = spec.cwd_path() {
            command.current_dir(cwd);
        }
        if spec.environment_policy() == EnvironmentPolicy::ClearAndAllowlist {
            command.env_clear();
        }
        for (key, value) in spec.envs() {
            command.env(key, value);
        }

        command.stdin(spec.stdin_policy().as_stdio());
        command.stdout(spec.stdout_policy().as_stdio());
        command.stderr(spec.stderr_policy().as_stdio());
        command.kill_on_drop(spec.should_kill_on_drop());

        let mut child = command.spawn()?;
        let id = child.id();
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (exit_tx, exit_rx) = watch::channel(None);
        let (kill_tx, kill_rx) = mpsc::unbounded_channel();
        let (drop_tx, drop_rx) = if spec.should_kill_on_drop() {
            let (drop_tx, drop_rx) = oneshot::channel();
            (Some(drop_tx), Some(drop_rx))
        } else {
            (None, None)
        };
        handle.spawn(run_process_lifecycle(child, kill_rx, drop_rx, exit_tx));

        Ok(TokioManagedProcess {
            id,
            stdin: Mutex::new(stdin),
            stdout,
            stderr,
            exit_rx,
            kill_tx,
            drop_tx,
        })
    }
}

/// Tokio-backed process handle with owned stdio pipes and shared lifecycle observation.
#[derive(Debug)]
pub struct TokioManagedProcess {
    id: Option<u32>,
    stdin: Mutex<Option<ChildStdin>>,
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
    exit_rx: watch::Receiver<Option<ExitState>>,
    kill_tx: mpsc::UnboundedSender<KillRequest>,
    drop_tx: Option<oneshot::Sender<()>>,
}

impl Drop for TokioManagedProcess {
    fn drop(&mut self) {
        if let Some(drop_tx) = self.drop_tx.take() {
            let _ = drop_tx.send(());
        }
    }
}

impl ManagedProcess for TokioManagedProcess {
    type Stdin = ChildStdin;
    type Stdout = ChildStdout;
    type Stderr = ChildStderr;

    fn id(&self) -> Option<u32> {
        self.id
    }

    fn take_stdin(&mut self) -> Option<Self::Stdin> {
        // A poisoned mutex only means some prior holder panicked while locked;
        // the guard is still usable, so recover it instead of poisoning the caller.
        self.stdin
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    fn take_stdout(&mut self) -> Option<Self::Stdout> {
        self.stdout.take()
    }

    fn take_stderr(&mut self) -> Option<Self::Stderr> {
        self.stderr.take()
    }

    fn try_wait(&self) -> io::Result<Option<ExitStatus>> {
        match exit_result(&self.exit_rx.borrow()) {
            Some(result) => result.map(Some),
            None => Ok(None),
        }
    }

    fn wait(&self) -> impl Future<Output = io::Result<ExitStatus>> + Send + '_ {
        // If the caller never took ownership of stdin, close the write end before
        // waiting so stdin-driven children (cat, more, grep, ...) observe EOF and
        // exit instead of hanging forever. tokio's native Child::wait does this
        // automatically; moving stdin off the Child and onto this handle lost that
        // protection, so we restore it here. The take is idempotent: a caller that
        // already took stdin to write finds None here and nothing is dropped.
        drop(
            self.stdin
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take(),
        );

        let mut exit_rx = self.exit_rx.clone();

        async move {
            loop {
                let current = exit_result(&exit_rx.borrow());
                if let Some(result) = current {
                    return result;
                }

                if exit_rx.changed().await.is_err() {
                    let final_state = exit_result(&exit_rx.borrow());
                    return final_state.unwrap_or_else(|| {
                        Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            "process lifecycle task stopped before reporting exit",
                        ))
                    });
                }
            }
        }
    }

    /// Forcefully terminates the child process.
    ///
    /// Known limitation: only the direct child is terminated. Descendant processes spawned by the
    /// child (for example a shell launching further tools) keep running, because tree-wide
    /// termination requires Unix process groups / Windows Job Objects. This also applies to the
    /// `kill_on_drop` path. Tree-wide termination is tracked as a follow-up task.
    ///
    /// Returns once the termination request is delivered to the OS; callers that need the final
    /// exit status must still await [`ManagedProcess::wait`].
    fn kill(&self) -> impl Future<Output = io::Result<()>> + Send + '_ {
        let kill_tx = self.kill_tx.clone();
        let exit_rx = self.exit_rx.clone();

        async move {
            if let Some(result) = exit_result(&exit_rx.borrow()) {
                return result.map(|_| ());
            }

            let (ack_tx, ack_rx) = oneshot::channel();
            if kill_tx.send(KillRequest { ack: ack_tx }).is_err() {
                return lifecycle_closed_result(&exit_rx);
            }

            match ack_rx.await {
                Ok(result) => result,
                Err(_) => lifecycle_closed_result(&exit_rx),
            }
        }
    }
}

#[derive(Debug)]
struct KillRequest {
    ack: oneshot::Sender<io::Result<()>>,
}

#[derive(Debug, Clone)]
enum ExitState {
    Exited(ExitStatus),
    Failed(WaitFailure),
}

#[derive(Debug, Clone)]
struct WaitFailure {
    kind: io::ErrorKind,
    message: String,
}

impl WaitFailure {
    fn from_error(error: io::Error) -> Self {
        Self {
            kind: error.kind(),
            message: error.to_string(),
        }
    }

    fn to_error(&self) -> io::Error {
        io::Error::new(self.kind, self.message.clone())
    }
}

async fn run_process_lifecycle(
    mut child: Child,
    mut kill_rx: mpsc::UnboundedReceiver<KillRequest>,
    mut drop_rx: Option<oneshot::Receiver<()>>,
    exit_tx: watch::Sender<Option<ExitState>>,
) {
    let mut kill_rx_open = true;

    loop {
        match drop_rx.as_mut() {
            Some(drop_signal) => {
                tokio::select! {
                    status = child.wait() => {
                        publish_exit(status, &exit_tx);
                        return;
                    }
                    request = kill_rx.recv(), if kill_rx_open => {
                        handle_kill_request(&mut child, request, &mut kill_rx_open);
                    }
                    _ = drop_signal => {
                        drop_rx = None;
                        let _ = child.start_kill();
                    }
                }
            }
            None => {
                tokio::select! {
                    status = child.wait() => {
                        publish_exit(status, &exit_tx);
                        return;
                    }
                    request = kill_rx.recv(), if kill_rx_open => {
                        handle_kill_request(&mut child, request, &mut kill_rx_open);
                    }
                }
            }
        }
    }
}

fn handle_kill_request(child: &mut Child, request: Option<KillRequest>, kill_rx_open: &mut bool) {
    match request {
        Some(KillRequest { ack }) => {
            let _ = ack.send(child.start_kill());
        }
        None => {
            *kill_rx_open = false;
        }
    }
}

fn publish_exit(status: io::Result<ExitStatus>, exit_tx: &watch::Sender<Option<ExitState>>) {
    let state = match status {
        Ok(status) => ExitState::Exited(status),
        Err(error) => ExitState::Failed(WaitFailure::from_error(error)),
    };
    let _ = exit_tx.send(Some(state));
}

fn exit_result(state: &Option<ExitState>) -> Option<io::Result<ExitStatus>> {
    match state {
        Some(ExitState::Exited(status)) => Some(Ok(status.to_owned())),
        Some(ExitState::Failed(error)) => Some(Err(error.to_error())),
        None => None,
    }
}

fn lifecycle_closed_result(exit_rx: &watch::Receiver<Option<ExitState>>) -> io::Result<()> {
    match exit_result(&exit_rx.borrow()) {
        Some(Ok(_)) => Ok(()),
        Some(Err(error)) => Err(error),
        None => Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "process lifecycle task stopped before accepting kill request",
        )),
    }
}
