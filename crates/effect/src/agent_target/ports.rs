//! Persistence port for Agent Target-shaped Effect state.
//!
//! Implementations must keep this shape independent of the surface-keyed Skill worker so Expand
//! can land without switching runtime claim loops.

use super::{
    AgentCapabilityRevision, AgentTarget, AgentTargetCondition, AgentTargetId, AgentTargetIdentity,
    AgentTargetLifecycle, AgentTargetPhase, AgentTargetReconcileRequest, AgentTargetRecord,
    AgentTargetStatus, AgentTargetWakeReason,
};
use crate::Generation;
use ora_domain::WorkspaceId;
use thiserror::Error;

/// Loads and mutates Agent Target persistence without activating target-keyed workers.
///
/// Callers during Expand may exercise these methods in tests and migration verification. Production
/// Skill reconciliation must continue to use the surface-keyed Effect repository APIs.
pub trait AgentTargetRepository {
    /// Creates or refreshes the durable Agent Target row for one Workspace × Agent Plugin pair.
    fn upsert_agent_target(
        &self,
        identity: &AgentTargetIdentity,
        capability_revision: &AgentCapabilityRevision,
        lifecycle: AgentTargetLifecycle,
        updated_at: i64,
    ) -> Result<AgentTarget, AgentTargetRepositoryError>;

    /// Loads one Agent Target by its natural identity when present.
    fn load_agent_target(
        &self,
        identity: &AgentTargetIdentity,
    ) -> Result<Option<AgentTarget>, AgentTargetRepositoryError>;

    /// Loads one Agent Target by opaque primary key when present.
    fn load_agent_target_by_id(
        &self,
        agent_target_id: &AgentTargetId,
    ) -> Result<Option<AgentTarget>, AgentTargetRepositoryError>;

    /// Replaces status generations, phase, version, and owned conditions atomically.
    fn save_agent_target_status(
        &self,
        status: &AgentTargetStatus,
    ) -> Result<(), AgentTargetRepositoryError>;

    /// Loads status plus owned conditions for one Agent Target identity.
    fn load_agent_target_status(
        &self,
        identity: &AgentTargetIdentity,
    ) -> Result<Option<AgentTargetStatus>, AgentTargetRepositoryError>;

    /// Upserts a durable target reconcile request using max-generation / earliest-due semantics.
    fn upsert_agent_target_reconcile_request(
        &self,
        identity: &AgentTargetIdentity,
        requested_generation: Generation,
        wake_reason: AgentTargetWakeReason,
        not_before_at: i64,
        updated_at: i64,
    ) -> Result<AgentTargetReconcileRequest, AgentTargetRepositoryError>;

    /// Loads the durable request for one Agent Target when present.
    fn load_agent_target_reconcile_request(
        &self,
        identity: &AgentTargetIdentity,
    ) -> Result<Option<AgentTargetReconcileRequest>, AgentTargetRepositoryError>;

    /// Replaces the complete condition set owned by one Agent Target.
    fn replace_agent_target_conditions(
        &self,
        agent_target_id: &AgentTargetId,
        conditions: &[AgentTargetCondition],
    ) -> Result<(), AgentTargetRepositoryError>;

    /// Loads the complete persisted record for repository round-trip assertions.
    fn load_agent_target_record(
        &self,
        identity: &AgentTargetIdentity,
    ) -> Result<Option<AgentTargetRecord>, AgentTargetRepositoryError>;

    /// Lists every Agent Target belonging to one Workspace.
    fn list_agent_targets_for_workspace(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<AgentTarget>, AgentTargetRepositoryError>;
}

/// Reports Agent Target persistence failures without leaking SQLite details to domain callers.
#[derive(Debug, Error)]
pub enum AgentTargetRepositoryError {
    #[error("agent target repository error: {0}")]
    Storage(String),
    #[error("agent target not found for workspace `{workspace_id}` and plugin `{agent_plugin_id}`")]
    NotFound {
        workspace_id: String,
        agent_plugin_id: String,
    },
    #[error("corrupt agent target state: {0}")]
    Corrupt(String),
}

impl AgentTargetRepositoryError {
    /// Wraps a storage failure while preserving the original display text.
    pub fn storage(error: impl ToString) -> Self {
        Self::Storage(error.to_string())
    }

    /// Wraps a corrupt-row failure discovered while decoding persisted state.
    pub fn corrupt(error: impl ToString) -> Self {
        Self::Corrupt(error.to_string())
    }
}

/// Initial status used when an Agent Target is first materialized without prior surface history.
pub fn initial_agent_target_status(
    agent_target_id: AgentTargetId,
    identity: AgentTargetIdentity,
    now: i64,
) -> AgentTargetStatus {
    AgentTargetStatus {
        agent_target_id,
        identity,
        desired_generation: Generation::default(),
        observed_generation: Generation::default(),
        applied_generation: Generation::default(),
        ready_generation: Generation::default(),
        phase: AgentTargetPhase::Current,
        status_version: 1,
        created_at: now,
        updated_at: now,
        conditions: Vec::new(),
    }
}
