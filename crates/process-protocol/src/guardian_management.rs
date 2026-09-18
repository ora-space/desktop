use serde::{Deserialize, Serialize};

use crate::{GuardianChannel, GuardianReadyRequest, HostBinding, ScopeCreationIntent};

/// Message selection separates readiness from management within the current wire version.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GuardianRequest {
    Ready(GuardianReadyRequest),
    Management(GuardianManagementRequest),
    Run(crate::GuardianRunRequest),
}

/// A public host binding used to reject stale callers, not a secret or an authentication proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardianHostSession {
    pub host: HostBinding,
}

/// Only host binding changes are available; no variant grants workload mutation or lease renewal.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum GuardianManagementOperation {
    Bind { host: HostBinding },
    Inspect { session: GuardianHostSession },
}

/// Every channel checks the original scope identity before entering the serialized execution gate.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardianManagementRequest {
    pub version: u16,
    pub intent: ScopeCreationIntent,
    pub channel: GuardianChannel,
    pub operation: GuardianManagementOperation,
}

/// Rejections distinguish stale callers from storage and channel failures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GuardianManagementRejection {
    StaleHost,
    ConflictingHost,
    InvalidEpoch,
    WrongChannel,
    StaleSession,
    StorageUnavailable,
}

/// Successful binding follows its durable commit; inspection proves only current host-session status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum GuardianManagementReply {
    Bound {
        intent: ScopeCreationIntent,
        session: GuardianHostSession,
    },
    Current {
        intent: ScopeCreationIntent,
        session: GuardianHostSession,
        channel: GuardianChannel,
    },
    Rejected(GuardianManagementRejection),
}
