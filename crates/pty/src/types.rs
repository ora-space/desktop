use tokio_util::sync::CancellationToken;

/// Carries the durable identifier that keys one live PTY runtime.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PtySessionId(String);

impl PtySessionId {
    /// Creates a PTY session identifier from any owned or borrowed string input.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl AsRef<str> for PtySessionId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for PtySessionId {
    /// Writes the stable session identifier into the provided formatter.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Captures whether a PTY runtime currently has a live attached client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PtyClientAttachmentState {
    Detached,
    Attached,
}

/// Describes whether a PTY runtime is still running or has exited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PtySessionStatus {
    Running,
    Exited,
}

/// Carries one ordered chunk of PTY output that adapters can replay or stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtyOutputChunk {
    pub data: String,
}

/// Carries the ordered replay buffer returned during attachment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtySessionReplay {
    pub chunks: Vec<PtyOutputChunk>,
}

/// Describes one command sent from an adapter into the PTY runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PtyCommand {
    Input { data: String },
    Resize { cols: u16, rows: u16 },
    Kill,
}

/// Reports one lifecycle event emitted by the PTY runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PtyLifecycleEvent {
    Exited {
        session_id: PtySessionId,
        exit_code: Option<i32>,
    },
}

/// Wraps the server-owned shutdown token that roots every PTY session token.
#[derive(Debug, Clone)]
pub struct PtyServerToken {
    token: CancellationToken,
}

impl PtyServerToken {
    /// Builds a new root shutdown token for PTY session ownership.
    pub fn new() -> Self {
        Self {
            token: CancellationToken::new(),
        }
    }

    /// Returns the root cancellation token so callers can derive child session tokens.
    pub fn cancellation_token(&self) -> CancellationToken {
        self.token.clone()
    }

    /// Cancels the root shutdown token and every derived child token.
    pub fn cancel(&self) {
        self.token.cancel();
    }
}

impl Default for PtyServerToken {
    /// Builds the default root server token for PTY runtime ownership.
    fn default() -> Self {
        Self::new()
    }
}

/// Captures stable control failures for attach, input, resize, and kill flows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PtySessionControlError {
    SessionMissing { session_id: PtySessionId },
    SessionExited { session_id: PtySessionId },
    AlreadyAttached { session_id: PtySessionId },
    NotAttached { session_id: PtySessionId },
    ControlFailed { message: String },
}
