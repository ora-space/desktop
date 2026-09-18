use serde::{Deserialize, Serialize};

use crate::{
    GuardianRunRejection, HostBinding, HostRunIntent, OutputRead, OutputStream, RunId, RunSnapshot,
    ScopeId, ScopeState,
};

pub const HOST_WIRE_VERSION: u16 = 2;

/// Connectivity and coordination are not process facts; historical Running is never live evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostCoordination {
    Pending,
    Observing,
    Unavailable,
    Rejected(GuardianRunRejection),
}

/// The host projection retains original guardian facts independently of current connectivity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostRunView {
    pub scope: ScopeId,
    pub run: RunId,
    pub stop_requested: bool,
    pub last_observed: Option<RunSnapshot>,
    pub coordination: HostCoordination,
}

/// Closing acceptance and observed guardian closure are deliberately distinct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostScopeView {
    pub scope: ScopeId,
    pub close_requested: bool,
    pub last_observed: Option<ScopeState>,
    pub coordination: HostCoordination,
}

/// Trusted-local requests carry stable attempt identities, not caller-supplied host authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostOperation {
    Inspect,
    CreateScope {
        scope: ScopeId,
    },
    Start {
        intent: HostRunIntent,
    },
    QueryRun {
        run: RunId,
    },
    Stop {
        run: RunId,
    },
    Close {
        scope: ScopeId,
    },
    QueryScope {
        scope: ScopeId,
    },
    Output {
        run: RunId,
        stream: OutputStream,
        offset: usize,
        max_bytes: usize,
    },
}

impl HostOperation {
    /// Output waits cannot consume the host's control-channel connection slots.
    pub fn socket_name(&self) -> &'static str {
        match self {
            Self::Output { .. } => "host-io.sock",
            Self::Inspect
            | Self::CreateScope { .. }
            | Self::Start { .. }
            | Self::QueryRun { .. }
            | Self::Stop { .. }
            | Self::Close { .. }
            | Self::QueryScope { .. } => "host.sock",
        }
    }
}

/// Unknown versions must fail before creating or modifying durable responsibility.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostRequest {
    pub version: u16,
    pub operation: HostOperation,
}

/// Stable failure categories deliberately omit command parameters and environment values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostRejection {
    IncompatibleVersion,
    UnknownIdentity,
    IntentRejected,
    StorageUnavailable,
    GuardianUnavailable,
    Guardian(GuardianRunRejection),
}

/// Acceptance survives disconnect; callers query these original identities instead of retrying work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostReply {
    Ready(HostBinding),
    Run(HostRunView),
    Scope(HostScopeView),
    Output {
        run: RunId,
        stream: OutputStream,
        offset: usize,
        output: OutputRead,
    },
    Rejected(HostRejection),
}
