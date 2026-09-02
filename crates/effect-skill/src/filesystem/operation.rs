use super::{MARKER_FILE_NAME, MARKER_SCHEMA_VERSION, ManagedItemMarker, SkillDirectoryError};
use ora_effect::{
    AdapterReceipt, ApplyReceipt, EffectOperation, EffectResourceId, ExactPlannedState,
    ExactPreviousState, FilesystemOperationPlan, Fingerprint, ManagedIdentity,
    VersionedAdapterPlan,
};
use ora_skill_package::{Limits, parse_manifest};
use ora_utils::directory::{copy_directory, fingerprint_directory};
use serde_json::json;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

/// Stages and validates immutable source content before an atomic swap.
pub(super) fn stage(
    operation: &EffectOperation,
    plan: &FilesystemOperationPlan,
) -> Result<(), SkillDirectoryError> {
    let source_root = plan
        .source_root
        .as_ref()
        .ok_or(SkillDirectoryError::MissingMaterializationInput)?;
    if plan.staging_path.exists() {
        if !state_matches_path(
            operation.planned(),
            &plan.staging_path,
            operation.resource(),
        )? {
            return Err(SkillDirectoryError::StagingMismatch {
                path: plan.staging_path.clone(),
            });
        }
        return Ok(());
    }
    let operation_root = plan
        .staging_path
        .parent()
        .ok_or(SkillDirectoryError::UnsafeOperationPath)?;
    fs::create_dir_all(operation_root).map_err(|source| SkillDirectoryError::Io {
        path: operation_root.to_path_buf(),
        source,
    })?;
    copy_directory(
        source_root,
        &plan.staging_path,
        &[OsStr::new(MARKER_FILE_NAME)],
    )?;
    validate_staged_skill(operation, &plan.staging_path)?;
    let managed_identity = match operation.planned() {
        ExactPlannedState::Present {
            managed_identity, ..
        } => managed_identity.clone(),
        ExactPlannedState::Missing => return Err(SkillDirectoryError::MissingPlannedItem),
    };
    let marker = ManagedItemMarker::current(operation.resource().clone(), managed_identity);
    let marker_bytes = serde_json::to_vec(&marker).map_err(SkillDirectoryError::MarkerJson)?;
    let marker_path = plan.staging_path.join(MARKER_FILE_NAME);
    fs::write(&marker_path, marker_bytes).map_err(|source| SkillDirectoryError::Io {
        path: marker_path,
        source,
    })?;
    if !state_matches_path(
        operation.planned(),
        &plan.staging_path,
        operation.resource(),
    )? {
        return Err(SkillDirectoryError::StagingMismatch {
            path: plan.staging_path.clone(),
        });
    }
    Ok(())
}

/// Revalidates the staged Skill manifest and exact package fingerprint after copying.
fn validate_staged_skill(
    operation: &EffectOperation,
    staging: &Path,
) -> Result<(), SkillDirectoryError> {
    let VersionedAdapterPlan::FilesystemDirectoryV1(plan) = operation.payload() else {
        return Err(SkillDirectoryError::UnsupportedAdapterPlan);
    };
    let source_root = plan
        .source_root
        .as_ref()
        .ok_or(SkillDirectoryError::MissingMaterializationInput)?;
    let source_manifest =
        fs::read(source_root.join("SKILL.md")).map_err(|source| SkillDirectoryError::Io {
            path: source_root.join("SKILL.md"),
            source,
        })?;
    let staged_manifest =
        fs::read(staging.join("SKILL.md")).map_err(|source| SkillDirectoryError::Io {
            path: staging.join("SKILL.md"),
            source,
        })?;
    let parsed = parse_manifest(&staged_manifest, Limits::default().max_manifest_bytes)
        .map_err(|_| SkillDirectoryError::InvalidSkillManifest)?;
    if source_manifest != staged_manifest {
        return Err(SkillDirectoryError::SourceChanged);
    }
    let planned_name = match operation.planned() {
        ExactPlannedState::Present {
            native_identity, ..
        } => native_identity,
        ExactPlannedState::Missing => return Err(SkillDirectoryError::MissingPlannedItem),
    };
    if !parsed.name.eq_ignore_ascii_case(planned_name.as_str()) {
        return Err(SkillDirectoryError::ManifestNameMismatch);
    }
    Ok(())
}

/// Tests the exact expected state at the locator implied by both operation states.
pub(super) fn state_matches_expected(
    operation: &EffectOperation,
    plan: &FilesystemOperationPlan,
) -> Result<bool, SkillDirectoryError> {
    match operation.expected() {
        ExactPreviousState::Missing => {
            let path = planned_path(operation, plan)?;
            Ok(!path.exists())
        }
        ExactPreviousState::Present { .. } => state_matches_path(
            operation.expected(),
            &expected_path(operation, plan)?,
            operation.resource(),
        ),
    }
}

/// Tests the exact planned state at the locator implied by both operation states.
pub(super) fn state_matches_planned(
    operation: &EffectOperation,
    plan: &FilesystemOperationPlan,
) -> Result<bool, SkillDirectoryError> {
    match operation.planned() {
        ExactPlannedState::Missing => {
            let path = expected_path(operation, plan)?;
            Ok(!path.exists())
        }
        ExactPlannedState::Present { .. } => state_matches_path(
            operation.planned(),
            &planned_path(operation, plan)?,
            operation.resource(),
        ),
    }
}

/// Compares fingerprint and marker proof for either exact present-state enum.
fn state_matches_path(
    state: &impl PresentState,
    path: &Path,
    resource: &EffectResourceId,
) -> Result<bool, SkillDirectoryError> {
    let Some((fingerprint_expected, managed_identity)) = state.present() else {
        return Ok(!path.exists());
    };
    if !path.exists() || fingerprint(path)? != *fingerprint_expected {
        return Ok(false);
    }
    Ok(read_marker(path).is_some_and(|marker| {
        marker.schema_version == MARKER_SCHEMA_VERSION
            && marker.resource == *resource
            && marker.managed_identity == *managed_identity
    }))
}

/// Supplies shared present-state access without collapsing the distinct expected/planned types.
trait PresentState {
    fn present(&self) -> Option<(&Fingerprint, &ManagedIdentity)>;
}

impl PresentState for ExactPreviousState {
    fn present(&self) -> Option<(&Fingerprint, &ManagedIdentity)> {
        match self {
            Self::Missing => None,
            Self::Present {
                fingerprint,
                managed_identity,
                ..
            } => Some((fingerprint, managed_identity)),
        }
    }
}

impl PresentState for ExactPlannedState {
    fn present(&self) -> Option<(&Fingerprint, &ManagedIdentity)> {
        match self {
            Self::Missing => None,
            Self::Present {
                fingerprint,
                managed_identity,
                ..
            } => Some((fingerprint, managed_identity)),
        }
    }
}

/// Resolves the expected native item path using a typed identity and Path::join.
pub(super) fn expected_path(
    operation: &EffectOperation,
    plan: &FilesystemOperationPlan,
) -> Result<PathBuf, SkillDirectoryError> {
    match operation.expected() {
        ExactPreviousState::Present {
            native_identity, ..
        } => Ok(plan.resource_root.join(native_identity.as_str())),
        ExactPreviousState::Missing => Err(SkillDirectoryError::MissingExpectedItem),
    }
}

/// Resolves the planned native item path using a typed identity and Path::join.
pub(super) fn planned_path(
    operation: &EffectOperation,
    plan: &FilesystemOperationPlan,
) -> Result<PathBuf, SkillDirectoryError> {
    match operation.planned() {
        ExactPlannedState::Present {
            native_identity, ..
        } => Ok(plan.resource_root.join(native_identity.as_str())),
        ExactPlannedState::Missing => Err(SkillDirectoryError::MissingPlannedItem),
    }
}

/// Reads a marker as untrusted evidence; malformed or absent markers establish no ownership.
pub(super) fn read_marker(path: &Path) -> Option<ManagedItemMarker> {
    fs::read(path.join(MARKER_FILE_NAME))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

/// Produces a versioned idempotence receipt without exposing filesystem paths.
pub(super) fn apply_receipt(operation: &EffectOperation) -> ApplyReceipt {
    ApplyReceipt {
        operation: operation.identity().clone(),
        proof: AdapterReceipt {
            version: 1,
            payload: json!({ "state": "applied_or_already_planned" }),
        },
    }
}

/// Excludes Ora's marker while fingerprinting the user-visible package tree.
pub(super) fn fingerprint(path: &Path) -> Result<Fingerprint, SkillDirectoryError> {
    Ok(Fingerprint::from(fingerprint_directory(
        path,
        &[OsStr::new(MARKER_FILE_NAME)],
    )?))
}

/// Restores the previous tree when a swap cannot install its staging directory.
pub(super) fn restore_backup(backup: &Path, previous: &Path) {
    if backup.exists() && !previous.exists() {
        let _ = fs::rename(backup, previous);
    }
}
