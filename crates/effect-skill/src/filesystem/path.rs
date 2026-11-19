use super::SkillDirectoryError;
use ora_effect::{EffectResource, FilesystemOperationPlan, VersionedResourceDescriptor};
use ora_utils::directory::{DirectoryTreeError, is_link_or_reparse_point};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Selects whether resolving a Resource path may create missing safe directories.
#[derive(Clone, Copy, PartialEq)]
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
    let root_metadata = match fs::symlink_metadata(workspace_root) {
        Ok(metadata) => metadata,
        // A deleted Worktree leaves nothing to observe. Reporting an empty Resource lets a
        // retiring Target forget its ledger and finish retiring when no surviving contributor
        // still desires content on the shared Resource (issue #6). Mutation and preparation
        // still refuse a Workspace root the user removed, so a deleted Worktree with a
        // surviving successor keeps both Targets failing until the workspace returns or the
        // successor's intent is withdrawn — surfaced for manual recovery as issue #664.
        Err(error) if error.kind() == io::ErrorKind::NotFound && access == RootAccess::Observe => {
            return Ok(None);
        }
        Err(source) => {
            return Err(SkillDirectoryError::WorkspaceUnavailable {
                path: workspace_root.to_path_buf(),
                source,
            });
        }
    };
    if is_link_or_reparse_point(&root_metadata) {
        return Err(DirectoryTreeError::LinkRejected {
            path: workspace_root.to_path_buf(),
        }
        .into());
    }
    if !root_metadata.is_dir() {
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
            Ok(metadata) if is_link_or_reparse_point(&metadata) => {
                return Err(DirectoryTreeError::LinkRejected { path: current }.into());
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(SkillDirectoryError::UnsafeResourcePath { path: current });
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => match access {
                RootAccess::Observe => return Ok(None),
                RootAccess::Prepare => path_is_missing = true,
                RootAccess::Mutate => {
                    fs::create_dir(&current).map_err(|source| SkillDirectoryError::Io {
                        action: "create_resource_directory",
                        path: current.clone(),
                        source,
                    })?;
                }
            },
            Err(source) => {
                return Err(SkillDirectoryError::Io {
                    action: "inspect_resource_directory",
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
                action: "canonicalize_resource_directory",
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

/// Checks persisted Workspace and Resource identity before accepting recovery evidence.
///
/// Recovery uses Prepare access to validate existing ancestors and resolve missing segments
/// without creating directories. Observe loses the resolved identity on missing paths;
/// Mutate would change the filesystem before recovery establishes authority.
pub(super) fn verify_operation_root(
    plan: &FilesystemOperationPlan,
) -> Result<(), SkillDirectoryError> {
    ensure_operation_paths_are_scoped(plan)?;
    let resolved = resolve_declared_root(
        &plan.workspace_root,
        &plan.resource_relative_path,
        RootAccess::Prepare,
    )?
    .ok_or(SkillDirectoryError::UnsafeOperationPath)?;
    if resolved != plan.resource_root {
        return Err(SkillDirectoryError::UnsafeOperationPath);
    }
    Ok(())
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
