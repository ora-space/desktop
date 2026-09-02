use crate::{
    AdapterReceipt, ArtifactId, AuditEventId, ConsumerIdentity, CoordinationContract,
    CoordinationRequirement, EffectOperationId, EffectResourceId, EffectScopeId, EffectTargetId,
    Fingerprint, Generation, LocalTimestamp, ManagedIdentity, NativeResourceIdentity,
    ProjectionDigest, ReconcileAttemptId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use thiserror::Error;

/// Exact state accepted before a Resource mutation is retried.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ExactPreviousState {
    Missing,
    Present {
        native_identity: NativeResourceIdentity,
        fingerprint: Fingerprint,
        managed_identity: ManagedIdentity,
    },
}

/// Exact state an adapter must prove after applying an operation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ExactPlannedState {
    Missing,
    Present {
        native_identity: NativeResourceIdentity,
        fingerprint: Fingerprint,
        managed_identity: ManagedIdentity,
    },
}

/// Durable kinds of Resource mutation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectMutation {
    Create,
    Update,
    Replace,
    Delete,
}

/// Filesystem-specific plan interpreted only by the filesystem Resource adapter.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FilesystemOperationPlan {
    pub workspace_root: PathBuf,
    pub resource_relative_path: crate::ResourcePath,
    pub resource_root: PathBuf,
    pub source_root: Option<PathBuf>,
    pub staging_path: PathBuf,
    pub backup_path: PathBuf,
}

/// Shared-file merge plan carrying only secret-free rendered configuration and exact paths.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct JsonMergeOperationPlan {
    pub workspace_root: PathBuf,
    pub resource_relative_path: crate::ResourcePath,
    pub ownership_relative_path: crate::ResourcePath,
    pub configuration_path: PathBuf,
    pub ownership_path: PathBuf,
    pub staging_path: PathBuf,
    pub backup_path: PathBuf,
    pub mutation: EffectMutation,
    pub managed_identity: ManagedIdentity,
    pub native_identity: NativeResourceIdentity,
    pub desired_effect: Option<crate::DesiredEffectIdentity>,
    pub input: Option<Value>,
}

/// Closed versioned operation payload set; Effect Core stores and sequences but does not inspect it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "plan", rename_all = "snake_case")]
pub enum VersionedAdapterPlan {
    FilesystemDirectoryV1(FilesystemOperationPlan),
    JsonMergeV1(Box<JsonMergeOperationPlan>),
}

/// Operation progress encodes legal timestamp combinations directly in its variants.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum OperationProgress {
    Prepared {
        prepared_at: LocalTimestamp,
    },
    Applied {
        prepared_at: LocalTimestamp,
        applied_at: LocalTimestamp,
    },
    Finalized {
        prepared_at: LocalTimestamp,
        applied_at: LocalTimestamp,
        finalized_at: LocalTimestamp,
    },
    RecoveryRequired {
        prepared_at: LocalTimestamp,
        /// Retains known apply evidence instead of erasing it when recovery becomes necessary.
        applied_at: Option<LocalTimestamp>,
        detected_at: LocalTimestamp,
    },
}

/// Immutable Resource mutation intent plus its monotonic durable progress.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EffectOperation {
    identity: EffectOperationId,
    attempt: ReconcileAttemptId,
    resource: EffectResourceId,
    generation: Generation,
    sequence: u32,
    mutation: EffectMutation,
    expected: ExactPreviousState,
    planned: ExactPlannedState,
    payload: VersionedAdapterPlan,
    progress: OperationProgress,
}

/// Complete immutable operation intent supplied to both new and restored journals.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EffectOperationIntent {
    pub attempt: ReconcileAttemptId,
    pub resource: EffectResourceId,
    pub generation: Generation,
    pub sequence: u32,
    pub mutation: EffectMutation,
    pub expected: ExactPreviousState,
    pub planned: ExactPlannedState,
    pub payload: VersionedAdapterPlan,
}

impl EffectOperation {
    /// Creates immutable operation intent before any external side effect is permitted.
    pub fn prepare(
        identity: EffectOperationId,
        intent: EffectOperationIntent,
        prepared_at: LocalTimestamp,
    ) -> Result<Self, OperationTransitionError> {
        validate_mutation_states(intent.mutation, &intent.expected, &intent.planned)?;
        Ok(Self {
            identity,
            attempt: intent.attempt,
            resource: intent.resource,
            generation: intent.generation,
            sequence: intent.sequence,
            mutation: intent.mutation,
            expected: intent.expected,
            planned: intent.planned,
            payload: intent.payload,
            progress: OperationProgress::Prepared { prepared_at },
        })
    }

    /// Restores a persisted journal whose illegal timestamp combinations were excluded by its enum.
    pub fn restore(
        identity: EffectOperationId,
        intent: EffectOperationIntent,
        progress: OperationProgress,
    ) -> Result<Self, OperationTransitionError> {
        validate_mutation_states(intent.mutation, &intent.expected, &intent.planned)?;
        Ok(Self {
            identity,
            attempt: intent.attempt,
            resource: intent.resource,
            generation: intent.generation,
            sequence: intent.sequence,
            mutation: intent.mutation,
            expected: intent.expected,
            planned: intent.planned,
            payload: intent.payload,
            progress,
        })
    }

    pub fn identity(&self) -> &EffectOperationId {
        &self.identity
    }

    pub fn attempt(&self) -> &ReconcileAttemptId {
        &self.attempt
    }

    pub fn resource(&self) -> &EffectResourceId {
        &self.resource
    }

    pub fn generation(&self) -> Generation {
        self.generation
    }

    pub fn sequence(&self) -> u32 {
        self.sequence
    }

    pub fn mutation(&self) -> EffectMutation {
        self.mutation
    }

    pub fn expected(&self) -> &ExactPreviousState {
        &self.expected
    }

    pub fn planned(&self) -> &ExactPlannedState {
        &self.planned
    }

    pub fn payload(&self) -> &VersionedAdapterPlan {
        &self.payload
    }

    pub fn progress(&self) -> &OperationProgress {
        &self.progress
    }

    /// Records a verified adapter application without changing immutable intent.
    pub fn mark_applied(
        &mut self,
        applied_at: LocalTimestamp,
    ) -> Result<(), OperationTransitionError> {
        let OperationProgress::Prepared { prepared_at } = self.progress else {
            return Err(OperationTransitionError::ExpectedPrepared);
        };
        self.progress = OperationProgress::Applied {
            prepared_at,
            applied_at,
        };
        Ok(())
    }

    /// Records atomic ledger/status finalization after the Resource state was verified.
    pub fn finalize(
        &mut self,
        finalized_at: LocalTimestamp,
    ) -> Result<(), OperationTransitionError> {
        let OperationProgress::Applied {
            prepared_at,
            applied_at,
        } = self.progress
        else {
            return Err(OperationTransitionError::ExpectedApplied);
        };
        self.progress = OperationProgress::Finalized {
            prepared_at,
            applied_at,
            finalized_at,
        };
        Ok(())
    }

    /// Preserves immutable recovery authority when external state matches neither accepted state.
    pub fn require_recovery(
        &mut self,
        detected_at: LocalTimestamp,
    ) -> Result<(), OperationTransitionError> {
        let (prepared_at, applied_at) = match self.progress {
            OperationProgress::Prepared { prepared_at } => (prepared_at, None),
            OperationProgress::Applied {
                prepared_at,
                applied_at,
            } => (prepared_at, Some(applied_at)),
            OperationProgress::Finalized { .. } | OperationProgress::RecoveryRequired { .. } => {
                return Err(OperationTransitionError::CannotRequireRecovery);
            }
        };
        self.progress = OperationProgress::RecoveryRequired {
            prepared_at,
            applied_at,
            detected_at,
        };
        Ok(())
    }
}

/// Rejects mutation labels whose accepted previous/planned states encode a different operation.
fn validate_mutation_states(
    mutation: EffectMutation,
    expected: &ExactPreviousState,
    planned: &ExactPlannedState,
) -> Result<(), OperationTransitionError> {
    let valid = match (mutation, expected, planned) {
        (
            EffectMutation::Create,
            ExactPreviousState::Missing,
            ExactPlannedState::Present { .. },
        )
        | (
            EffectMutation::Delete,
            ExactPreviousState::Present { .. },
            ExactPlannedState::Missing,
        ) => true,
        (
            EffectMutation::Update | EffectMutation::Replace,
            ExactPreviousState::Present {
                native_identity: previous_native,
                managed_identity: previous_managed,
                ..
            },
            ExactPlannedState::Present {
                native_identity: planned_native,
                managed_identity: planned_managed,
                ..
            },
        ) => previous_native == planned_native && previous_managed == planned_managed,
        (EffectMutation::Create | EffectMutation::Update | EffectMutation::Replace, _, _)
        | (EffectMutation::Delete, _, _) => false,
    };
    if !valid {
        return Err(OperationTransitionError::InvalidMutationStates);
    }
    Ok(())
}

/// Role of a temporary external artifact in safe apply or compensation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactRole {
    Staging,
    Backup,
    Compensation,
}

/// Versioned locator interpreted only by the Resource adapter that created it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "locator", rename_all = "snake_case")]
pub enum VersionedResourceLocator {
    FilesystemPathV1(PathBuf),
}

/// Cleanup state is independent from operation finalization.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactState {
    Reserved,
    Retained,
    PendingCleanup,
    CleanupFailed,
}

/// Operation-owned temporary Resource with exact cleanup authority.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OperationArtifact {
    pub identity: ArtifactId,
    pub operation: EffectOperationId,
    pub role: ArtifactRole,
    pub locator: VersionedResourceLocator,
    pub expected_fingerprint: Fingerprint,
    pub state: ArtifactState,
}

/// Complete participant set required before any shared Resource mutation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CoordinationPlan {
    pub resources: BTreeSet<EffectResourceId>,
    pub participants: BTreeMap<EffectTargetId, CoordinationRequirement>,
}

impl CoordinationPlan {
    /// Refuses an empty plan because coordination exists only around actual Resource mutation.
    pub fn new(
        resources: BTreeSet<EffectResourceId>,
        participants: BTreeMap<EffectTargetId, CoordinationRequirement>,
    ) -> Result<Self, OperationTransitionError> {
        if resources.is_empty() {
            return Err(OperationTransitionError::EmptyCoordinationResources);
        }
        Ok(Self {
            resources,
            participants,
        })
    }
}

/// Durable progress of an immutable multi-Resource Target attempt.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconcileAttemptPhase {
    Prepared,
    Coordinated,
    Applied,
    Verified,
    Activated,
    Finalized,
    RecoveryRequired,
}

/// Immutable orchestration inputs and ordered operations for one Target generation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReconcileAttempt {
    identity: ReconcileAttemptId,
    target: EffectTargetId,
    generation: Generation,
    consumer_revision: crate::ConsumerRevisionId,
    target_projection: ProjectionDigest,
    resource_projections: BTreeSet<ProjectionDigest>,
    coordination: CoordinationPlan,
    operations: Vec<EffectOperationId>,
    phase: ReconcileAttemptPhase,
}

/// Complete immutable orchestration intent supplied when an attempt journal is prepared.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReconcileAttemptIntent {
    pub target: EffectTargetId,
    pub generation: Generation,
    pub consumer_revision: crate::ConsumerRevisionId,
    pub target_projection: ProjectionDigest,
    pub resource_projections: BTreeSet<ProjectionDigest>,
    pub coordination: CoordinationPlan,
    pub operations: Vec<EffectOperationId>,
}

impl ReconcileAttempt {
    /// Creates immutable orchestration input before the attempt journal is persisted.
    pub fn prepare(
        identity: ReconcileAttemptId,
        intent: ReconcileAttemptIntent,
    ) -> Result<Self, OperationTransitionError> {
        if intent.operations.is_empty() {
            return Err(OperationTransitionError::EmptyAttemptOperations);
        }
        Ok(Self {
            identity,
            target: intent.target,
            generation: intent.generation,
            consumer_revision: intent.consumer_revision,
            target_projection: intent.target_projection,
            resource_projections: intent.resource_projections,
            coordination: intent.coordination,
            operations: intent.operations,
            phase: ReconcileAttemptPhase::Prepared,
        })
    }

    pub fn identity(&self) -> &ReconcileAttemptId {
        &self.identity
    }

    pub fn target(&self) -> &EffectTargetId {
        &self.target
    }

    pub fn generation(&self) -> Generation {
        self.generation
    }

    pub fn consumer_revision(&self) -> &crate::ConsumerRevisionId {
        &self.consumer_revision
    }

    pub fn target_projection(&self) -> &ProjectionDigest {
        &self.target_projection
    }

    pub fn resource_projections(&self) -> &BTreeSet<ProjectionDigest> {
        &self.resource_projections
    }

    pub fn coordination(&self) -> &CoordinationPlan {
        &self.coordination
    }

    pub fn operations(&self) -> &[EffectOperationId] {
        &self.operations
    }

    pub fn phase(&self) -> ReconcileAttemptPhase {
        self.phase
    }

    /// Records that every required Consumer participant proved safe-to-mutate.
    pub fn mark_coordinated(&mut self) -> Result<(), OperationTransitionError> {
        self.transition(
            ReconcileAttemptPhase::Prepared,
            ReconcileAttemptPhase::Coordinated,
        )
    }

    /// Records that every ordered Resource operation completed its external apply.
    pub fn mark_applied(&mut self) -> Result<(), OperationTransitionError> {
        self.transition(
            ReconcileAttemptPhase::Coordinated,
            ReconcileAttemptPhase::Applied,
        )
    }

    /// Records that adapters proved the exact planned state for every Resource operation.
    pub fn mark_verified(&mut self) -> Result<(), OperationTransitionError> {
        self.transition(
            ReconcileAttemptPhase::Applied,
            ReconcileAttemptPhase::Verified,
        )
    }

    /// Records that every coordinated Consumer participant was reactivated.
    pub fn mark_activated(&mut self) -> Result<(), OperationTransitionError> {
        self.transition(
            ReconcileAttemptPhase::Verified,
            ReconcileAttemptPhase::Activated,
        )
    }

    /// Records the in-memory final phase submitted with the atomic business finalization.
    pub fn finalize(&mut self) -> Result<(), OperationTransitionError> {
        self.transition(
            ReconcileAttemptPhase::Activated,
            ReconcileAttemptPhase::Finalized,
        )
    }

    /// Moves any unfinished attempt into explicit recovery without changing immutable input.
    pub fn require_recovery(&mut self) -> Result<(), OperationTransitionError> {
        match self.phase {
            ReconcileAttemptPhase::Prepared
            | ReconcileAttemptPhase::Coordinated
            | ReconcileAttemptPhase::Applied
            | ReconcileAttemptPhase::Verified
            | ReconcileAttemptPhase::Activated => {
                self.phase = ReconcileAttemptPhase::RecoveryRequired;
                Ok(())
            }
            ReconcileAttemptPhase::Finalized | ReconcileAttemptPhase::RecoveryRequired => {
                Err(OperationTransitionError::CannotRequireAttemptRecovery)
            }
        }
    }

    /// Enforces the linear attempt state machine so callers cannot skip durable evidence stages.
    fn transition(
        &mut self,
        expected: ReconcileAttemptPhase,
        next: ReconcileAttemptPhase,
    ) -> Result<(), OperationTransitionError> {
        if self.phase != expected {
            return Err(OperationTransitionError::UnexpectedAttemptPhase {
                expected,
                actual: self.phase,
            });
        }
        self.phase = next;
        Ok(())
    }
}

/// Coordination receipt state keeps mutation and reactivation proofs distinct.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoordinationReceiptState {
    SafeToMutate,
    Reactivated,
}

/// Consumer adapter proof tied to one attempt's exact coordination contract.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CoordinationReceipt {
    pub target: EffectTargetId,
    pub contract: CoordinationContract,
    pub state: CoordinationReceiptState,
    pub proof: AdapterReceipt,
}

/// Scope association of an Audit event is explicit for global history.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "scope", rename_all = "snake_case")]
pub enum AuditScope {
    Global,
    Scoped(EffectScopeId),
}

/// Initiator identity distinguishes system policy from a Consumer acknowledgement.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "identity", rename_all = "snake_case")]
pub enum AuditInitiator {
    User,
    System,
    Consumer(ConsumerIdentity),
}

/// Generation association is explicit because many Audit events are not Desired-State changes.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "generation", rename_all = "snake_case")]
pub enum AuditGeneration {
    NotApplicable,
    At(Generation),
}

/// Versioned safe Audit payload that never participates in state reconstruction.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VersionedSafeAuditPayload {
    pub version: u32,
    pub payload: Value,
}

/// Append-only, independently prunable business history record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EffectAuditEvent {
    pub identity: AuditEventId,
    pub scope: AuditScope,
    pub subject_kind: String,
    pub subject_id: String,
    pub kind: String,
    pub generation: AuditGeneration,
    pub initiator: AuditInitiator,
    pub payload: VersionedSafeAuditPayload,
    pub occurred_at: LocalTimestamp,
}

/// Reports an invalid Operation or coordination state transition.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum OperationTransitionError {
    #[error("operation mutation does not match its exact previous and planned states")]
    InvalidMutationStates,
    #[error("operation must be Prepared before application")]
    ExpectedPrepared,
    #[error("operation must be Applied before finalization")]
    ExpectedApplied,
    #[error("a Finalized or already-recovering operation cannot enter recovery")]
    CannotRequireRecovery,
    #[error("coordination plan must cover at least one Resource")]
    EmptyCoordinationResources,
    #[error("a mutation attempt must contain at least one operation")]
    EmptyAttemptOperations,
    #[error("attempt phase transition expected {expected:?}, found {actual:?}")]
    UnexpectedAttemptPhase {
        expected: ReconcileAttemptPhase,
        actual: ReconcileAttemptPhase,
    },
    #[error("a Finalized or already-recovering attempt cannot enter recovery")]
    CannotRequireAttemptRecovery,
}
