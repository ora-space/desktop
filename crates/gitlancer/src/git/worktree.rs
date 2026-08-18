use crate::domain::paths::WorktreeRoot;
use crate::domain::refs::{BranchName, CommitId};
use crate::domain::repo::Repository;
use crate::domain::worktree::{WorktreeHandle, WorktreeKind};
use crate::error::{DomainError, GitlancerError};
use crate::exec::command::{GitCommand, GitIntent};
use crate::exec::env::GitEnv;
use crate::exec::runner::GitRunner;
use crate::git::Git;
use ora_utils::path::canonicalize_longest_existing_prefix;

/// Carries the information needed to select one worktree from a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveWorktreeRequest<'a> {
    pub repository: &'a Repository,
    pub worktree_name: &'a str,
}

/// Carries the branch identity used to resolve one existing linked worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveWorktreeByBranchRequest<'a> {
    pub repository: &'a Repository,
    pub branch_name: &'a str,
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
    pub base_commit_id: CommitId,
}

/// Returns the linked worktree created by the runtime API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateWorktreeResponse {
    pub worktree: WorktreeHandle,
    pub head_commit_id: CommitId,
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

    /// Resolves an existing worktree through Git's authoritative branch-to-path metadata.
    pub fn resolve_worktree_by_branch(
        &self,
        request: ResolveWorktreeByBranchRequest<'_>,
    ) -> Result<WorktreeHandle, GitlancerError> {
        let worktrees = self
            .list_worktrees(crate::git::repository::ListWorktreesRequest {
                repository: request.repository,
            })?
            .worktrees;

        worktrees
            .into_iter()
            .find(|worktree| {
                worktree
                    .branch_name()
                    .is_some_and(|branch| branch.as_str() == request.branch_name)
            })
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
        let candidate = canonicalize_longest_existing_prefix(request.candidate_path);
        let worktrees = self
            .list_worktrees(crate::git::repository::ListWorktreesRequest {
                repository: request.repository,
            })?
            .worktrees;

        worktrees
            .into_iter()
            .filter_map(|worktree| {
                let normalized_root =
                    canonicalize_longest_existing_prefix(worktree.worktree_root().as_path());
                candidate
                    .starts_with(&normalized_root)
                    .then_some((worktree, normalized_root))
            })
            .max_by_key(|(_, normalized_root)| normalized_root.components().count())
            .map(|(worktree, _)| worktree)
            .ok_or(GitlancerError::Domain(DomainError::NotAWorktree(candidate)))
    }

    /// Creates one linked worktree and returns the resulting typed worktree handle.
    pub fn create_worktree(
        &self,
        request: CreateWorktreeRequest<'_>,
    ) -> Result<CreateWorktreeResponse, GitlancerError> {
        let command = build_create_worktree_command(&request);
        let _output = self.runner().run(&command)?;
        let worktree = match self.find_worktree(FindWorktreeRequest {
            repository: request.repository,
            candidate_path: request.worktree_root.as_path(),
        }) {
            Ok(worktree) => worktree,
            Err(error) => {
                self.cleanup_failed_worktree_creation(&request)?;
                return Err(error);
            }
        };

        Ok(CreateWorktreeResponse {
            worktree,
            head_commit_id: request.base_commit_id.clone(),
        })
    }

    /// Removes both resources created by `git worktree add` when response discovery fails.
    fn cleanup_failed_worktree_creation(
        &self,
        request: &CreateWorktreeRequest<'_>,
    ) -> Result<(), GitlancerError> {
        let worktree_cleanup = self
            .runner()
            .run(&build_failed_create_worktree_cleanup_command(request));
        let branch_cleanup = self
            .runner()
            .run(&build_failed_create_branch_cleanup_command(request));

        // Both resources are independent cleanup targets, so a failure removing one must not
        // prevent an attempt to remove the other. Preserve the first failure when both fail.
        worktree_cleanup?;
        branch_cleanup?;
        Ok(())
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

/// Builds a stable `git worktree add` command pinned to the baseline captured before mutation.
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
            request.base_commit_id.as_str().to_string(),
        ],
        GitEnv::default(),
        GitIntent::Mutating,
    )
}

/// Builds the forced worktree removal used only to compensate a partially completed creation.
fn build_failed_create_worktree_cleanup_command(request: &CreateWorktreeRequest<'_>) -> GitCommand {
    GitCommand::new(
        request.repository.root().as_path().to_path_buf(),
        vec![
            "worktree".to_string(),
            "remove".to_string(),
            request
                .worktree_root
                .as_path()
                .to_string_lossy()
                .into_owned(),
            "--force".to_string(),
        ],
        GitEnv::default(),
        GitIntent::Mutating,
    )
}

/// Deletes the new branch after its failed worktree has been removed successfully.
fn build_failed_create_branch_cleanup_command(request: &CreateWorktreeRequest<'_>) -> GitCommand {
    GitCommand::new(
        request.repository.root().as_path().to_path_buf(),
        vec![
            "branch".to_string(),
            "-D".to_string(),
            request.branch_name.as_str().to_string(),
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

#[cfg(test)]
mod tests {
    use super::{CreateWorktreeRequest, Git, Repository};
    use crate::{
        BranchName, CommitId, GitCommand, GitExecError, GitOutput, GitRunner, GitlancerError,
        RepoRoot, WorktreeRoot,
    };
    use pretty_assertions::assert_eq;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    /// Supplies a deterministic command sequence so post-mutation rollback can be tested in isolation.
    struct SequencedRunner {
        results: RefCell<VecDeque<Result<GitOutput, GitExecError>>>,
        commands: RefCell<Vec<GitCommand>>,
    }

    impl SequencedRunner {
        /// Creates a runner whose results are consumed in command order.
        fn new(results: Vec<Result<GitOutput, GitExecError>>) -> Self {
            Self {
                results: RefCell::new(results.into()),
                commands: RefCell::new(Vec::new()),
            }
        }
    }

    impl GitRunner for SequencedRunner {
        /// Records each command and returns the next configured process result.
        fn run(&self, command: &GitCommand) -> Result<GitOutput, GitExecError> {
            self.commands.borrow_mut().push(command.clone());
            match self.results.borrow_mut().pop_front() {
                Some(result) => result,
                None => panic!("missing fake result for {:?}", command.args),
            }
        }
    }

    /// Verifies Git errors prevent worktree discovery from being treated as a successful create.
    #[test]
    fn does_not_report_a_worktree_when_git_add_fails() {
        let runner = SequencedRunner::new(vec![Err(GitExecError::NonZeroExit {
            code: Some(128),
            args: vec!["worktree".to_string(), "add".to_string()],
            stdout: String::new(),
            stderr: "add failed".to_string(),
        })]);
        let git = Git::new(runner);
        let repository = Repository::new(RepoRoot::new("/repo"));

        let result = git.create_worktree(CreateWorktreeRequest {
            repository: &repository,
            worktree_root: WorktreeRoot::new("/worktrees/task"),
            branch_name: BranchName::new("ora/task"),
            base_commit_id: CommitId::new("base-commit"),
        });

        assert!(result.is_err());
        assert_eq!(
            git.runner()
                .commands
                .borrow()
                .iter()
                .map(|command| command.args.clone())
                .collect::<Vec<_>>(),
            vec![vec![
                "worktree",
                "add",
                "-b",
                "ora/task",
                "/worktrees/task",
                "base-commit"
            ]]
        );
    }

    /// Verifies metadata discovery failures remove both resources created by `git worktree add`.
    #[test]
    fn cleans_up_worktree_and_branch_when_creation_discovery_fails() {
        let runner = SequencedRunner::new(vec![
            Ok(successful_output("")),
            Err(GitExecError::NonZeroExit {
                code: Some(1),
                args: vec![
                    "worktree".to_string(),
                    "list".to_string(),
                    "--porcelain".to_string(),
                ],
                stdout: String::new(),
                stderr: "list failed".to_string(),
            }),
            Ok(successful_output("")),
            Ok(successful_output("")),
        ]);
        let git = Git::new(runner);
        let repository = Repository::new(RepoRoot::new("/repo"));
        let result = git.create_worktree(CreateWorktreeRequest {
            repository: &repository,
            worktree_root: WorktreeRoot::new("/worktrees/task"),
            branch_name: BranchName::new("ora/task"),
            base_commit_id: CommitId::new("base-commit"),
        });

        let error = match result {
            Err(GitlancerError::Exec(GitExecError::NonZeroExit {
                code, args, stderr, ..
            })) => (code, args, stderr),
            other => panic!("expected discovery failure after successful cleanup, got {other:?}"),
        };
        assert_eq!(
            error,
            (
                Some(1),
                vec![
                    "worktree".to_string(),
                    "list".to_string(),
                    "--porcelain".to_string(),
                ],
                "list failed".to_string(),
            )
        );
        assert_eq!(
            git.runner()
                .commands
                .borrow()
                .iter()
                .map(|command| command.args.clone())
                .collect::<Vec<_>>(),
            vec![
                vec![
                    "worktree",
                    "add",
                    "-b",
                    "ora/task",
                    "/worktrees/task",
                    "base-commit",
                ],
                vec!["worktree", "list", "--porcelain"],
                vec!["worktree", "remove", "/worktrees/task", "--force"],
                vec!["branch", "-D", "ora/task"],
            ]
        );
    }

    /// Verifies branch cleanup is still attempted when forced worktree removal fails.
    #[test]
    fn attempts_branch_cleanup_after_worktree_cleanup_fails() {
        let cleanup_error = GitExecError::NonZeroExit {
            code: Some(1),
            args: vec!["worktree".to_string(), "remove".to_string()],
            stdout: String::new(),
            stderr: "worktree cleanup failed".to_string(),
        };
        let runner = SequencedRunner::new(vec![
            Ok(successful_output("")),
            Err(GitExecError::NonZeroExit {
                code: Some(1),
                args: vec![
                    "worktree".to_string(),
                    "list".to_string(),
                    "--porcelain".to_string(),
                ],
                stdout: String::new(),
                stderr: "list failed".to_string(),
            }),
            Err(cleanup_error),
            Ok(successful_output("")),
        ]);
        let git = Git::new(runner);
        let repository = Repository::new(RepoRoot::new("/repo"));

        let result = git.create_worktree(CreateWorktreeRequest {
            repository: &repository,
            worktree_root: WorktreeRoot::new("/worktrees/task"),
            branch_name: BranchName::new("ora/task"),
            base_commit_id: CommitId::new("base-commit"),
        });

        assert!(result.is_err());
        assert_eq!(
            git.runner()
                .commands
                .borrow()
                .iter()
                .map(|command| command.args.clone())
                .collect::<Vec<_>>(),
            vec![
                vec![
                    "worktree",
                    "add",
                    "-b",
                    "ora/task",
                    "/worktrees/task",
                    "base-commit",
                ],
                vec!["worktree", "list", "--porcelain"],
                vec!["worktree", "remove", "/worktrees/task", "--force"],
                vec!["branch", "-D", "ora/task"],
            ]
        );
    }

    /// Creates one successful fake output without repeating process metadata in test setup.
    fn successful_output(stdout: &str) -> GitOutput {
        GitOutput::new(Some(0), stdout.to_string(), String::new(), 0)
    }
}
