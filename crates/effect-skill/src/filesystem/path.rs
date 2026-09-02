use super::SkillDirectoryError;
use ora_effect::{EffectResource, FilesystemOperationPlan, VersionedResourceDescriptor};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Selects whether resolving a Resource path may create missing safe directories.
#[derive(Clone, Copy)]
pub(super) enum RootAccess {
    Observe,
    Prepare,
    Mutate,
}

/// Resolves the typed filesystem descriptor and checks the root stays inside its Workspace.
pub(super) fn resolve_resource_root(
    resource: &EffectResource,
    access: RootAccess,
) -> Result<Option<PathBuf>, SkillDirectoryError> {
    let VersionedResourceDescriptor::FilesystemDirectoryV1(descriptor) = &resource.descriptor
    else {
        return Err(SkillDirectoryError::UnsupportedResourceDescriptor);
    };
    resolve_declared_root(
        &descriptor.workspace_root,
        &descriptor.relative_path,
        access,
    )
}

/// Resolves a filesystem descriptor while refusing links and optionally creating safe segments.
pub(super) fn resolve_declared_root(
    workspace_root: &Path,
    relative_path: &ora_effect::ResourcePath,
    access: RootAccess,
) -> Result<Option<PathBuf>, SkillDirectoryError> {
    let root_metadata = fs::symlink_metadata(workspace_root).map_err(|source| {
        SkillDirectoryError::WorkspaceUnavailable {
            path: workspace_root.to_path_buf(),
            source,
        }
    })?;
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        return Err(SkillDirectoryError::UnsafeResourcePath {
            path: workspace_root.to_path_buf(),
        });
    }
    let canonical_workspace = workspace_root.canonicalize().map_err(|source| {
        SkillDirectoryError::WorkspaceUnavailable {
            path: workspace_root.to_path_buf(),
            source,
        }
    })?;
    let mut current = canonical_workspace.clone();
    let mut path_is_missing = false;
    for component in relative_path.to_path_buf().components() {
        let std::path::Component::Normal(segment) = component else {
            return Err(SkillDirectoryError::UnsafeResourcePath { path: current });
        };
        current = current.join(segment);
        if path_is_missing {
            continue;
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(SkillDirectoryError::UnsafeResourcePath { path: current });
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => match access {
                RootAccess::Observe => return Ok(None),
                RootAccess::Prepare => path_is_missing = true,
                RootAccess::Mutate => {
                    fs::create_dir(&current).map_err(|source| SkillDirectoryError::Io {
                        path: current.clone(),
                        source,
                    })?;
                }
            },
            Err(source) => {
                return Err(SkillDirectoryError::Io {
                    path: current,
                    source,
                });
            }
        }
        if path_is_missing {
            continue;
        }
        let canonical = current
            .canonicalize()
            .map_err(|source| SkillDirectoryError::Io {
                path: current.clone(),
                source,
            })?;
        if !canonical.starts_with(&canonical_workspace) {
            return Err(SkillDirectoryError::UnsafeResourcePath { path: current });
        }
        current = canonical;
    }
    Ok(Some(current))
}

/// Resolves the Resource root for intent preparation without creating it yet.
pub(super) fn resource_root(resource: &EffectResource) -> Result<PathBuf, SkillDirectoryError> {
    let VersionedResourceDescriptor::FilesystemDirectoryV1(descriptor) = &resource.descriptor
    else {
        return Err(SkillDirectoryError::UnsupportedResourceDescriptor);
    };
    resolve_declared_root(
        &descriptor.workspace_root,
        &descriptor.relative_path,
        RootAccess::Prepare,
    )?
    .ok_or(SkillDirectoryError::UnsafeOperationPath)
}

/// Prevents a journal payload from redirecting artifacts outside its Resource directory.
pub(super) fn ensure_operation_paths_are_scoped(
    plan: &FilesystemOperationPlan,
) -> Result<(), SkillDirectoryError> {
    let Some(staging_parent) = plan.staging_path.parent() else {
        return Err(SkillDirectoryError::UnsafeOperationPath);
    };
    if !staging_parent.starts_with(&plan.resource_root)
        || !plan.backup_path.starts_with(staging_parent)
    {
        return Err(SkillDirectoryError::UnsafeOperationPath);
    }
    Ok(())
}
