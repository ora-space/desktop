use portable_pty::{CommandBuilder, NativePtySystem, PtySize as PortablePtySize, PtySystem};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Describes the initial size applied when a PTY process starts or resizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtySize {
    pub cols: u16,
    pub rows: u16,
}

/// Carries the spawn configuration needed to launch one PTY-backed shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtyProcessSpawnRequest {
    pub cwd: PathBuf,
    pub program: String,
    pub args: Vec<String>,
    pub size: PtySize,
}

/// Identifies the PTY process instance created for one session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtyProcessId(u64);

impl PtyProcessId {
    /// Creates a process identifier from a raw numeric value.
    pub fn new(value: u64) -> Self {
        Self(value)
    }
}

/// Reports the observed exit status for a PTY process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtyChildExit {
    pub exit_code: Option<i32>,
}

/// Groups the read and write handles adapters use to bridge PTY IO.
pub struct PtyIoHandle {
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
}

/// Describes the process-control surface required by the PTY runtime manager.
pub trait PtyProcessHandle: Send + Sync {
    /// Returns a numeric identifier for the spawned PTY process when the platform exposes one.
    fn id(&self) -> PtyProcessId;

    /// Resizes the PTY without recreating the running child process.
    fn resize(&self, size: PtySize) -> Result<(), PtyProcessFactoryError>;

    /// Requests PTY termination for explicit kill or server shutdown flows.
    fn kill(&self) -> Result<(), PtyProcessFactoryError>;

    /// Waits for the child process to exit and returns the observed exit status.
    fn wait(&self) -> Result<PtyChildExit, PtyProcessFactoryError>;
}

/// Groups the process handle and IO handles returned after a successful PTY spawn.
pub struct PtyProcess {
    pub handle: Arc<dyn PtyProcessHandle>,
    pub io: PtyIoHandle,
}

/// Supplies PTY spawning behind a testable abstraction.
pub trait PtyProcessFactory: Send + Sync {
    /// Starts one PTY-backed process using the provided spawn request.
    fn spawn(&self, request: PtyProcessSpawnRequest) -> Result<PtyProcess, PtyProcessFactoryError>;
}

/// Captures PTY factory and process-control failures behind stable runtime errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PtyProcessFactoryError {
    #[error("failed to spawn pty process: {message}")]
    SpawnFailed { message: String },
    #[error("failed to resize pty process: {message}")]
    ResizeFailed { message: String },
    #[error("failed to kill pty process: {message}")]
    KillFailed { message: String },
    #[error("failed to wait for pty process exit: {message}")]
    WaitFailed { message: String },
}

/// Spawns real PTY-backed processes through `portable_pty`.
#[derive(Debug, Default)]
pub struct PortablePtyProcessFactory;

impl PortablePtyProcessFactory {
    /// Builds the portable PTY factory used by production runtime wiring.
    pub fn new() -> Self {
        Self
    }
}

impl PtyProcessFactory for PortablePtyProcessFactory {
    /// Starts one real PTY process and returns blocking IO handles for runtime orchestration.
    fn spawn(&self, request: PtyProcessSpawnRequest) -> Result<PtyProcess, PtyProcessFactoryError> {
        let pty_system = NativePtySystem::default();
        let pair = pty_system
            .openpty(portable_size(request.size))
            .map_err(|error| PtyProcessFactoryError::SpawnFailed {
                message: error.to_string(),
            })?;
        let mut command = CommandBuilder::new(request.program);

        command.cwd(request.cwd);
        request.args.into_iter().for_each(|argument| {
            command.arg(argument);
        });

        let child = pair.slave.spawn_command(command).map_err(|error| {
            PtyProcessFactoryError::SpawnFailed {
                message: error.to_string(),
            }
        })?;
        let reader = pair.master.try_clone_reader().map_err(|error| {
            PtyProcessFactoryError::SpawnFailed {
                message: error.to_string(),
            }
        })?;
        let writer =
            pair.master
                .take_writer()
                .map_err(|error| PtyProcessFactoryError::SpawnFailed {
                    message: error.to_string(),
                })?;

        Ok(PtyProcess {
            handle: Arc::new(PortablePtyProcessHandle {
                pair_master: Arc::new(Mutex::new(pair.master)),
                child: Arc::new(Mutex::new(child)),
            }),
            io: PtyIoHandle { reader, writer },
        })
    }
}

/// Bridges the `portable_pty` process handle into the runtime-owned trait surface.
struct PortablePtyProcessHandle {
    pair_master: Arc<Mutex<Box<dyn portable_pty::MasterPty + Send>>>,
    child: Arc<Mutex<Box<dyn portable_pty::Child + Send + Sync>>>,
}

impl PtyProcessHandle for PortablePtyProcessHandle {
    /// Returns the platform process identifier when `portable_pty` exposes one.
    fn id(&self) -> PtyProcessId {
        let child = self
            .child
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        PtyProcessId::new(child.process_id().map_or(0, u64::from))
    }

    /// Applies a live PTY resize through the underlying master handle.
    fn resize(&self, size: PtySize) -> Result<(), PtyProcessFactoryError> {
        let master = self
            .pair_master
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        master
            .resize(portable_size(size))
            .map_err(|error| PtyProcessFactoryError::ResizeFailed {
                message: error.to_string(),
            })
    }

    /// Sends a termination request to the underlying child process.
    fn kill(&self) -> Result<(), PtyProcessFactoryError> {
        let mut child = self
            .child
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        child
            .kill()
            .map_err(|error| PtyProcessFactoryError::KillFailed {
                message: error.to_string(),
            })
    }

    /// Waits for the PTY child process to exit and returns the resulting exit code.
    fn wait(&self) -> Result<PtyChildExit, PtyProcessFactoryError> {
        let mut child = self
            .child
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let status = child
            .wait()
            .map_err(|error| PtyProcessFactoryError::WaitFailed {
                message: error.to_string(),
            })?;

        Ok(PtyChildExit {
            exit_code: Some(status.exit_code() as i32),
        })
    }
}

/// Converts the runtime-owned PTY size into the `portable_pty` representation.
fn portable_size(size: PtySize) -> PortablePtySize {
    PortablePtySize {
        rows: size.rows,
        cols: size.cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}
