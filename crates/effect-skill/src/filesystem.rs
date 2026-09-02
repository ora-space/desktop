use ora_effect::{
    AdapterReceipt, ApplyReceipt, ArtifactId, ArtifactRole, ArtifactState, CleanupReceipt,
    EffectMutation, EffectOperation, EffectOperationId, EffectOperationIntent, EffectResource,
    EffectResourceId, ExactPlannedState, ExactPreviousState, FilesystemOperationPlan, Fingerprint,
    LocalTimestamp, ManagedIdentity, NativeResourceIdentity, OperationArtifact, PlannedMutation,
    PreparedOperation, ReconcileAttemptId, ResourceAdapter, ResourceAdapterError,
    ResourceObservation, VerificationReceipt, VersionedAdapterPlan, VersionedMaterializationInput,
    VersionedResourceDescriptor, VersionedResourceLocator,
};
use ora_utils::directory::DirectoryTreeError;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use thiserror::Error;

mod operation;
mod path;

use operation::{
    apply_receipt, expected_path, fingerprint, planned_path, read_marker, restore_backup, stage,
    state_matches_expected, state_matches_planned,
};
use path::{
    RootAccess, ensure_operation_paths_are_scoped, resolve_declared_root, resolve_resource_root,
    resource_root,
};

pub const MARKER_FILE_NAME: &str = ".ora-managed.json";
pub(super) const OPERATIONS_DIR_NAME: &str = ".ora-effect-operations";
pub(super) const MARKER_SCHEMA_VERSION: u32 = 1;

/// On-disk ownership claim that is useful only when matched with the Resource ledger.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ManagedItemMarker {
    pub schema_version: u32,
    pub resource: EffectResourceId,
    pub managed_identity: ManagedIdentity,
}

impl ManagedItemMarker {
    /// Creates the current marker schema for a newly staged Managed Item.
    pub fn current(resource: EffectResourceId, managed_identity: ManagedIdentity) -> Self {
        Self {
            schema_version: MARKER_SCHEMA_VERSION,
            resource,
            managed_identity,
        }
    }
}

/// Local filesystem implementation of the versioned directory Resource contract.
#[derive(Clone, Copy, Debug, Default)]
pub struct SkillDirectoryResourceAdapter;

impl SkillDirectoryResourceAdapter {
    /// Converts a pure mutation proposal into immutable adapter intent and artifact authority.
    pub fn prepare_operation(
        &self,
        resource: &EffectResource,
        attempt: ReconcileAttemptId,
        generation: ora_effect::Generation,
        sequence: u32,
        mutation: PlannedMutation,
        prepared_at: LocalTimestamp,
    ) -> Result<PreparedOperation, SkillDirectoryError> {
        let resource_root = resource_root(resource)?;
        let VersionedResourceDescriptor::FilesystemDirectoryV1(descriptor) = &resource.descriptor
        else {
            return Err(SkillDirectoryError::UnsupportedResourceDescriptor);
        };
        let operation_id = EffectOperationId::random();
        let operation_root = resource_root
            .join(OPERATIONS_DIR_NAME)
            .join(operation_id.as_str());
        let staging_path = operation_root.join("staging");
        let backup_path = operation_root.join("backup");
        let source_root = mutation.input.as_ref().map(|input| {
            let VersionedMaterializationInput::SkillDirectoryV1(input) = input;
            input.package_root.clone()
        });
        let payload = VersionedAdapterPlan::FilesystemDirectoryV1(FilesystemOperationPlan {
            workspace_root: descriptor.workspace_root.clone(),
            resource_relative_path: descriptor.relative_path.clone(),
            resource_root,
            source_root,
            staging_path: staging_path.clone(),
            backup_path: backup_path.clone(),
        });
        let mut artifacts = Vec::new();
        if let ExactPlannedState::Present { fingerprint, .. } = &mutation.planned {
            artifacts.push(OperationArtifact {
                identity: ArtifactId::random(),
                operation: operation_id.clone(),
                role: ArtifactRole::Staging,
                locator: VersionedResourceLocator::FilesystemPathV1(staging_path),
                expected_fingerprint: fingerprint.clone(),
                state: ArtifactState::Reserved,
            });
        }
        if let ExactPreviousState::Present { fingerprint, .. } = &mutation.expected {
            artifacts.push(OperationArtifact {
                identity: ArtifactId::random(),
                operation: operation_id.clone(),
                role: ArtifactRole::Backup,
                locator: VersionedResourceLocator::FilesystemPathV1(backup_path),
                expected_fingerprint: fingerprint.clone(),
                state: ArtifactState::Reserved,
            });
        }
        Ok(PreparedOperation {
            operation: EffectOperation::prepare(
                operation_id,
                EffectOperationIntent {
                    attempt,
                    resource: resource.identity.clone(),
                    generation,
                    sequence,
                    mutation: mutation.mutation,
                    expected: mutation.expected,
                    planned: mutation.planned,
                    payload,
                },
                prepared_at,
            )?,
            artifacts,
        })
    }

    /// Observes a filesystem directory without creating a missing Resource as a read side effect.
    fn observe_resource(
        self,
        resource: &EffectResource,
    ) -> Result<ResourceObservation, SkillDirectoryError> {
        let root = resolve_resource_root(resource, RootAccess::Observe)?;
        let Some(root) = root else {
            return Ok(ResourceObservation {
                resource: resource.identity.clone(),
                items: BTreeMap::new(),
                fingerprint: Fingerprint::sha256(&[]),
            });
        };
        let mut items = BTreeMap::new();
        for entry in fs::read_dir(&root).map_err(|source| SkillDirectoryError::Io {
            path: root.clone(),
            source,
        })? {
            let entry = entry.map_err(|source| SkillDirectoryError::Io {
                path: root.clone(),
                source,
            })?;
            let entry_name = entry.file_name().to_string_lossy().into_owned();
            if entry_name == OPERATIONS_DIR_NAME {
                continue;
            }
            let native_identity = NativeResourceIdentity::parse(entry_name.clone())
                .map_err(|_| SkillDirectoryError::InvalidNativeIdentity(entry_name))?;
            let path = entry.path();
            let metadata =
                fs::symlink_metadata(&path).map_err(|source| SkillDirectoryError::Io {
                    path: path.clone(),
                    source,
                })?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(SkillDirectoryError::UnsupportedResourceEntry { path });
            }
            let fingerprint = fingerprint(&path)?;
            let ownership_evidence = read_marker(&path)
                .filter(|marker| {
                    marker.schema_version == MARKER_SCHEMA_VERSION
                        && marker.resource == resource.identity
                })
                .map_or(
                    ora_effect::OwnershipEvidence::NoOwnershipEvidence,
                    |marker| ora_effect::OwnershipEvidence::Claims(marker.managed_identity),
                );
            items.insert(
                native_identity.clone(),
                ora_effect::ObservedItem {
                    native_identity,
                    fingerprint,
                    ownership_evidence,
                },
            );
        }
        let summary = serde_json::to_vec(&items).map_err(SkillDirectoryError::MarkerJson)?;
        Ok(ResourceObservation {
            resource: resource.identity.clone(),
            items,
            fingerprint: Fingerprint::sha256(&summary),
        })
    }

    /// Applies a journal only when disk equals its exact expected state or already-planned state.
    fn apply_operation(
        self,
        operation: &EffectOperation,
    ) -> Result<ApplyReceipt, SkillDirectoryError> {
        let VersionedAdapterPlan::FilesystemDirectoryV1(plan) = operation.payload() else {
            return Err(SkillDirectoryError::UnsupportedAdapterPlan);
        };
        ensure_operation_paths_are_scoped(plan)?;
        let resolved_root = resolve_declared_root(
            &plan.workspace_root,
            &plan.resource_relative_path,
            RootAccess::Mutate,
        )?
        .ok_or(SkillDirectoryError::UnsafeOperationPath)?;
        if resolved_root != plan.resource_root {
            return Err(SkillDirectoryError::UnsafeOperationPath);
        }
        if state_matches_planned(operation, plan)? {
            return Ok(apply_receipt(operation));
        }
        if !state_matches_expected(operation, plan)? {
            return Err(SkillDirectoryError::RecoveryRequired {
                operation: operation.identity().clone(),
            });
        }

        match operation.mutation() {
            EffectMutation::Create => {
                stage(operation, plan)?;
                let target = planned_path(operation, plan)?;
                if target.exists() {
                    return Err(SkillDirectoryError::TargetOccupied { path: target });
                }
                fs::rename(&plan.staging_path, &target).map_err(|source| {
                    SkillDirectoryError::Io {
                        path: target,
                        source,
                    }
                })?;
            }
            EffectMutation::Update | EffectMutation::Replace => {
                stage(operation, plan)?;
                let previous = expected_path(operation, plan)?;
                let target = planned_path(operation, plan)?;
                fs::create_dir_all(
                    plan.backup_path
                        .parent()
                        .ok_or(SkillDirectoryError::UnsafeOperationPath)?,
                )
                .map_err(|source| SkillDirectoryError::Io {
                    path: plan.backup_path.clone(),
                    source,
                })?;
                fs::rename(&previous, &plan.backup_path).map_err(|source| {
                    SkillDirectoryError::Io {
                        path: previous.clone(),
                        source,
                    }
                })?;
                if target != previous && target.exists() {
                    restore_backup(&plan.backup_path, &previous);
                    return Err(SkillDirectoryError::TargetOccupied { path: target });
                }
                if let Err(source) = fs::rename(&plan.staging_path, &target) {
                    restore_backup(&plan.backup_path, &previous);
                    return Err(SkillDirectoryError::Io {
                        path: target,
                        source,
                    });
                }
            }
            EffectMutation::Delete => {
                let previous = expected_path(operation, plan)?;
                fs::create_dir_all(
                    plan.backup_path
                        .parent()
                        .ok_or(SkillDirectoryError::UnsafeOperationPath)?,
                )
                .map_err(|source| SkillDirectoryError::Io {
                    path: plan.backup_path.clone(),
                    source,
                })?;
                fs::rename(&previous, &plan.backup_path).map_err(|source| {
                    SkillDirectoryError::Io {
                        path: previous,
                        source,
                    }
                })?;
            }
        }
        Ok(apply_receipt(operation))
    }

    /// Verifies exact planned state and never treats a merely similar directory as completion.
    fn verify_operation(
        self,
        operation: &EffectOperation,
    ) -> Result<VerificationReceipt, SkillDirectoryError> {
        let VersionedAdapterPlan::FilesystemDirectoryV1(plan) = operation.payload() else {
            return Err(SkillDirectoryError::UnsupportedAdapterPlan);
        };
        if !state_matches_planned(operation, plan)? {
            return Err(SkillDirectoryError::VerificationFailed {
                operation: operation.identity().clone(),
            });
        }
        Ok(VerificationReceipt {
            operation: operation.identity().clone(),
            proof: AdapterReceipt {
                version: 1,
                payload: json!({ "state": "planned" }),
            },
        })
    }

    /// Cleans only the exact path/fingerprint pair granted by durable Artifact authority.
    fn cleanup_artifact(
        self,
        artifact: &OperationArtifact,
    ) -> Result<CleanupReceipt, SkillDirectoryError> {
        let VersionedResourceLocator::FilesystemPathV1(path) = &artifact.locator;
        if path.exists() {
            if fingerprint(path)? != artifact.expected_fingerprint {
                return Err(SkillDirectoryError::ArtifactFingerprintMismatch {
                    artifact: artifact.identity.clone(),
                });
            }
            fs::remove_dir_all(path).map_err(|source| SkillDirectoryError::Io {
                path: path.clone(),
                source,
            })?;
        }
        Ok(CleanupReceipt {
            artifact: artifact.identity.clone(),
            proof: AdapterReceipt {
                version: 1,
                payload: json!({ "state": "absent" }),
            },
        })
    }
}

impl ResourceAdapter for SkillDirectoryResourceAdapter {
    fn prepare_operation(
        &self,
        resource: &EffectResource,
        attempt: ReconcileAttemptId,
        generation: ora_effect::Generation,
        sequence: u32,
        mutation: PlannedMutation,
        prepared_at: LocalTimestamp,
    ) -> Result<PreparedOperation, ResourceAdapterError> {
        SkillDirectoryResourceAdapter::prepare_operation(
            self,
            resource,
            attempt,
            generation,
            sequence,
            mutation,
            prepared_at,
        )
        .map_err(ResourceAdapterError::new)
    }

    fn observe(
        &self,
        resource: &EffectResource,
    ) -> Result<ResourceObservation, ResourceAdapterError> {
        (*self)
            .observe_resource(resource)
            .map_err(ResourceAdapterError::new)
    }

    fn apply(&self, operation: &EffectOperation) -> Result<ApplyReceipt, ResourceAdapterError> {
        (*self)
            .apply_operation(operation)
            .map_err(ResourceAdapterError::new)
    }

    fn verify(
        &self,
        operation: &EffectOperation,
    ) -> Result<VerificationReceipt, ResourceAdapterError> {
        (*self)
            .verify_operation(operation)
            .map_err(ResourceAdapterError::new)
    }

    fn cleanup(
        &self,
        artifact: &OperationArtifact,
    ) -> Result<CleanupReceipt, ResourceAdapterError> {
        (*self)
            .cleanup_artifact(artifact)
            .map_err(ResourceAdapterError::new)
    }
}

/// Reports filesystem validation, observation, mutation, and recovery failures.
#[derive(Debug, Error)]
pub enum SkillDirectoryError {
    #[error(transparent)]
    Operation(#[from] ora_effect::OperationTransitionError),
    #[error("Skill adapter received a non-directory Resource descriptor")]
    UnsupportedResourceDescriptor,
    #[error("Skill adapter received a non-directory operation plan")]
    UnsupportedAdapterPlan,
    #[error("Workspace root is unavailable: {path:?}")]
    WorkspaceUnavailable {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("unsafe Effect Resource path: {path:?}")]
    UnsafeResourcePath { path: PathBuf },
    #[error("unsupported entry in Skill directory Resource: {path:?}")]
    UnsupportedResourceEntry { path: PathBuf },
    #[error("invalid native Resource identity: {0}")]
    InvalidNativeIdentity(String),
    #[error("invalid Skill manifest")]
    InvalidSkillManifest,
    #[error("Skill manifest name does not match its native identity")]
    ManifestNameMismatch,
    #[error("source content changed while staging")]
    SourceChanged,
    #[error("materialization operation is missing source input")]
    MissingMaterializationInput,
    #[error("operation expected state does not name an item")]
    MissingExpectedItem,
    #[error("operation planned state does not name an item")]
    MissingPlannedItem,
    #[error("operation artifact paths are outside the Resource")]
    UnsafeOperationPath,
    #[error("operation staging state does not match durable intent: {path:?}")]
    StagingMismatch { path: PathBuf },
    #[error("target is occupied: {path:?}")]
    TargetOccupied { path: PathBuf },
    #[error("operation {operation} requires manual recovery")]
    RecoveryRequired { operation: EffectOperationId },
    #[error("operation {operation} did not reach exact planned state")]
    VerificationFailed { operation: EffectOperationId },
    #[error("artifact {artifact} no longer matches its cleanup authority")]
    ArtifactFingerprintMismatch { artifact: ArtifactId },
    #[error("Effect Resource filesystem operation failed: {path:?}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(transparent)]
    DirectoryTree(#[from] DirectoryTreeError),
    #[error("ownership marker serialization failed")]
    MarkerJson(#[source] serde_json::Error),
}
