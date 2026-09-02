//! Generic Target Effect convergence with independent Resource ownership and recovery.

mod desired;
mod identity;
mod operation;
mod planner;
mod ports;
mod projection;
mod reconcile;
mod status;
mod target;

#[cfg(test)]
mod tests;

pub use desired::{
    CapabilityRequirement, DesiredEffect, DesiredState, DesiredStateError, EffectPublication,
    EffectRevision, EffectSource, EffectSourceLifecycle, RevisionAvailability, SkillDefinition,
    SkillParameters, SkillSourceKey, SkillSourceKind, StableReason, TargetInclusion,
    TargetSelector, ValidatedEffectDefinition, ValidatedEffectParameters,
};
pub use identity::{
    ArtifactId, AuditEventId, ConditionId, ConsumerAdapterIdentity, ConsumerIdentity, ConsumerKind,
    ConsumerRevisionId, DesiredEffectIdentity, Digest, EffectKind, EffectOperationId,
    EffectResourceId, EffectRevisionId, EffectScopeId, EffectSourceIdentity, EffectTargetId,
    FencingToken, Fingerprint, Generation, IdentityError, ManagedIdentity, NativeResourceIdentity,
    ProjectionDigest, ReconcileAttemptId, ResourceAdapterIdentity, ResourceKey, RetryAttempt,
    SkillName, SourceRevisionKey, StatusVersion, WorkerIdentity,
};
pub use operation::{
    ArtifactRole, ArtifactState, AuditGeneration, AuditInitiator, AuditScope, CoordinationPlan,
    CoordinationReceipt, CoordinationReceiptState, EffectAuditEvent, EffectMutation,
    EffectOperation, EffectOperationIntent, ExactPlannedState, ExactPreviousState,
    FilesystemOperationPlan, JsonMergeOperationPlan, OperationArtifact, OperationProgress,
    OperationTransitionError, ReconcileAttempt, ReconcileAttemptIntent, ReconcileAttemptPhase,
    VersionedAdapterPlan, VersionedResourceLocator, VersionedSafeAuditPayload,
};
pub use planner::{
    EffectPlanner, PlannedMutation, PlannedResourceChange, PlannerError, PlanningResult,
    ResourcePlan, ResourcePlanningInput, TargetPlanningInput,
};
pub use ports::{
    ApplyReceipt, AttemptFinalization, CleanupReceipt, ConsumerAdapter, ConsumerAdapterError,
    EffectRepository, PreparedOperation, ProjectionCommit, ReconcileSnapshot,
    RelatedTargetSnapshot, ReplaceDesiredStateOutcome, RepositoryError, ResourceAdapter,
    ResourceAdapterError, VerificationReceipt,
};
pub use projection::{
    ManagedItem, ObservedItem, OwnershipEvidence, PreservedItem, ResolvedMaterialization,
    ResourceObservation, ResourceProjection, ResourceRequirement, SkillMaterializationInput,
    TargetProjection, VersionedMaterializationInput,
};
pub use reconcile::{
    EffectReconciler, ReconcileError, ReconcileOutcome, recovery_condition,
    resource_recovery_condition,
};
pub use status::{
    AdapterReceipt, AttemptReadinessReceipt, BackoffPolicy, ConditionGeneration, ConditionImpact,
    ConditionOwner, ConditionProposal, ConditionRetry, ConditionSubject, EffectCondition,
    LocalTimestamp, ReadinessReceipt, ReconcileClaim, ReconcileRequest, ReconcileRequestState,
    ReconcileStage, ResourceClaim, ResourcePhase, ResourceStatus, ResumeTrigger,
    SafeConditionDetails, StableConditionCode, StatusTransitionError, TargetIssueState,
    TargetPhase, TargetProgress, TargetStatus, WakeReason,
};
pub use target::{
    CapabilitySet, Consumer, ConsumerDeclaration, ConsumerLifecycle, ConsumerRevision,
    CoordinationContract, CoordinationRequirement, DeclarationError, EffectResource, EffectScope,
    EffectScopeLifecycle, EffectTarget, FilesystemDirectoryDescriptor, FilesystemFileDescriptor,
    FilesystemResourceTemplate, MaterializationContract, MaterializationFormat, ResourceLifecycle,
    ResourcePath, TargetDeclaration, TargetLifecycle, TargetResourceBinding,
    VersionedResourceDescriptor,
};
