use crate::{
    AppliedFingerprint, ConsumerId, Digest, EffectOperationId, Generation, ManagedIdentity,
    SkillName, SkillSelectionKey, SourceVersion, SurfaceKey,
};
use ora_domain::{Namespace, WorkspaceId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use thiserror::Error;

/// Describes the origin-specific portion of exact Skill state identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SkillSource {
    Local {
        namespace: Namespace,
        version: SourceVersion,
    },
    Plugin {
        namespace: Namespace,
        version: SourceVersion,
    },
    Preserved {
        workspace_id: WorkspaceId,
    },
}

impl SkillSource {
    /// Returns a stable selection key only for catalog-backed sources.
    pub fn selection_key(&self, name: SkillName) -> Option<SkillSelectionKey> {
        match self {
            Self::Local { namespace, .. } => Some(SkillSelectionKey::new(
                crate::SourceKind::Local,
                namespace.clone(),
                name,
            )),
            Self::Plugin { namespace, .. } => Some(SkillSelectionKey::new(
                crate::SourceKind::Plugin,
                namespace.clone(),
                name,
            )),
            Self::Preserved { .. } => None,
        }
    }

    /// Returns the exact revision for a catalog-backed source.
    pub fn version(&self) -> Option<&SourceVersion> {
        match self {
            Self::Local { version, .. } | Self::Plugin { version, .. } => Some(version),
            Self::Preserved { .. } => None,
        }
    }
}

/// The minimal value shared by desired, managed, observed, and preserved Skill state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SkillState {
    pub name: SkillName,
    pub skill_md_digest: Digest,
    pub source: SkillSource,
}

/// A catalog-backed state that is legal inside a Workspace desired specification.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DesiredSkillState(SkillState);

impl DesiredSkillState {
    /// Refuses preserved state so it can never accidentally become desired or managed.
    pub fn try_new(state: SkillState) -> Result<Self, StateError> {
        if matches!(state.source, SkillSource::Preserved { .. }) {
            return Err(StateError::PreservedCannotBeDesired);
        }
        Ok(Self(state))
    }

    /// Returns the exact catalog state selected by the Workspace.
    pub fn state(&self) -> &SkillState {
        &self.0
    }

    /// Consumes the wrapper when materialization needs an owned snapshot.
    pub fn into_state(self) -> SkillState {
        self.0
    }
}

/// The normalized complete desired specification for one Workspace generation.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkspaceEffectSpec {
    pub skills: BTreeMap<SkillSelectionKey, DesiredSkillState>,
}

impl WorkspaceEffectSpec {
    /// Validates that each map key describes the exact value it indexes.
    pub fn normalized(
        skills: impl IntoIterator<Item = DesiredSkillState>,
    ) -> Result<Self, StateError> {
        let mut normalized = BTreeMap::new();
        for desired in skills {
            let state = desired.state();
            let key = state
                .source
                .selection_key(state.name.clone())
                .ok_or(StateError::PreservedCannotBeDesired)?;
            if normalized.insert(key.clone(), desired).is_some() {
                return Err(StateError::DuplicateSelection(key));
            }
        }
        Ok(Self { skills: normalized })
    }
}

/// A Workspace desired specification and its compare-and-swap generation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkspaceEffect {
    pub workspace_id: WorkspaceId,
    pub generation: Generation,
    pub spec: WorkspaceEffectSpec,
}

/// Availability of an active catalog source revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceState {
    Available(DesiredSkillState),
    Unavailable {
        selection_key: SkillSelectionKey,
        message: String,
    },
}

/// Database ownership ledger for one materialized Skill locator.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ManagedSkill {
    pub managed_identity: ManagedIdentity,
    pub workspace_id: WorkspaceId,
    pub surface_key: SurfaceKey,
    pub selection_key: SkillSelectionKey,
    pub locator: String,
    pub target_name: SkillName,
    pub state: DesiredSkillState,
    pub applied_fingerprint: AppliedFingerprint,
    pub applied_generation: Generation,
}

/// A legal Skill found during one live surface scan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ObservedSkill {
    Managed {
        locator: String,
        state: DesiredSkillState,
        managed_identity: ManagedIdentity,
        fingerprint: AppliedFingerprint,
    },
    Preserved {
        locator: String,
        state: SkillState,
    },
}

/// High-level progression of one surface reconciliation state machine.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfacePhase {
    Pending,
    WaitingForIdle,
    Quiescing,
    Applying,
    Resuming,
    Current,
    Degraded,
    Retiring,
    RecoveryRequired,
}

/// Current status of one physical Skill surface.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SurfaceStatus {
    pub workspace_id: WorkspaceId,
    pub surface_key: SurfaceKey,
    pub desired_generation: Generation,
    pub observed_generation: Generation,
    pub applied_generation: Generation,
    pub phase: SurfacePhase,
    pub revision: u64,
    pub updated_at: i64,
    pub conditions: Vec<Condition>,
}

/// Per-consumer readiness can lag behind files and other consumers independently.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConsumerStatus {
    pub surface_key: SurfaceKey,
    pub consumer_id: ConsumerId,
    pub ready_generation: Generation,
    pub phase: SurfacePhase,
    pub revision: u64,
    pub updated_at: i64,
    pub conditions: Vec<Condition>,
}

/// Identifies the precise desired, managed, surface, or consumer subject of a condition.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConditionSubject {
    DesiredSkill { selection_key: SkillSelectionKey },
    ManagedSkill { managed_identity: ManagedIdentity },
    Surface { surface_key: SurfaceKey },
    Consumer { consumer_id: ConsumerId },
}

/// Stable machine-readable reasons for a surface not being fully current.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionReason {
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
}

impl ConditionReason {
    /// Maps stable reasons to retry behavior without requiring callers to parse messages.
    pub fn retry_policy(self) -> RetryPolicy {
        match self {
            Self::TransientIo | Self::MaterializationFailed | Self::ConsumerResumeFailed => {
                RetryPolicy::Backoff
            }
            Self::RecoveryRequired => RetryPolicy::Manual,
            Self::NoConsumers
            | Self::IncompatibleSurfaceDeclarations
            | Self::DesiredCollision
            | Self::PreservedConflict
            | Self::OwnershipConflict
            | Self::DriftConflict
            | Self::SourceUnavailable
            | Self::PathUnsafe
            | Self::ScanFailed
            | Self::WaitingForIdle => RetryPolicy::OnChange,
        }
    }
}

/// Determines which external fact may schedule another reconcile attempt.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryPolicy {
    OnChange,
    Backoff,
    Manual,
}

/// A current, structured explanation for a blocked or degraded subject.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Condition {
    pub subject: ConditionSubject,
    pub reason: ConditionReason,
    pub message: String,
    pub first_occurred_at: i64,
    pub last_occurred_at: i64,
    pub failed_generation: Generation,
    pub retry_policy: RetryPolicy,
}

impl Condition {
    /// Creates a condition while deriving retry policy from its stable reason.
    pub fn new(
        subject: ConditionSubject,
        reason: ConditionReason,
        message: impl Into<String>,
        occurred_at: i64,
        failed_generation: Generation,
    ) -> Self {
        Self {
            subject,
            reason,
            message: message.into(),
            first_occurred_at: occurred_at,
            last_occurred_at: occurred_at,
            failed_generation,
            retry_policy: reason.retry_policy(),
        }
    }
}

/// Durable kinds of per-resource filesystem mutation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectOperationKind {
    Create,
    Update,
    Replace,
    Delete,
}

/// Durable transaction phase spanning SQLite and filesystem boundaries.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectOperationPhase {
    Prepared,
    Applied,
    Finalized,
}

/// Expected disk identity used to make crash recovery decisions without guessing.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "fingerprint", rename_all = "snake_case")]
pub enum OperationState {
    Missing,
    Present(AppliedFingerprint),
}

/// Complete durable intent for one filesystem mutation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EffectOperation {
    pub operation_id: EffectOperationId,
    pub generation: Generation,
    pub workspace_id: WorkspaceId,
    pub surface_key: SurfaceKey,
    pub locator: String,
    pub target_name: SkillName,
    pub kind: EffectOperationKind,
    pub phase: EffectOperationPhase,
    pub previous_state: OperationState,
    pub planned_state: OperationState,
    pub previous_identity: Option<ManagedIdentity>,
    pub planned_identity: Option<ManagedIdentity>,
    pub previous_managed: Option<ManagedSkill>,
    pub planned_desired: Option<DesiredSkillState>,
    pub staging_path: PathBuf,
    pub backup_path: PathBuf,
}

/// Reports a specification invariant violation.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum StateError {
    #[error("preserved Skill state cannot enter desired state")]
    PreservedCannotBeDesired,
    #[error("desired specification contains duplicate selection {0:?}")]
    DuplicateSelection(SkillSelectionKey),
}
