//! Workspace-scoped declarative Skill State and safe filesystem reconciliation.

mod application_state;
mod filesystem;
mod identity;
mod mcp;
mod mcp_reconcile;
mod planner;
mod ports;
mod reconcile;
mod state;
mod surface;

#[cfg(test)]
mod tests;

pub use application_state::{
    McpApplicationState, McpApplicationStateInput, derive_mcp_application_state,
};
pub use filesystem::{
    FilesystemEffectError, FilesystemSurfaceAdapter, MARKER_FILE_NAME, ManagedSkillMarker,
    OperationPaths, RecoveryDecision, ScanDiagnostic, SurfaceScan,
};
pub use identity::{
    AppliedFingerprint, ConsumerId, Digest, EffectOperationId, Generation, ManagedIdentity,
    SkillName, SkillSelectionKey, SourceKind, SourceVersion, SurfaceKey,
};
pub use mcp::{DesiredMcpState, McpHttpHeaderEffect, McpHttpTransportEffect, McpSelectionKey};
pub use mcp_reconcile::{McpRenderError, McpRenderer, RenderedMcpFile, reconcile_mcp_surface};
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
    ConsumerCoordination, DescriptorMergeError, FilesystemMcpSurface, FilesystemSkillSurface,
    MaterializationFormat, OPENCODE_MCP_COMPLETE_FILE_RELATIVE_PATH, SurfaceDeclaration,
    SurfaceDescriptorSet, SurfaceLifecycle, SurfacePath,
};
