use crate::{
    ConsumerId, DesiredSkillState, EffectOperation, Generation, ManagedIdentity, ManagedSkill,
    SkillSelectionKey, SourceVersion, SurfaceKey, SurfaceStatus, WorkspaceEffect,
    WorkspaceEffectSpec,
};
use ora_domain::WorkspaceId;
use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;

/// Preserves concrete persistence failures across the transport-independent Effect boundary.
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

/// Result of replacing a complete desired specification using generation CAS.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReplaceEffectOutcome {
    Unchanged(WorkspaceEffect),
    Replaced(WorkspaceEffect),
    Conflict {
        expected_generation: Generation,
        current_generation: Generation,
    },
    SourceUnavailable {
        selection_key: SkillSelectionKey,
    },
}

/// Atomic ledger change committed with operation finalization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LedgerTransition {
    Upsert(ManagedSkill),
    Replace {
        previous_identity: ManagedIdentity,
        next: ManagedSkill,
    },
    Delete {
        managed_identity: ManagedIdentity,
    },
}

/// Defines durable state transitions required by desired CRUD and reconciliation.
///
/// Implementations must make desired replacement and reconcile-request upsert one transaction,
/// and must never infer managed ownership from filesystem observations.
pub trait EffectRepository {
    /// Loads the latest complete desired specification or generation zero with an empty spec.
    fn load_workspace_effect(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<WorkspaceEffect, RepositoryError>;

    /// Replaces the full normalized spec and enqueues affected surfaces atomically.
    fn replace_workspace_effect(
        &self,
        workspace_id: &WorkspaceId,
        expected_generation: Generation,
        spec: WorkspaceEffectSpec,
        updated_at: i64,
    ) -> Result<ReplaceEffectOutcome, RepositoryError>;

    /// Loads every live ownership ledger for one physical surface.
    fn load_managed_skills(
        &self,
        workspace_id: &WorkspaceId,
        surface_key: &SurfaceKey,
    ) -> Result<Vec<ManagedSkill>, RepositoryError>;

    /// Creates or advances one managed ledger after its exact file operation is applied.
    fn save_managed_skill(&self, managed: ManagedSkill) -> Result<(), RepositoryError>;

    /// Terminates one ownership lifecycle after safe cleanup or proven absence.
    fn delete_managed_skill(
        &self,
        managed_identity: &ManagedIdentity,
    ) -> Result<(), RepositoryError>;

    /// Loads current status independently from desired generation.
    fn load_surface_status(
        &self,
        workspace_id: &WorkspaceId,
        surface_key: &SurfaceKey,
    ) -> Result<Option<SurfaceStatus>, RepositoryError>;

    /// Replaces current status and conditions without mutating desired generation.
    fn save_surface_status(&self, status: SurfaceStatus) -> Result<(), RepositoryError>;

    /// Durably records intent before any corresponding filesystem mutation.
    fn prepare_operation(&self, operation: EffectOperation) -> Result<(), RepositoryError>;

    /// Persists the next phase of an already prepared operation.
    fn save_operation(&self, operation: EffectOperation) -> Result<(), RepositoryError>;

    /// Commits the ownership-ledger transition and `Finalized` phase in one database transaction.
    fn finalize_operation(
        &self,
        operation: EffectOperation,
        transition: LedgerTransition,
    ) -> Result<(), RepositoryError>;

    /// Releases durable recovery authority after the adapter proves every artifact absent.
    fn complete_operation_cleanup(
        &self,
        operation_id: &crate::EffectOperationId,
    ) -> Result<(), RepositoryError>;

    /// Loads unfinished operations in deterministic preparation order for startup recovery.
    fn load_unfinished_operations(&self) -> Result<Vec<EffectOperation>, RepositoryError>;

    /// Persists per-consumer readiness independently from surface file application.
    fn save_consumer_status(&self, status: crate::ConsumerStatus) -> Result<(), RepositoryError>;

    /// Coalesces an explicit retry wakeup at the current Desired generation.
    fn retry_surface(
        &self,
        workspace_id: &WorkspaceId,
        surface_key: &SurfaceKey,
        requested_at: i64,
    ) -> Result<bool, RepositoryError>;
}

/// Generates fresh, random ownership identities for new lifecycles.
pub trait ManagedIdentityGenerator {
    fn generate_managed_identity(&self) -> ManagedIdentity;
}

/// Uses cryptographically random UUIDs for production ownership identities.
#[derive(Clone, Copy, Debug, Default)]
pub struct UuidManagedIdentityGenerator;

impl ManagedIdentityGenerator for UuidManagedIdentityGenerator {
    fn generate_managed_identity(&self) -> ManagedIdentity {
        ManagedIdentity::random()
    }
}

/// A stable read handle for one already validated source revision.
#[derive(Clone, Debug)]
pub struct SourceSnapshot {
    pub state: DesiredSkillState,
    pub package_root: PathBuf,
    _lease: Option<Arc<tempfile::TempDir>>,
}

impl SourceSnapshot {
    /// Borrows an adapter-coordinated immutable path for the lifetime of the returned value.
    pub fn borrowed(state: DesiredSkillState, package_root: PathBuf) -> Self {
        Self {
            state,
            package_root,
            _lease: None,
        }
    }

    /// Copies a mutable catalog directory into an owned, link-free snapshot for one operation.
    pub fn copy_from(
        state: DesiredSkillState,
        package_root: &std::path::Path,
    ) -> Result<Self, SourceError> {
        let lease = tempfile::Builder::new()
            .prefix("ora-effect-source-")
            .tempdir()
            .map_err(|source| SourceError::Provider {
                source: Box::new(source),
            })?;
        let snapshot_root = lease.path().join("package");
        ora_utils::directory::copy_directory(
            package_root,
            &snapshot_root,
            &[std::ffi::OsStr::new(crate::MARKER_FILE_NAME)],
        )
        .map_err(|source| SourceError::Provider {
            source: Box::new(source),
        })?;
        Ok(Self {
            state,
            package_root: snapshot_root,
            _lease: Some(Arc::new(lease)),
        })
    }
}

/// Reports source disappearance, drift, or adapter failures without exposing package content.
#[derive(Debug, Error)]
pub enum SourceError {
    #[error("source revision is unavailable")]
    Unavailable,
    #[error("source revision no longer matches its published state")]
    IntegrityMismatch,
    #[error("source provider failed")]
    Provider {
        #[source]
        source: Box<dyn Error + Send + Sync + 'static>,
    },
}

/// Supplies validated, immutable-enough snapshots for one materialization attempt.
///
/// Implementations must revalidate manifest name and digest on every open and must not expose a
/// snapshot that can mix bytes from two source revisions during one copy.
pub trait SourceProvider {
    /// Opens the exact desired revision or reports it unavailable without silently substituting a
    /// newer revision.
    fn open_snapshot(&self, desired: &DesiredSkillState) -> Result<SourceSnapshot, SourceError>;

    /// Loads the newest active state for propagation by stable selection identity.
    fn load_active_state(
        &self,
        selection_key: &SkillSelectionKey,
    ) -> Result<DesiredSkillState, SourceError>;

    /// Confirms an immutable plugin version still maps to the expected revision when applicable.
    fn verify_version(
        &self,
        selection_key: &SkillSelectionKey,
        version: &SourceVersion,
    ) -> Result<(), SourceError>;
}

/// Outcome of asking all relevant consumers to reach a safe mutation boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoordinationOutcome {
    Ready,
    WaitingForIdle,
}

/// Reports a consumer coordination failure separately from filesystem application.
#[derive(Debug, Error)]
#[error("consumer coordination failed")]
pub struct CoordinationError {
    #[source]
    source: Box<dyn Error + Send + Sync + 'static>,
}

impl CoordinationError {
    pub fn new(source: impl Error + Send + Sync + 'static) -> Self {
        Self {
            source: Box::new(source),
        }
    }
}

/// Coordinates runtime consumers around mutation of one shared physical surface.
///
/// Implementations block new turns before returning `Ready`, wait only for consumers whose policy
/// requires it, and resume each consumer independently after the filesystem reaches a stable state.
pub trait ConsumerCoordinator {
    /// Attempts to quiesce the supplied consumer snapshot without interrupting active turns.
    fn quiesce(
        &self,
        surface_key: &SurfaceKey,
        consumers: &[ConsumerId],
    ) -> Result<CoordinationOutcome, CoordinationError>;

    /// Restores one consumer independently so a failure cannot hold healthy siblings offline.
    fn resume(
        &self,
        surface_key: &SurfaceKey,
        consumer: &ConsumerId,
        generation: Generation,
    ) -> Result<(), CoordinationError>;
}
