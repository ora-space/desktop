use crate::{
    AdapterReceipt, ArtifactId, ConditionProposal, ConsumerRevision, CoordinationPlan,
    CoordinationReceipt, DesiredEffect, DesiredState, EffectOperation, EffectResource,
    EffectScopeId, EffectTarget, EffectTargetId, Generation, LocalTimestamp, ManagedItem,
    OperationArtifact, ReadinessReceipt, ReconcileAttempt, ReconcileClaim, ReconcileRequest,
    ResourceClaim, ResourceObservation, ResourceStatus, TargetDeclaration, TargetProjection,
    TargetStatus,
};
use std::collections::BTreeMap;
use std::error::Error;
use thiserror::Error;

/// Preserves concrete persistence failures across the transport-independent Effect seam.
#[derive(Debug, Error)]
#[error("Effect repository operation failed")]
pub struct RepositoryError {
    #[source]
    source: Box<dyn Error + Send + Sync + 'static>,
}

impl RepositoryError {
    pub fn new(source: impl Error + Send + Sync + 'static) -> Self {
        Self {
            source: Box::new(source),
        }
    }
}

/// Result of replacing a complete Desired State using generation compare-and-swap.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReplaceDesiredStateOutcome {
    Unchanged(DesiredState),
    Replaced(DesiredState),
    Conflict {
        expected_generation: Generation,
        current_generation: Generation,
    },
    RevisionUnavailable(crate::EffectRevisionId),
    ScopeRetiring,
}

/// Complete current facts reloaded after a Target request has been claimed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconcileSnapshot {
    pub request: ReconcileRequest,
    pub claim: ReconcileClaim,
    pub desired: DesiredState,
    pub target: EffectTarget,
    pub consumer_revision: ConsumerRevision,
    pub declaration: TargetDeclaration,
    pub resources: BTreeMap<crate::EffectResourceId, EffectResource>,
    pub revisions: BTreeMap<crate::EffectRevisionId, crate::EffectRevision>,
    pub related_targets: BTreeMap<EffectTargetId, RelatedTargetSnapshot>,
    pub coordination_participants:
        BTreeMap<crate::EffectResourceId, BTreeMap<EffectTargetId, crate::CoordinationRequirement>>,
    pub participant_targets: BTreeMap<EffectTargetId, EffectTarget>,
    pub target_status: TargetStatus,
    pub resource_statuses: BTreeMap<crate::EffectResourceId, ResourceStatus>,
    pub managed: BTreeMap<crate::EffectResourceId, Vec<ManagedItem>>,
}

/// Complete planning inputs for another Target contributing to a shared Resource.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelatedTargetSnapshot {
    pub target: EffectTarget,
    pub consumer_revision: ConsumerRevision,
    pub declaration: TargetDeclaration,
}

/// Atomic final result of a planner pass that produced no external mutation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionCommit {
    pub target_projections: Vec<TargetProjection>,
    pub resource_projections: Vec<crate::ResourceProjection>,
    pub target_status: TargetStatus,
    pub resource_statuses: Vec<ResourceStatus>,
    pub managed: Vec<ManagedItem>,
    pub removed_managed: Vec<crate::ManagedIdentity>,
    pub conditions: Vec<ConditionProposal>,
    pub readiness: Option<ReadinessReceipt>,
}

/// Atomic business transition committed after every operation in an attempt was verified.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptFinalization {
    pub attempt: ReconcileAttempt,
    pub operations: Vec<EffectOperation>,
    pub managed: Vec<ManagedItem>,
    pub removed_managed: Vec<crate::ManagedIdentity>,
    pub target_statuses: Vec<TargetStatus>,
    pub resource_statuses: Vec<ResourceStatus>,
    pub readiness: Option<ReadinessReceipt>,
    pub coordination_receipts: Vec<CoordinationReceipt>,
    pub conditions: Vec<ConditionProposal>,
}

/// Deep persistence interface for Desired replacement and durable reconcile state transitions.
///
/// Implementations must enforce Scope isolation and fencing in every write. Preparing attempts,
/// finalizing ledger transitions, and completing requests are atomic operations; callers never
/// write individual status fields or ownership rows directly.
pub trait EffectRepository {
    /// Loads one complete Desired State snapshot.
    fn load_desired_state(&self, scope: &EffectScopeId) -> Result<DesiredState, RepositoryError>;

    /// Replaces the complete normalized set and wakes every active Target atomically.
    fn replace_desired_state(
        &self,
        scope: &EffectScopeId,
        expected_generation: Generation,
        effects: Vec<DesiredEffect>,
        updated_at: LocalTimestamp,
    ) -> Result<ReplaceDesiredStateOutcome, RepositoryError>;

    /// Loads the transaction-consistent Target status and its current Conditions.
    fn load_target_status(
        &self,
        target: &EffectTargetId,
    ) -> Result<Option<(TargetStatus, Vec<crate::EffectCondition>)>, RepositoryError>;

    /// Loads the active Target status selected by its stable Scope and Consumer identities.
    fn load_consumer_target_status(
        &self,
        scope: &EffectScopeId,
        consumer: &crate::ConsumerIdentity,
    ) -> Result<Option<(TargetStatus, Vec<crate::EffectCondition>)>, RepositoryError>;

    /// Coalesces an explicit Target wakeup without mutating Desired State.
    fn request_reconcile(
        &self,
        target: &EffectTargetId,
        requested_at: LocalTimestamp,
    ) -> Result<bool, RepositoryError>;

    /// Claims due Target requests with fencing and returns only their opaque Target identities.
    fn claim_due_targets(
        &self,
        worker: &crate::WorkerIdentity,
        now: LocalTimestamp,
        lease_until: LocalTimestamp,
        limit: usize,
    ) -> Result<Vec<(EffectTargetId, ReconcileClaim)>, RepositoryError>;

    /// Reloads all current facts after claiming so wakeup payloads never become correctness input.
    fn load_reconcile_snapshot(
        &self,
        target: &EffectTargetId,
        claim: &ReconcileClaim,
    ) -> Result<ReconcileSnapshot, RepositoryError>;

    /// Acquires all required Resource claims in stable identity order or returns no authority.
    fn claim_resources(
        &self,
        target: &EffectTargetId,
        claim: &ReconcileClaim,
        resources: &[crate::EffectResourceId],
        now: LocalTimestamp,
        lease_until: LocalTimestamp,
    ) -> Result<Option<Vec<ResourceClaim>>, RepositoryError>;

    /// Persists immutable attempt and operation journals before any external side effect.
    fn prepare_attempt(
        &self,
        claim: &ReconcileClaim,
        attempt: ReconcileAttempt,
        target_projections: Vec<TargetProjection>,
        resource_projections: Vec<crate::ResourceProjection>,
        operations: Vec<EffectOperation>,
        artifacts: Vec<OperationArtifact>,
    ) -> Result<(), RepositoryError>;

    /// Persists monotonic attempt, operation, and coordination progress between external calls.
    fn record_attempt_progress(
        &self,
        claim: &ReconcileClaim,
        attempt: &ReconcileAttempt,
        operations: &[EffectOperation],
        coordination_receipts: &[CoordinationReceipt],
        updated_at: LocalTimestamp,
    ) -> Result<(), RepositoryError>;

    /// Commits current blocked Conditions and request state under the Target fencing token.
    fn block_target(
        &self,
        target: &EffectTargetId,
        claim: &ReconcileClaim,
        target_status: TargetStatus,
        resource_statuses: Vec<ResourceStatus>,
        conditions: Vec<ConditionProposal>,
        updated_at: LocalTimestamp,
    ) -> Result<(), RepositoryError>;

    /// Commits a no-mutation projection/readiness transition and completes or preserves the wakeup.
    fn commit_projection(
        &self,
        claim: &ReconcileClaim,
        commit: ProjectionCommit,
    ) -> Result<(), RepositoryError>;

    /// Atomically finalizes operations, ownership ledgers, statuses, receipts, and request state.
    fn finalize_attempt(
        &self,
        claim: &ReconcileClaim,
        finalization: AttemptFinalization,
    ) -> Result<(), RepositoryError>;

    /// Releases a failed claim into a durable retry schedule while preserving newer wakeups.
    fn schedule_retry(
        &self,
        target: &EffectTargetId,
        claim: &ReconcileClaim,
        not_before: LocalTimestamp,
        updated_at: LocalTimestamp,
    ) -> Result<Option<crate::RetryAttempt>, RepositoryError>;

    /// Loads immutable unfinished operations in deterministic preparation order for recovery.
    fn load_unfinished_operations(&self) -> Result<Vec<EffectOperation>, RepositoryError>;

    /// Converts every unfinished journal into explicit manual recovery instead of guessing state.
    fn quarantine_unfinished_operations(
        &self,
        detected_at: LocalTimestamp,
    ) -> Result<usize, RepositoryError>;

    /// Deletes durable artifact authority after the adapter proves the exact artifact absent.
    fn complete_artifact_cleanup(
        &self,
        artifact: &ArtifactId,
        receipt: CleanupReceipt,
    ) -> Result<(), RepositoryError>;

    /// Persists cleanup failure without changing already-finalized business state.
    fn mark_artifact_cleanup_failed(
        &self,
        artifact: OperationArtifact,
        failed_at: LocalTimestamp,
    ) -> Result<(), RepositoryError>;
}

/// Reports an external Consumer protocol failure without exposing it as a Core state variant.
#[derive(Debug, Error)]
#[error("Consumer adapter operation failed")]
pub struct ConsumerAdapterError {
    #[source]
    source: Box<dyn Error + Send + Sync + 'static>,
}

impl ConsumerAdapterError {
    pub fn new(source: impl Error + Send + Sync + 'static) -> Self {
        Self {
            source: Box::new(source),
        }
    }
}

/// Adapter at the seam between Generic Targets and Consumer-specific coordination/readiness.
///
/// Implementations interpret only their own versioned contracts and return receipts tied to the
/// exact Target projection. Runtime-specific session or deployment state stays behind this seam.
pub trait ConsumerAdapter {
    /// Establishes the safe-to-mutate barrier for one participating Target.
    fn coordinate(
        &self,
        target: &EffectTarget,
        plan: &CoordinationPlan,
    ) -> Result<CoordinationReceipt, ConsumerAdapterError>;

    /// Reactivates one previously coordinated Target after Resource verification.
    fn reactivate(
        &self,
        target: &EffectTarget,
        plan: &CoordinationPlan,
    ) -> Result<CoordinationReceipt, ConsumerAdapterError>;

    /// Confirms that the Consumer can consume the exact complete Target projection.
    fn verify_ready(
        &self,
        target: &EffectTarget,
        projection: &TargetProjection,
    ) -> Result<ReadinessReceipt, ConsumerAdapterError>;
}

/// Reports a Resource adapter observation, mutation, verification, or cleanup failure.
#[derive(Debug, Error)]
#[error("Resource adapter operation failed")]
pub struct ResourceAdapterError {
    #[source]
    source: Box<dyn Error + Send + Sync + 'static>,
}

impl ResourceAdapterError {
    pub fn new(source: impl Error + Send + Sync + 'static) -> Self {
        Self {
            source: Box::new(source),
        }
    }
}

/// Receipt returned after an idempotent Resource apply call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplyReceipt {
    pub operation: crate::EffectOperationId,
    pub proof: AdapterReceipt,
}

/// Receipt proving exact planned state after Resource application or recovery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationReceipt {
    pub operation: crate::EffectOperationId,
    pub proof: AdapterReceipt,
}

/// Receipt proving an exact operation-owned artifact no longer exists.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CleanupReceipt {
    pub artifact: ArtifactId,
    pub proof: AdapterReceipt,
}

/// Immutable operation journal and its exact cleanup artifacts prepared as one unit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedOperation {
    pub operation: EffectOperation,
    pub artifacts: Vec<OperationArtifact>,
}

/// Adapter at the seam between generic Resource facts and one external Resource protocol.
///
/// Implementations turn pure mutation proposals into durable intent, make apply idempotent against
/// exact expected/planned state, and refuse to guess when neither state matches. Cleanup uses only
/// durable operation artifact authority.
pub trait ResourceAdapter {
    /// Builds one immutable operation and all artifact authority without applying side effects.
    fn prepare_operation(
        &self,
        resource: &EffectResource,
        attempt: crate::ReconcileAttemptId,
        generation: Generation,
        sequence: u32,
        mutation: crate::PlannedMutation,
        prepared_at: LocalTimestamp,
    ) -> Result<PreparedOperation, ResourceAdapterError>;

    /// Observes a complete normalized snapshot without granting ownership.
    fn observe(
        &self,
        resource: &EffectResource,
    ) -> Result<ResourceObservation, ResourceAdapterError>;

    /// Applies one previously persisted immutable operation journal.
    fn apply(&self, operation: &EffectOperation) -> Result<ApplyReceipt, ResourceAdapterError>;

    /// Verifies that external state equals the operation's exact planned state.
    fn verify(
        &self,
        operation: &EffectOperation,
    ) -> Result<VerificationReceipt, ResourceAdapterError>;

    /// Deletes only the exact artifact authorized by its durable locator and fingerprint.
    fn cleanup(&self, artifact: &OperationArtifact)
    -> Result<CleanupReceipt, ResourceAdapterError>;
}
