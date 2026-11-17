//! Durable stop responsibility contains identifiers and evidence only, never runtime input.
use crate::AttemptId;
use ora_node_protocol::{ExecutionId, NodeRuntimeIdentity, PluginId, PluginVersion};
use serde::{Deserialize, Serialize};

/// Deployment supplies a trusted physical-host identity; a container machine-id is not inferred.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionStopHost {
    pub boot_id: String,
    pub trusted_host_id: Option<String>,
}

impl SessionStopHost {
    /// The kernel boot identity is useful on its own only for same-boot evidence.
    pub fn current_untrusted() -> Self {
        Self {
            boot_id: std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
                .map(|boot| boot.trim().to_owned())
                .unwrap_or_default(),
            trusted_host_id: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStopKind {
    Agent,
    StdioProbe,
    HttpProbe,
    LegacyAgent,
}

/// A view key names Node-owned immutable bytes, never a caller-supplied filesystem path.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionStopPackage {
    pub plugin_id: PluginId,
    pub plugin_version: PluginVersion,
    pub view_key: Option<String>,
}

/// Immutable identity recorded after the lock is synced and before any process is dispatched.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionStopIdentity {
    pub attempt: AttemptId,
    pub execution: ExecutionId,
    pub checkout: ExecutionId,
    pub node: NodeRuntimeIdentity,
    pub host: SessionStopHost,
    pub kind: SessionStopKind,
    pub packages: Vec<SessionStopPackage>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStopState {
    Prepared,
    Stopping,
    StopUnconfirmed,
    LegacyUnknown,
    Confirmed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStopProof {
    LockAndProcesses,
    HostReboot,
    FencedVolume,
}

/// The first stop deadline is immutable across repeated requests and Node restarts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionStopRecord {
    pub identity: SessionStopIdentity,
    pub state: SessionStopState,
    pub created_at_ms: i64,
    pub first_stop_at_ms: Option<i64>,
    pub bound_at_ms: Option<i64>,
    pub confirmed_at_ms: Option<i64>,
    pub proof: Option<SessionStopProof>,
    pub resources_released: bool,
}

/// Closed mutations retain ownership until evidence and resource release have both committed.
#[derive(Clone, Debug)]
pub enum SessionStopChange {
    Request { at_ms: i64, bound_at_ms: i64 },
    Unconfirmed,
    Confirm { at_ms: i64, proof: SessionStopProof },
    ResourcesReleased,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStopReason {
    Maintenance,
    ProbeCapacity,
    LegacyUpgrade,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStopMode {
    Recheck,
    AfterHostReboot,
}

/// The management client cannot supply an actor UID or free-form diagnostic/secret text.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionStopReconcile {
    pub request_id: String,
    pub execution: ExecutionId,
    pub attempt: AttemptId,
    pub reason: SessionStopReason,
    pub mode: SessionStopMode,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStopObservation {
    Stopped,
    Running,
    Unknown,
}

/// The stored audit result is also the idempotent response to an exact repeated request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionStopAuditResult {
    pub observation: SessionStopObservation,
    pub before: SessionStopState,
    pub after: SessionStopState,
    pub proof: Option<SessionStopProof>,
    pub released: Vec<SessionStopPackage>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionStopAuditAdmission {
    Started,
    InProgress,
    Completed(SessionStopAuditResult),
}
