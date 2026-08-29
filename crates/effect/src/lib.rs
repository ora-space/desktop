//! Workspace-scoped declarative Skill State and safe filesystem reconciliation.

mod agent_target;
mod filesystem;
mod identity;
mod planner;
mod ports;
mod reconcile;
mod state;
mod surface;

#[cfg(test)]
mod tests;

pub use agent_target::{
    AgentCapabilityRevision, AgentPluginId, AgentTarget, AgentTargetCondition,
    AgentTargetConditionAttachment, AgentTargetConditionReason, AgentTargetConditionSubject,
    AgentTargetId, AgentTargetIdentity, AgentTargetLifecycle, AgentTargetPhase,
    AgentTargetReconcileRequest, AgentTargetReconcileState, AgentTargetRecord,
    AgentTargetRepository, AgentTargetRepositoryError, AgentTargetStatus, AgentTargetWakeReason,
    ConditionImpact, initial_agent_target_status,
};
pub use filesystem::{
    FilesystemEffectError, FilesystemSurfaceAdapter, MARKER_FILE_NAME, ManagedSkillMarker,
    OperationPaths, RecoveryDecision, ScanDiagnostic, SurfaceScan,
};
pub use identity::{
    AppliedFingerprint, ConsumerId, Digest, EffectOperationId, Generation, ManagedIdentity,
    SkillName, SkillSelectionKey, SourceKind, SourceVersion, SurfaceKey,
};
pub use planner::{
    PlanOperation, PlanOperationKind, Planner, PlannerInput, ReconcilePlan, TargetObservation,
};
pub use ports::{
    ConsumerCoordinator, CoordinationError, CoordinationOutcome, EffectRepository,
    LedgerTransition, ManagedIdentityGenerator, ReplaceEffectOutcome, RepositoryError, SourceError,
    SourceProvider, SourceSnapshot, UuidManagedIdentityGenerator,
};
pub use reconcile::{ReconcileError, ReconcileOutcome, Reconciler};
pub use state::{
    Condition, ConditionReason, ConditionSubject, ConsumerStatus, DesiredSkillState,
    EffectOperation, EffectOperationKind, EffectOperationPhase, ManagedSkill, ObservedSkill,
    OperationState, RetryPolicy, SkillSource, SkillState, SourceState, SurfacePhase, SurfaceStatus,
    WorkspaceEffect, WorkspaceEffectSpec,
};
pub use surface::{
    ConsumerCoordination, DescriptorMergeError, FilesystemSkillSurface, MaterializationFormat,
    SurfaceDescriptorSet, SurfaceLifecycle, SurfacePath,
};
