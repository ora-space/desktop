use crate::domain::paths::WorktreeRoot;
use crate::domain::refs::BranchName;
use crate::domain::repo::Repository;
use crate::domain::worktree::{WorktreeHandle, WorktreeKind};
use crate::error::{DomainError, GitlancerError};
use crate::exec::command::{GitCommand, GitIntent};
use crate::exec::env::GitEnv;
use crate::exec::runner::GitRunner;
use crate::git::Git;

/// Carries the information needed to select one worktree from a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveWorktreeRequest<'a> {
    pub repository: &'a Repository,
    pub worktree_name: &'a str,
}

/// Carries the information needed to locate which worktree contains a caller path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindWorktreeRequest<'a> {
    pub repository: &'a Repository,
    pub candidate_path: &'a std::path::Path,
}

/// Returns the complete list of worktrees associated with one repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListWorktreesResponse {
    pub worktrees: Vec<WorktreeHandle>,
}

/// Represents the internal result shape produced before it is wrapped for the public response type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListWorktreesResult {
    pub worktrees: Vec<WorktreeHandle>,
}

/// Carries the information needed to create one linked worktree from a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateWorktreeRequest<'a> {
    pub repository: &'a Repository,
    pub worktree_root: WorktreeRoot,
    pub branch_name: BranchName,
}

/// Returns the linked worktree created by the runtime API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateWorktreeResponse {
    pub worktree: WorktreeHandle,
}

/// Describes how worktree deletion should behave when Git would otherwise protect the checkout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeDeletionMode {
    Checked,
    Force,
}

/// Carries the information needed to delete one linked worktree from its owning repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteWorktreeRequest<'a> {
    pub repository: &'a Repository,
    pub worktree: &'a WorktreeHandle,
    pub mode: WorktreeDeletionMode,
}

/// Returns the linked worktree root removed by the runtime API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteWorktreeResponse {
    pub worktree_root: WorktreeRoot,
}

impl From<ListWorktreesResult> for ListWorktreesResponse {
    /// Converts the internal result shape into the stable public response type.
    fn from(value: ListWorktreesResult) -> Self {
        Self {
            worktrees: value.worktrees,
        }
    }
}

impl<R: GitRunner> Git<R> {
    /// Resolves one named worktree by scanning the repository's known worktrees and matching their stable names.
    pub fn resolve_worktree(
        &self,
        request: ResolveWorktreeRequest<'_>,
    ) -> Result<WorktreeHandle, GitlancerError> {
        let worktrees = self
            .list_worktrees(crate::git::repository::ListWorktreesRequest {
                repository: request.repository,
            })?
            .worktrees;

        worktrees
            .into_iter()
            .find(|worktree| worktree_name(worktree) == request.worktree_name)
            .ok_or_else(|| {
                GitlancerError::Domain(DomainError::NotAWorktree(
                    request.repository.root().as_path().to_path_buf(),
                ))
            })
    }

    /// Finds which worktree contains a candidate path by choosing the deepest worktree root prefix match.
    pub fn find_worktree(
        &self,
        request: FindWorktreeRequest<'_>,
    ) -> Result<WorktreeHandle, GitlancerError> {
        let candidate = normalize_candidate_path(request.candidate_path);
        let worktrees = self
            .list_worktrees(crate::git::repository::ListWorktreesRequest {
                repository: request.repository,
            })?
            .worktrees;

        worktrees
            .into_iter()
            .filter(|worktree| candidate.starts_with(worktree.worktree_root().as_path()))
            .max_by_key(|worktree| worktree.worktree_root().as_path().components().count())
            .ok_or(GitlancerError::Domain(DomainError::NotAWorktree(candidate)))
    }

    /// Creates one linked worktree and returns the resulting typed worktree handle.
    pub fn create_worktree(
        &self,
        request: CreateWorktreeRequest<'_>,
    ) -> Result<CreateWorktreeResponse, GitlancerError> {
        let command = build_create_worktree_command(&request);
        let _output = self.runner().run(&command)?;
        let worktree = self.find_worktree(FindWorktreeRequest {
            repository: request.repository,
            candidate_path: request.worktree_root.as_path(),
        })?;

        Ok(CreateWorktreeResponse { worktree })
    }

    /// Deletes one linked worktree after validating repository ownership and rejecting the main checkout explicitly.
    pub fn delete_worktree(
        &self,
        request: DeleteWorktreeRequest<'_>,
    ) -> Result<DeleteWorktreeResponse, GitlancerError> {
        if request.worktree.repo_root() != request.repository.root() {
            return Err(GitlancerError::Domain(DomainError::WorktreeMismatch {
                worktree: request.worktree.worktree_root().as_path().to_path_buf(),
                repo: request.repository.root().as_path().to_path_buf(),
            }));
        }
        if matches!(request.worktree.kind(), WorktreeKind::Main) {
            return Err(GitlancerError::Domain(
                DomainError::MainWorktreeDeletionUnsupported(
                    request.repository.root().as_path().to_path_buf(),
                ),
            ));
        }

        let command = build_delete_worktree_command(&request);
        let _output = self.runner().run(&command)?;

        Ok(DeleteWorktreeResponse {
            worktree_root: request.worktree.worktree_root().clone(),
        })
    }
}

/// Derives the stable name callers use to address one worktree.
fn worktree_name(worktree: &WorktreeHandle) -> &str {
    match worktree.kind() {
        crate::domain::worktree::WorktreeKind::Main => "main",
        crate::domain::worktree::WorktreeKind::Linked { name } => name.as_str(),
    }
}

/// Normalizes a candidate path lexically so worktree comparisons do not depend on filesystem canonicalization.
fn normalize_candidate_path(path: &std::path::Path) -> std::path::PathBuf {
    let mut normalized = std::path::PathBuf::new();

    for component in path.components() {
        match component {
            std::path::Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            std::path::Component::RootDir => normalized.push(component.as_os_str()),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                let _ = normalized.pop();
            }
            std::path::Component::Normal(part) => normalized.push(part),
        }
    }

    normalized
}

/// Builds a stable `git worktree add` command so linked-worktree creation stays explicit and testable.
pub fn build_create_worktree_command(request: &CreateWorktreeRequest<'_>) -> GitCommand {
    GitCommand::new(
        request.repository.root().as_path().to_path_buf(),
        vec![
            "worktree".to_string(),
            "add".to_string(),
            "-b".to_string(),
            request.branch_name.as_str().to_string(),
            request
                .worktree_root
                .as_path()
                .to_string_lossy()
                .into_owned(),
        ],
        GitEnv::default(),
        GitIntent::Mutating,
    )
}

/// Builds a stable `git worktree remove` command so deletion mode remains visible in one place.
pub fn build_delete_worktree_command(request: &DeleteWorktreeRequest<'_>) -> GitCommand {
    let mut args = vec![
        "worktree".to_string(),
        "remove".to_string(),
        request
            .worktree
            .worktree_root()
            .as_path()
            .to_string_lossy()
            .into_owned(),
    ];
    if matches!(request.mode, WorktreeDeletionMode::Force) {
        args.push("--force".to_string());
    }

    GitCommand::new(
        request.repository.root().as_path().to_path_buf(),
        args,
        GitEnv::default(),
        GitIntent::Mutating,
    )
}
