use ora_pty::{PtyOutputChunkEvent, PtySessionAttachment, PtySessionId};
use tokio::sync::broadcast;

/// Carries the transport-neutral PTY startup request owned by the application layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalRuntimeRequest {
    pub session_id: PtySessionId,
    pub cwd: std::path::PathBuf,
    pub program: String,
    pub args: Vec<String>,
    pub cols: u16,
    pub rows: u16,
}

/// Groups the backend-owned settings used to start task terminal shells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalStartupConfig {
    pub work_dir: std::path::PathBuf,
    pub shell_program: String,
}

/// Carries the runtime result returned after a successful PTY startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalRuntimeResult {
    pub session_id: PtySessionId,
}

/// Carries the replay and live runtime stream returned when a client attaches.
pub struct TerminalAttachment {
    pub replay: Vec<String>,
    pub session_token: tokio_util::sync::CancellationToken,
    pub output_receiver: broadcast::Receiver<PtyOutputChunkEvent>,
}

/// Supplies PTY runtime lifecycle operations behind an application-owned abstraction.
///
/// Implementations are expected to own PTY processes, single-client attachment rules,
/// and session-scoped runtime cleanup while keeping adapters away from runtime internals.
pub trait TerminalRuntime {
    /// Starts one PTY runtime for a persisted terminal session.
    fn start_session(
        &self,
        request: TerminalRuntimeRequest,
    ) -> Result<TerminalRuntimeResult, TerminalRuntimeError>;

    /// Attaches one client to the addressed runtime and returns ordered replay plus live events.
    fn attach_session(
        &self,
        session_id: &PtySessionId,
    ) -> Result<TerminalAttachment, TerminalRuntimeError>;

    /// Marks the addressed runtime detached without terminating the PTY.
    fn detach_session(&self, session_id: &PtySessionId) -> Result<(), TerminalRuntimeError>;

    /// Sends raw terminal input into one running PTY runtime.
    fn send_input(
        &self,
        session_id: &PtySessionId,
        data: String,
    ) -> Result<(), TerminalRuntimeError>;

    /// Applies a live resize request to one running PTY runtime.
    fn resize_session(
        &self,
        session_id: &PtySessionId,
        cols: u16,
        rows: u16,
    ) -> Result<(), TerminalRuntimeError>;

    /// Requests explicit PTY termination for one running terminal session.
    fn kill_session(&self, session_id: &PtySessionId) -> Result<(), TerminalRuntimeError>;
}

impl TerminalAttachment {
    /// Converts the PTY runtime attachment into the application-owned attachment shape.
    pub fn from_pty_attachment(attachment: PtySessionAttachment) -> Self {
        Self {
            replay: attachment
                .replay
                .chunks
                .into_iter()
                .map(|chunk| chunk.data)
                .collect(),
            session_token: attachment.session_token,
            output_receiver: attachment.output_receiver,
        }
    }
}

/// Captures stable PTY runtime failures that application handlers must translate for adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalRuntimeError {
    StartupFailed { message: String },
    RuntimeMissing { session_id: String },
    AlreadyAttached { session_id: String },
    SessionStopped { session_id: String },
    ControlFailed { message: String },
}
