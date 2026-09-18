use serde::{Deserialize, Serialize};

use crate::{
    GuardianChannel, GuardianHostSession, GuardianManagementRejection, OutputRead, OutputStream,
    RunId, RunSnapshot, RunSpec, ScopeCreationIntent, ScopeState,
};

pub const GUARDIAN_OUTPUT_CHUNK_LIMIT: usize = 4096;
pub const GUARDIAN_CAPTURE_LIMIT: usize = 1_048_576;
pub const GUARDIAN_RUN_LIMIT: usize = 64;

/// The initial rootless path requires an explicit choice, not an implicit connection lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GuardianHostDisconnect {
    KeepRunning,
}

/// Polling and force-stop form the initial trusted-local Run capability; stdin remains closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum GuardianRunOperation {
    Start {
        run: RunId,
        spec: RunSpec,
        host_disconnect: GuardianHostDisconnect,
    },
    Query {
        run: RunId,
    },
    Stop {
        run: RunId,
    },
    Close,
    Scope,
    Output {
        run: RunId,
        stream: OutputStream,
        offset: usize,
        max_bytes: usize,
    },
}

impl GuardianRunOperation {
    /// Keeps socket routing with the operation declaration rather than duplicated at each caller.
    pub fn channel(&self) -> GuardianChannel {
        match self {
            Self::Output { .. } => GuardianChannel::Io,
            Self::Start { .. }
            | Self::Query { .. }
            | Self::Stop { .. }
            | Self::Close
            | Self::Scope => GuardianChannel::Control,
        }
    }
}

/// Public identity and host ordering prevent accidental stale execution, not malicious same-UID use.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardianRunRequest {
    pub version: u16,
    pub intent: ScopeCreationIntent,
    pub channel: GuardianChannel,
    pub session: GuardianHostSession,
    pub operation: GuardianRunOperation,
}

/// Transport success alone cannot be interpreted as durable acceptance or completed cleanup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GuardianRunRejection {
    Management(GuardianManagementRejection),
    UnknownRun,
    ConflictingRun,
    ScopeClosed,
    LimitExceeded,
    StorageUnavailable,
    OutputUnavailable,
}

/// Snapshot replies are journaled facts; output is explicitly volatile, not part of that promise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GuardianRunResult {
    Run(RunSnapshot),
    Scope(ScopeState),
    Output {
        run: RunId,
        stream: OutputStream,
        offset: usize,
        output: OutputRead,
    },
    Rejected(GuardianRunRejection),
}

/// Every exchange echoes its immutable scope and host binding for semantic correlation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardianRunReply {
    pub intent: ScopeCreationIntent,
    pub session: GuardianHostSession,
    pub result: GuardianRunResult,
}
