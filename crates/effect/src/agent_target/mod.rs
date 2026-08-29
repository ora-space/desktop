//! Agent Target identity, status, conditions, and reconcile-request domain types.
//!
//! These types define the target-keyed Effect persistence shape introduced beside the existing
//! surface-keyed Skill worker. Runtime cutover is intentionally out of scope for this module.

mod ports;

pub use ports::{AgentTargetRepository, AgentTargetRepositoryError, initial_agent_target_status};

use crate::{ConsumerId, Generation, ManagedIdentity, SkillSelectionKey, SurfaceKey};
use ora_domain::WorkspaceId;
use serde::{Deserialize, Serialize};
use std::fmt::{self, Display, Formatter};
use uuid::Uuid;

/// Opaque primary key for one persisted Agent Target row.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct AgentTargetId(String);

impl AgentTargetId {
    /// Wraps an already-persisted identity without inventing a new one.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Allocates a random identity so targets cannot be inferred from Workspace paths.
    pub fn random() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// Returns the persistence representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for AgentTargetId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Canonical Agent Plugin identity used as half of an Agent Target unique key.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct AgentPluginId(String);

impl AgentPluginId {
    /// Accepts the stable plugin identity string already used by surface consumers.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the persistence representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for AgentPluginId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<ConsumerId> for AgentPluginId {
    /// Surface consumers already carry the Agent Plugin identity string.
    fn from(value: ConsumerId) -> Self {
        Self::new(value.as_str())
    }
}

/// Digest of the Agent Plugin version and its declared configuration capabilities.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct AgentCapabilityRevision(String);

impl AgentCapabilityRevision {
    /// Accepts an opaque revision token; empty means "not yet negotiated" during Expand.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the persistence representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for AgentCapabilityRevision {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Whether an Agent Target still participates in Desired convergence.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTargetLifecycle {
    Active,
    Retired,
}

/// Unique natural key for one Agent Target: Workspace × Agent Plugin.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct AgentTargetIdentity {
    pub workspace_id: WorkspaceId,
    pub agent_plugin_id: AgentPluginId,
}

impl AgentTargetIdentity {
    /// Builds the only legal Agent Target identity shape.
    pub fn new(workspace_id: WorkspaceId, agent_plugin_id: AgentPluginId) -> Self {
        Self {
            workspace_id,
            agent_plugin_id,
        }
    }
}

/// Durable Agent Target row without runtime scheduling state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentTarget {
    pub id: AgentTargetId,
    pub identity: AgentTargetIdentity,
    pub capability_revision: AgentCapabilityRevision,
    pub lifecycle: AgentTargetLifecycle,
    pub created_at: i64,
    pub updated_at: i64,
}

/// High-level progression of one Agent Target reconciliation state machine.
///
/// ReadyWithIssues is target-owned: Skill surfaces never express that phase.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTargetPhase {
    Pending,
    WaitingForIdle,
    Quiescing,
    Applying,
    Resuming,
    Current,
    ReadyWithIssues,
    Degraded,
    Retiring,
    RecoveryRequired,
}

/// Whether a condition blocks readiness or only remains visible after Ready.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionImpact {
    Blocking,
    NonBlocking,
}

/// Identifies the precise subject of an Agent Target-owned condition.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentTargetConditionSubject {
    AgentTarget,
    Surface { surface_key: SurfaceKey },
    Consumer { consumer_id: ConsumerId },
    DesiredSkill { selection_key: SkillSelectionKey },
    ManagedSkill { managed_identity: ManagedIdentity },
    Mcp { managed_identity: String },
}

/// Stable machine-readable reasons for an Agent Target not being fully ready.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTargetConditionReason {
    NoConsumers,
    IncompatibleSurfaceDeclarations,
    DesiredCollision,
    PreservedConflict,
    OwnershipConflict,
    DriftConflict,
    SourceUnavailable,
    PathUnsafe,
    ScanFailed,
    WaitingForIdle,
    ConsumerResumeFailed,
    MaterializationFailed,
    TransientIo,
    RecoveryRequired,
    UnsupportedByAgent,
    CapabilityInvalid,
}

/// A current, structured explanation owned by one Agent Target.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentTargetCondition {
    pub id: String,
    pub subject: AgentTargetConditionSubject,
    pub reason: AgentTargetConditionReason,
    pub impact: ConditionImpact,
    pub message: String,
    pub first_observed_at: i64,
    pub last_observed_at: i64,
    pub failed_generation: Option<Generation>,
    /// Optional physical surface association retained for Skill diagnostics.
    pub surface_key: Option<SurfaceKey>,
    /// Optional consumer association retained when the condition is consumer-scoped.
    pub consumer_id: Option<ConsumerId>,
}

/// Current status of one Agent Target, including readiness generations and conditions.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentTargetStatus {
    pub agent_target_id: AgentTargetId,
    pub identity: AgentTargetIdentity,
    pub desired_generation: Generation,
    pub observed_generation: Generation,
    pub applied_generation: Generation,
    pub ready_generation: Generation,
    pub phase: AgentTargetPhase,
    pub status_version: u64,
    pub created_at: i64,
    pub updated_at: i64,
    pub conditions: Vec<AgentTargetCondition>,
}

/// Scheduling state for a durable Agent Target reconcile request.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTargetReconcileState {
    Pending,
    Claimed,
    Blocked,
    RetryScheduled,
}

/// Why a target was woken; kept as a closed set so workers cannot invent opaque strings.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTargetWakeReason {
    DesiredChanged,
    CapabilityChanged,
    Retry,
    Recovery,
    StartupRepair,
}

/// Durable Agent Target reconcile request used by later target-keyed workers.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentTargetReconcileRequest {
    pub agent_target_id: AgentTargetId,
    pub identity: AgentTargetIdentity,
    pub requested_generation: Generation,
    pub request_token: String,
    pub state: AgentTargetReconcileState,
    pub wake_reason: AgentTargetWakeReason,
    pub blocked_reason: Option<String>,
    pub lease_owner: Option<String>,
    pub lease_expires_at: Option<i64>,
    pub attempt_count: u32,
    pub requested_at: i64,
    pub not_before_at: i64,
    pub updated_at: i64,
}

/// Complete persisted Agent Target snapshot returned by repository round-trips.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentTargetRecord {
    pub target: AgentTarget,
    pub status: AgentTargetStatus,
    pub reconcile_request: Option<AgentTargetReconcileRequest>,
}
