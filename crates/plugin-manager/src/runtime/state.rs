use ora_process::{ProcessExit, ProcessTreeError};

use crate::StopReason;

/// Runtime actor state with illegal phase combinations excluded by enum construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeState {
    Stopped,
    Starting {
        generation: u64,
        spawn_token: SpawnToken,
    },
    CancellingStart {
        generation: u64,
        spawn_token: SpawnToken,
        reason: StopReason,
    },
    Initializing {
        generation: u64,
        pid: u32,
    },
    Activating {
        generation: u64,
        pid: u32,
    },
    Running {
        generation: u64,
        pid: u32,
    },
    Stopping {
        generation: u64,
        pid: u32,
        reason: StopReason,
    },
    CleanupPending {
        generation: u64,
        process_tree: ProcessTreeToken,
        reason: StopReason,
    },
    Draining {
        generation: u64,
        primary_trigger: DrainTrigger,
        progress: DrainProgress,
    },
    Crashed {
        generation: u64,
        exit: ProcessExit,
    },
    CrashLoop {
        recent_crashes: u32,
    },
}

/// Identifies one start worker without exposing a process capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpawnToken(pub u64);

/// Identifies a cleanup continuation while the actual Job handles remain private.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProcessTreeToken(pub u64);

/// First fatal/drain trigger; later failures are diagnostics and cannot replace it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrainTrigger {
    DirectProcessExit,
    TreeBecameEmpty,
    StdoutBoundaryEof,
    StdoutReadFailure(IoFailure),
    WriterFailure {
        stage: WriterFailureStage,
        failure: IoFailure,
    },
    ProtocolFailure(ProtocolFailure),
    ProcessTreeFailure(ProcessTreeError),
    StopEscalation,
}

/// Closed writer command classes used to map a failure to a transport stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriterFailureStage {
    Request,
    TransportCancel,
    SessionControl,
}

/// Metadata-only I/O categories; raw OS messages remain bounded diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoFailure {
    BrokenPipe,
    ConnectionReset,
    TimedOut,
    Other,
}

/// Stable protocol failure classes that never contain rejected payload text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolFailure {
    InvalidFrame,
    InvalidJson,
    InvalidEnvelope,
    DirectionViolation,
    UnknownResponseId,
    DuplicateTerminal,
    InvalidStreamSequence,
    UnexpectedLifecycleMessage,
}

/// Stable initialize handshake failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeFailure {
    DeadlineExceeded,
    FirstFrameMismatch,
    IdentityMismatch,
    RuntimeVersionMismatch,
    ProcessExited,
}

/// Stable activate failures separated from transport diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationFailure {
    DeadlineExceeded,
    RemoteError,
    InvalidResult,
    ProviderMismatch,
    AdmissionChanged,
    ProcessExited,
}

/// Agent DTO and correlation violations at the Host boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentContractFailure {
    InvalidRequestDto,
    InvalidStreamEvent,
    InvalidTerminalResult,
    InvalidBusinessError,
    ConversationCorrelation,
    ActiveTurnCollision,
    GeneratorProtocol,
}

/// Orthogonal progress of every resource that must converge before settlement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrainProgress {
    pub direct_process: DirectProcessDrain,
    pub stdout: PipeDrain,
    pub stderr: PipeDrain,
    pub tree: TreeDrain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectProcessDrain {
    Awaiting { pid: u32 },
    Reaped { exit: ProcessExit },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipeDrain {
    Open,
    BoundaryEof,
    Failed(PipeDrainFailure),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipeDrainFailure {
    Io(IoFailure),
    Protocol(ProtocolFailure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeDrain {
    Active,
    Empty,
}
