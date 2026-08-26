use crate::{
    AppliedFingerprint, DesiredSkillState, Digest, EffectOperation, ManagedIdentity,
    OperationState, SkillName, SkillSource, SkillState, SourceSnapshot, SurfaceKey, SurfacePath,
    TargetObservation,
};
use ora_domain::{WorkspaceId, validate_skill_name};
use ora_skill_package::{Limits, parse_manifest};
use ora_utils::directory::{
    DirectoryFingerprint, DirectoryTreeError, copy_directory, fingerprint_directory,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const MARKER_FILE_NAME: &str = ".ora-managed.json";
const OPERATIONS_DIR_NAME: &str = ".ora-effect-operations";
const MARKER_SCHEMA_VERSION: u32 = 1;

/// On-disk half of the dual ownership proof for one managed Skill directory.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ManagedSkillMarker {
    pub schema_version: u32,
    pub workspace_id: WorkspaceId,
    pub surface_key: SurfaceKey,
    pub managed_identity: ManagedIdentity,
}

impl ManagedSkillMarker {
    /// Creates the current marker schema for a newly staged managed directory.
    pub fn current(
        workspace_id: WorkspaceId,
        surface_key: SurfaceKey,
        managed_identity: ManagedIdentity,
    ) -> Self {
        Self {
            schema_version: MARKER_SCHEMA_VERSION,
            workspace_id,
            surface_key,
            managed_identity,
        }
    }
}

/// A scan issue that is not disguised as a legal preserved Skill.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanDiagnostic {
    pub entry_name: String,
    pub message: String,
    pub related_locator: Option<String>,
}

/// Complete live observation used for one planner pass.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SurfaceScan {
    pub targets: BTreeMap<String, TargetObservation>,
    pub preserved: Vec<SkillState>,
    pub diagnostics: Vec<ScanDiagnostic>,
}

/// Exact, journal-recorded staging and backup paths for one operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationPaths {
    pub root: PathBuf,
    pub staging: PathBuf,
    pub backup: PathBuf,
}

impl OperationPaths {
    /// Derives controlled paths under this surface from an opaque operation identity.
    pub fn for_operation(surface_root: &Path, operation_id: &crate::EffectOperationId) -> Self {
        let root = surface_root
            .join(OPERATIONS_DIR_NAME)
            .join(operation_id.as_str());
        Self {
            staging: root.join("staging"),
            backup: root.join("backup"),
            root,
        }
    }
}

/// Deterministic action selected by comparing current disk state to a durable operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryDecision {
    RetryApply,
    Finalize,
    RecoveryRequired,
}

/// Filesystem adapter for one local Workspace and consumer-declared surface.
#[derive(Clone, Debug)]
pub struct FilesystemSurfaceAdapter {
    workspace_id: WorkspaceId,
    workspace_root: PathBuf,
    surface_key: SurfaceKey,
    surface_path: SurfacePath,
}

impl FilesystemSurfaceAdapter {
    /// Captures a descriptor without creating a missing Workspace root.
    pub fn new(
        workspace_id: WorkspaceId,
        workspace_root: PathBuf,
        surface_key: SurfaceKey,
        surface_path: SurfacePath,
    ) -> Self {
        Self {
            workspace_id,
            workspace_root,
            surface_key,
            surface_path,
        }
    }

    /// Returns the descriptor-resolved surface root without string path concatenation.
    pub fn surface_root(&self) -> PathBuf {
        self.workspace_root.join(self.surface_path.to_path_buf())
    }

    /// Creates a missing surface path only after proving every existing ancestor is an ordinary
    /// directory inside the existing Workspace.
    pub fn ensure_surface_root(&self) -> Result<PathBuf, FilesystemEffectError> {
        let root_metadata = fs::symlink_metadata(&self.workspace_root).map_err(|source| {
            FilesystemEffectError::WorkspaceUnavailable {
                path: self.workspace_root.clone(),
                source,
            }
        })?;
        if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
            return Err(FilesystemEffectError::UnsafeSurfacePath {
                path: self.workspace_root.clone(),
            });
        }
        let canonical_workspace = self.workspace_root.canonicalize().map_err(|source| {
            FilesystemEffectError::WorkspaceUnavailable {
                path: self.workspace_root.clone(),
                source,
            }
        })?;
        let mut current = canonical_workspace.clone();
        for component in self.surface_path.to_path_buf().components() {
            let std::path::Component::Normal(segment) = component else {
                return Err(FilesystemEffectError::UnsafeSurfacePath { path: current });
            };
            current = current.join(segment);
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                    return Err(FilesystemEffectError::UnsafeSurfacePath { path: current });
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    fs::create_dir(&current).map_err(|source| FilesystemEffectError::Io {
                        path: current.clone(),
                        source,
                    })?;
                }
                Err(source) => {
                    return Err(FilesystemEffectError::Io {
                        path: current,
                        source,
                    });
                }
            }
            let canonical = current
                .canonicalize()
                .map_err(|source| FilesystemEffectError::Io {
                    path: current.clone(),
                    source,
                })?;
            if !canonical.starts_with(&canonical_workspace) {
                return Err(FilesystemEffectError::UnsafeSurfacePath { path: current });
            }
            current = canonical;
        }
        Ok(current)
    }

    /// Scans all legal Skill directories and keeps unrelated diagnostics non-blocking.
    pub fn scan(&self) -> Result<SurfaceScan, FilesystemEffectError> {
        let surface_root = self.ensure_surface_root()?;
        let mut scan = SurfaceScan::default();
        let entries = fs::read_dir(&surface_root).map_err(|source| FilesystemEffectError::Io {
            path: surface_root.clone(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| FilesystemEffectError::Io {
                path: surface_root.clone(),
                source,
            })?;
            let entry_name = entry.file_name().to_string_lossy().into_owned();
            if entry_name == OPERATIONS_DIR_NAME {
                continue;
            }
            let path = entry.path();
            let metadata =
                fs::symlink_metadata(&path).map_err(|source| FilesystemEffectError::Io {
                    path: path.clone(),
                    source,
                })?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                scan.diagnostics.push(ScanDiagnostic {
                    entry_name,
                    message: "surface entry is not an ordinary Skill directory".to_string(),
                    related_locator: None,
                });
                continue;
            }
            let skill_name = match SkillName::parse(entry_name.clone()) {
                Ok(name) => name,
                Err(_) => {
                    scan.diagnostics.push(ScanDiagnostic {
                        entry_name,
                        message: "surface directory name is not a valid Skill name".to_string(),
                        related_locator: None,
                    });
                    continue;
                }
            };
            let locator = skill_name.canonical().to_string();
            match self.scan_skill_directory(&path, skill_name) {
                Ok((_state, Some(marker), fingerprint))
                    if marker.schema_version == MARKER_SCHEMA_VERSION
                        && marker.workspace_id == self.workspace_id
                        && marker.surface_key == self.surface_key =>
                {
                    scan.targets.insert(
                        locator,
                        TargetObservation::Managed {
                            marker_identity: marker.managed_identity,
                            fingerprint,
                        },
                    );
                }
                Ok((state, _, _)) => {
                    scan.targets
                        .insert(locator.clone(), TargetObservation::Preserved);
                    scan.preserved.push(SkillState {
                        source: SkillSource::Preserved {
                            workspace_id: self.workspace_id.clone(),
                        },
                        ..state
                    });
                }
                Err(error) => {
                    scan.targets.insert(
                        locator.clone(),
                        TargetObservation::Invalid {
                            message: error.safe_message(),
                        },
                    );
                    scan.diagnostics.push(ScanDiagnostic {
                        entry_name,
                        message: error.safe_message(),
                        related_locator: Some(locator),
                    });
                }
            }
        }
        Ok(scan)
    }

    /// Stages a complete validated source package and writes ownership metadata separately.
    pub fn stage(
        &self,
        snapshot: &SourceSnapshot,
        managed_identity: &ManagedIdentity,
        paths: &OperationPaths,
    ) -> Result<AppliedFingerprint, FilesystemEffectError> {
        fs::create_dir_all(&paths.root).map_err(|source| FilesystemEffectError::Io {
            path: paths.root.clone(),
            source,
        })?;
        if paths.staging.exists() {
            validate_staged_state(&paths.staging, &snapshot.state)?;
            return fingerprint(&paths.staging);
        }
        copy_directory(
            &snapshot.package_root,
            &paths.staging,
            &[OsStr::new(MARKER_FILE_NAME)],
        )?;
        validate_staged_state(&paths.staging, &snapshot.state)?;
        let marker = ManagedSkillMarker::current(
            self.workspace_id.clone(),
            self.surface_key.clone(),
            managed_identity.clone(),
        );
        let marker_bytes =
            serde_json::to_vec(&marker).map_err(FilesystemEffectError::MarkerJson)?;
        fs::write(paths.staging.join(MARKER_FILE_NAME), marker_bytes).map_err(|source| {
            FilesystemEffectError::Io {
                path: paths.staging.join(MARKER_FILE_NAME),
                source,
            }
        })?;
        fingerprint(&paths.staging)
    }

    /// Computes the target fingerprint before reserving or creating recovery artifacts.
    pub fn planned_fingerprint(
        &self,
        snapshot: &SourceSnapshot,
    ) -> Result<AppliedFingerprint, FilesystemEffectError> {
        validate_staged_state(&snapshot.package_root, &snapshot.state)?;
        fingerprint(&snapshot.package_root)
    }

    /// Applies a staged create only when the target locator is still absent.
    pub fn apply_create(
        &self,
        target_name: &SkillName,
        paths: &OperationPaths,
    ) -> Result<(), FilesystemEffectError> {
        let target = self.surface_root().join(target_name.as_str());
        if target.exists() {
            return Err(FilesystemEffectError::TargetOccupied { path: target });
        }
        fs::rename(&paths.staging, &target).map_err(|source| FilesystemEffectError::Io {
            path: target,
            source,
        })
    }

    /// Atomically swaps a staged directory while retaining the previous tree for recovery.
    pub fn apply_swap(
        &self,
        previous_name: &SkillName,
        target_name: &SkillName,
        paths: &OperationPaths,
    ) -> Result<(), FilesystemEffectError> {
        let previous = self.surface_root().join(previous_name.as_str());
        let target = self.surface_root().join(target_name.as_str());
        if previous.exists() {
            fs::rename(&previous, &paths.backup).map_err(|source| FilesystemEffectError::Io {
                path: previous.clone(),
                source,
            })?;
        }
        if target != previous && target.exists() {
            restore_backup_after_failed_swap(&paths.backup, &previous);
            return Err(FilesystemEffectError::TargetOccupied { path: target });
        }
        if let Err(source) = fs::rename(&paths.staging, &target) {
            restore_backup_after_failed_swap(&paths.backup, &previous);
            return Err(FilesystemEffectError::Io {
                path: target,
                source,
            });
        }
        Ok(())
    }

    /// Moves an owned target to its exact journaled backup instead of deleting in place.
    pub fn apply_delete(
        &self,
        target_name: &SkillName,
        paths: &OperationPaths,
    ) -> Result<(), FilesystemEffectError> {
        let target = self.surface_root().join(target_name.as_str());
        if !target.exists() {
            return Ok(());
        }
        fs::create_dir_all(&paths.root).map_err(|source| FilesystemEffectError::Io {
            path: paths.root.clone(),
            source,
        })?;
        fs::rename(&target, &paths.backup).map_err(|source| FilesystemEffectError::Io {
            path: target,
            source,
        })
    }

    /// Removes only the operation directory named by its durable journal after finalization.
    pub fn cleanup_operation(&self, paths: &OperationPaths) -> Result<(), FilesystemEffectError> {
        if paths.root.exists() {
            fs::remove_dir_all(&paths.root).map_err(|source| FilesystemEffectError::Io {
                path: paths.root.clone(),
                source,
            })?;
        }
        Ok(())
    }

    /// Reads current target identity for recovery without trusting a watcher payload.
    pub fn operation_state(
        &self,
        target_name: &SkillName,
    ) -> Result<OperationState, FilesystemEffectError> {
        let target = self.surface_root().join(target_name.as_str());
        if !target.exists() {
            return Ok(OperationState::Missing);
        }
        Ok(OperationState::Present(fingerprint(&target)?))
    }

    /// Selects the only safe automatic recovery action for a prepared or applied operation.
    pub fn recovery_decision(
        &self,
        operation: &EffectOperation,
    ) -> Result<RecoveryDecision, FilesystemEffectError> {
        let current = self.operation_state(&operation.target_name)?;
        if current == operation.previous_state {
            Ok(RecoveryDecision::RetryApply)
        } else if current == operation.planned_state {
            Ok(RecoveryDecision::Finalize)
        } else {
            Ok(RecoveryDecision::RecoveryRequired)
        }
    }

    /// Returns a safe message while keeping absolute paths and source contents out of status.
    fn scan_skill_directory(
        &self,
        path: &Path,
        directory_name: SkillName,
    ) -> Result<(SkillState, Option<ManagedSkillMarker>, AppliedFingerprint), FilesystemEffectError>
    {
        let manifest_path = path.join("SKILL.md");
        let manifest_bytes =
            fs::read(&manifest_path).map_err(|source| FilesystemEffectError::Io {
                path: manifest_path,
                source,
            })?;
        let parsed = parse_manifest(&manifest_bytes, Limits::default().max_manifest_bytes)
            .map_err(|_| FilesystemEffectError::InvalidSkillManifest)?;
        if parsed.name != directory_name.as_str() || validate_skill_name(&parsed.name).is_err() {
            return Err(FilesystemEffectError::ManifestNameMismatch);
        }
        let marker_path = path.join(MARKER_FILE_NAME);
        let marker = match fs::read(marker_path) {
            Ok(bytes) => serde_json::from_slice(&bytes).ok(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(source) => {
                return Err(FilesystemEffectError::Io {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };
        let fingerprint = fingerprint(path)?;
        Ok((
            SkillState {
                name: directory_name,
                skill_md_digest: Digest::sha256(&manifest_bytes),
                source: SkillSource::Preserved {
                    workspace_id: self.workspace_id.clone(),
                },
            },
            marker,
            fingerprint,
        ))
    }
}

impl FilesystemEffectError {
    /// Redacts paths and adapter details before a diagnostic enters persisted status.
    fn safe_message(&self) -> String {
        match self {
            Self::WorkspaceUnavailable { .. } => "Workspace is unavailable".to_string(),
            Self::UnsafeSurfacePath { .. } => "surface path is unsafe".to_string(),
            Self::InvalidSkillManifest => "Skill manifest is invalid".to_string(),
            Self::ManifestNameMismatch => {
                "Skill manifest name does not match its directory".to_string()
            }
            Self::OperationPathOccupied { .. }
            | Self::TargetOccupied { .. }
            | Self::Io { .. }
            | Self::DirectoryTree(_)
            | Self::MarkerJson(_) => "surface filesystem operation failed".to_string(),
        }
    }
}

/// Reports safe filesystem validation, materialization, and transaction failures.
#[derive(Debug, Error)]
pub enum FilesystemEffectError {
    #[error("Workspace root is unavailable: {path:?}")]
    WorkspaceUnavailable {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("unsafe surface path: {path:?}")]
    UnsafeSurfacePath { path: PathBuf },
    #[error("invalid Skill manifest")]
    InvalidSkillManifest,
    #[error("Skill manifest name does not match its directory")]
    ManifestNameMismatch,
    #[error("operation path already exists: {path:?}")]
    OperationPathOccupied { path: PathBuf },
    #[error("target is occupied: {path:?}")]
    TargetOccupied { path: PathBuf },
    #[error("surface filesystem operation failed: {path:?}")]
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

/// Revalidates identity and digest after copying so source drift cannot publish mixed state.
fn validate_staged_state(
    staging: &Path,
    desired: &DesiredSkillState,
) -> Result<(), FilesystemEffectError> {
    let manifest_path = staging.join("SKILL.md");
    let bytes = fs::read(&manifest_path).map_err(|source| FilesystemEffectError::Io {
        path: manifest_path,
        source,
    })?;
    let parsed = parse_manifest(&bytes, Limits::default().max_manifest_bytes)
        .map_err(|_| FilesystemEffectError::InvalidSkillManifest)?;
    if parsed.name != desired.state().name.as_str() {
        return Err(FilesystemEffectError::ManifestNameMismatch);
    }
    if Digest::sha256(&bytes) != desired.state().skill_md_digest {
        return Err(FilesystemEffectError::InvalidSkillManifest);
    }
    Ok(())
}

/// Converts the generic directory fingerprint into Effect's persisted fingerprint type.
fn fingerprint(path: &Path) -> Result<AppliedFingerprint, FilesystemEffectError> {
    let fingerprint: DirectoryFingerprint =
        fingerprint_directory(path, &[OsStr::new(MARKER_FILE_NAME)])?;
    AppliedFingerprint::parse(fingerprint.as_str().to_string())
        .map_err(|_| FilesystemEffectError::InvalidSkillManifest)
}

/// Restores the previous tree when a swap cannot install its staging directory.
fn restore_backup_after_failed_swap(backup: &Path, previous: &Path) {
    if backup.exists() && !previous.exists() {
        let _ = fs::rename(backup, previous);
    }
}
