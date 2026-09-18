use crate::{Clock, Node, RepositoryBinding, WorktreeGit};
use ora_node_db::{Command, Target, WriteGuard};
use ora_node_protocol::{WorktreeFailure, WorktreeFailureCode, WorktreePathPolicy};
use ora_utils::path::{CanonicalPathRoot, RelativePathLimits, StrictRelativePath};
use std::path::Path;

impl<G: WorktreeGit, W: WriteGuard, C: Clock> Node<G, W, C> {
    /// Resolves explicit configuration into a frozen target without granting deletion ownership.
    pub(crate) fn resolve(&self, command: &Command) -> Result<Target, WorktreeFailure> {
        let spec = command.spec();
        let binding = self
            .repositories
            .iter()
            .find(|b| b.repository == spec.repository)
            .ok_or_else(|| {
                failure(
                    WorktreeFailureCode::RepositoryNotFound,
                    "repository is not registered",
                )
            })?;
        if binding.main_workspace != spec.main_workspace
            || spec.workspace_id == spec.main_workspace.workspace_id
        {
            return Err(failure(
                WorktreeFailureCode::InvalidMainWorkspace,
                "main workspace binding does not match a distinct task workspace",
            ));
        }
        let (main_path, authorized_root, worktree_root, path) =
            resolve_paths(binding, &spec.path_policy)?;
        let git_directory = self.git.main_git_directory(&main_path)?;
        match command {
            Command::Ensure(_) => {
                let base_commit = self.git.resolve_base(
                    &main_path,
                    spec.base_ref.as_str(),
                    spec.expected_branch.as_str(),
                )?;
                let target = Target {
                    main_path,
                    authorized_root,
                    worktree_root,
                    path,
                    git_directory,
                    branch: spec.expected_branch.clone(),
                    base_commit,
                };
                let observation = self.git.observe(&target)?;
                if observation.branch.is_some() || observation.branch_elsewhere {
                    return Err(failure(
                        WorktreeFailureCode::BranchConflict,
                        "branch is already occupied",
                    ));
                }
                if !observation.absent() {
                    return Err(failure(
                        WorktreeFailureCode::WorktreeConflict,
                        "target is already occupied",
                    ));
                }
                Ok(target)
            }
            Command::Remove(_) => {
                let resource = self
                    .database
                    .resource(&spec.worktree_id)
                    .map_err(|e| failure(WorktreeFailureCode::OperationFailed, format!("{e:?}")))?
                    .ok_or_else(|| {
                        failure(
                            WorktreeFailureCode::WorktreeConflict,
                            "no retained ownership record",
                        )
                    })?;
                let target = Target {
                    main_path,
                    authorized_root,
                    worktree_root,
                    path,
                    git_directory,
                    branch: spec.expected_branch.clone(),
                    base_commit: resource.target.base_commit.clone(),
                };
                if !ora_node_db::owns(&resource, spec, &target) {
                    return Err(failure(
                        WorktreeFailureCode::WorktreeConflict,
                        "request differs from retained ownership",
                    ));
                }
                Ok(target)
            }
        }
    }

    /// Rechecks current binding, filesystem topology and persistent ownership before every mutation.
    pub(crate) fn verify(&self, command: &Command, target: &Target) -> Result<(), WorktreeFailure> {
        let binding = self
            .repositories
            .iter()
            .find(|b| {
                b.repository == command.spec().repository
                    && b.main_workspace == command.spec().main_workspace
            })
            .ok_or_else(|| {
                failure(
                    WorktreeFailureCode::WorktreeConflict,
                    "registered binding changed",
                )
            })?;
        let (main, authorized, root, path) = resolve_paths(binding, &command.spec().path_policy)?;
        if (main, authorized, root, path)
            != (
                target.main_path.clone(),
                target.authorized_root.clone(),
                target.worktree_root.clone(),
                target.path.clone(),
            )
            || self.git.main_git_directory(&target.main_path)? != target.git_directory
        {
            return Err(failure(
                WorktreeFailureCode::WorktreeConflict,
                "saved target no longer matches configuration or repository",
            ));
        }
        let resource = self
            .database
            .resource(&command.spec().worktree_id)
            .map_err(|e| failure(WorktreeFailureCode::OperationFailed, format!("{e:?}")))?
            .ok_or_else(|| {
                failure(
                    WorktreeFailureCode::WorktreeConflict,
                    "ownership record is missing",
                )
            })?;
        if !ora_node_db::owns(&resource, command.spec(), target) {
            return Err(failure(
                WorktreeFailureCode::WorktreeConflict,
                "ownership changed",
            ));
        }
        Ok(())
    }
}

/// Validates a single portable directory name and canonical existing roots before joining the target.
fn resolve_paths(
    binding: &RepositoryBinding,
    policy: &WorktreePathPolicy,
) -> Result<
    (
        std::path::PathBuf,
        std::path::PathBuf,
        std::path::PathBuf,
        std::path::PathBuf,
    ),
    WorktreeFailure,
> {
    let WorktreePathPolicy::NodeManaged { directory_name } = policy;
    let name = StrictRelativePath::parse_with_limits(
        directory_name,
        &RelativePathLimits {
            max_depth: 0,
            ..RelativePathLimits::default()
        },
    )
    .map_err(|e| {
        failure(
            WorktreeFailureCode::PathOutsideAuthorizedRoot,
            format!("{e:?}"),
        )
    })?;
    let authorized = CanonicalPathRoot::new(&binding.authorized_root).map_err(|e| {
        failure(
            WorktreeFailureCode::PathOutsideAuthorizedRoot,
            format!("{e:?}"),
        )
    })?;
    let main = authorized
        .resolve_existing_absolute(Path::new(binding.main_workspace.path.as_str()))
        .map_err(|e| failure(WorktreeFailureCode::InvalidMainWorkspace, format!("{e:?}")))?;
    let root = authorized
        .resolve_existing_absolute(&binding.worktree_root)
        .map_err(|e| {
            failure(
                WorktreeFailureCode::PathOutsideAuthorizedRoot,
                format!("{e:?}"),
            )
        })?;
    let target = name.to_path(&root);
    if std::fs::symlink_metadata(&target).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(failure(
            WorktreeFailureCode::PathOutsideAuthorizedRoot,
            "target must not be a symbolic link",
        ));
    }
    if target.starts_with(&main)
        || main.starts_with(&target)
        || target == authorized.as_path()
        || root == main
    {
        return Err(failure(
            WorktreeFailureCode::PathOutsideAuthorizedRoot,
            "target overlaps a protected checkout or authorization root",
        ));
    }
    Ok((main, authorized.as_path().to_path_buf(), root, target))
}

/// Builds structured diagnostics without requiring consumers to parse Git or filesystem messages.
pub(crate) fn failure(code: WorktreeFailureCode, message: impl Into<String>) -> WorktreeFailure {
    WorktreeFailure {
        code,
        message: message.into(),
    }
}
