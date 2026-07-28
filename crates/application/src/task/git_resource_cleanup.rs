use gitlancer::git::branch::{BranchDeletionMode, DeleteBranchRequest};
use gitlancer::git::worktree::{
    DeleteWorktreeRequest, ResolveWorktreeByBranchRequest, ResolveWorktreeByRootRequest,
    WorktreeDeletionMode,
};
use gitlancer::{
    BranchName, DomainError, Git, GitRunner, GitlancerError, RepoRoot, Repository, WorktreeKind,
};
use std::path::PathBuf;

/// Supplies best-effort removal of the Git resources owned by a deleted task.
///
/// Implementations perform individual Git attempts and report each outcome without
/// deciding whether aggregate deletion itself succeeded.
pub trait TaskGitResourceCleaner {
    /// Attempts independent worktree and branch cleanup for one deleted task.
    fn cleanup_task_git_resources(
        &self,
        request: TaskGitResourceCleanupRequest,
    ) -> TaskGitResourceCleanupReport;
}

/// Carries the validated repository, branch, and deterministic checkout root for cleanup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskGitResourceCleanupRequest {
    pub repository_root: PathBuf,
    pub expected_worktree_root: PathBuf,
    pub branch_name: String,
}

/// Reports the independent outcomes of one task's worktree and branch cleanup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskGitResourceCleanupReport {
    pub worktree: GitResourceCleanupOutcome,
    pub branch: GitResourceCleanupOutcome,
}

/// Models one best-effort Git resource attempt without conflating absence and failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitResourceCleanupOutcome {
    Removed,
    AlreadyAbsent,
    Failed { message: String },
}

/// Removes task-owned Git resources through an injected Git runtime.
#[derive(Debug)]
pub struct GitTaskGitResourceCleaner<Runner: GitRunner> {
    git: Git<Runner>,
}

impl<Runner: GitRunner> GitTaskGitResourceCleaner<Runner> {
    /// Builds a cleaner whose runner can be replaced by deterministic tests.
    pub fn new(runner: Runner) -> Self {
        Self {
            git: Git::new(runner),
        }
    }
}

impl<Runner: GitRunner> TaskGitResourceCleaner for GitTaskGitResourceCleaner<Runner> {
    /// Uses branch metadata first and the exact Ora checkout root only as a safe fallback.
    fn cleanup_task_git_resources(
        &self,
        request: TaskGitResourceCleanupRequest,
    ) -> TaskGitResourceCleanupReport {
        let repository = Repository::new(RepoRoot::new(&request.repository_root));
        let worktree = self
            .git
            .resolve_worktree_by_branch(ResolveWorktreeByBranchRequest {
                repository: &repository,
                branch_name: &request.branch_name,
            })
            .or_else(|error| match error {
                GitlancerError::Domain(DomainError::NotAWorktree(_)) => self
                    .git
                    .resolve_worktree_by_root(ResolveWorktreeByRootRequest {
                        repository: &repository,
                        worktree_root: &request.expected_worktree_root,
                    }),
                other => Err(other),
            });

        let worktree = match worktree {
            Ok(worktree) if matches!(worktree.kind(), WorktreeKind::Linked { .. }) => self
                .git
                .delete_worktree(DeleteWorktreeRequest {
                    repository: &repository,
                    worktree: &worktree,
                    mode: WorktreeDeletionMode::Force,
                })
                .map(|_| GitResourceCleanupOutcome::Removed)
                .unwrap_or_else(cleanup_failure),
            Ok(_) => GitResourceCleanupOutcome::Failed {
                message: "refusing to delete the main worktree".to_string(),
            },
            Err(GitlancerError::Domain(DomainError::NotAWorktree(_))) => {
                GitResourceCleanupOutcome::AlreadyAbsent
            }
            Err(error) => cleanup_failure(error),
        };

        // Branch cleanup remains independent because a failed or locked worktree should
        // not prevent removal of an otherwise unreferenced Ora-owned branch.
        let branch = self
            .git
            .delete_branch(DeleteBranchRequest {
                repository: &repository,
                branch_name: BranchName::new(request.branch_name),
                mode: BranchDeletionMode::Force,
            })
            .map(|_| GitResourceCleanupOutcome::Removed)
            .unwrap_or_else(|error| match error {
                GitlancerError::Domain(DomainError::BranchNotFound { .. }) => {
                    GitResourceCleanupOutcome::AlreadyAbsent
                }
                other => cleanup_failure(other),
            });

        TaskGitResourceCleanupReport { worktree, branch }
    }
}

/// Preserves Git diagnostics in a structured best-effort outcome for backend logging.
fn cleanup_failure(error: GitlancerError) -> GitResourceCleanupOutcome {
    GitResourceCleanupOutcome::Failed {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GitResourceCleanupOutcome, GitTaskGitResourceCleaner, TaskGitResourceCleaner,
        TaskGitResourceCleanupReport, TaskGitResourceCleanupRequest,
    };
    use gitlancer::{GitCommand, GitExecError, GitOutput, GitRunner};
    use pretty_assertions::assert_eq;
    use std::cell::RefCell;
    use std::path::PathBuf;

    /// Records cleanup commands against one detached linked worktree.
    #[derive(Debug)]
    struct DetachedWorktreeRunner {
        linked_worktree_root: PathBuf,
        commands: RefCell<Vec<GitCommand>>,
    }

    impl DetachedWorktreeRunner {
        /// Builds a runner whose worktree listing never exposes a branch association.
        fn new(linked_worktree_root: PathBuf) -> Self {
            Self {
                linked_worktree_root,
                commands: RefCell::new(Vec::new()),
            }
        }

        /// Returns every Git command attempted by the cleaner.
        fn commands(&self) -> Vec<GitCommand> {
            self.commands.borrow().clone()
        }
    }

    impl GitRunner for DetachedWorktreeRunner {
        /// Supplies detached porcelain metadata while allowing destructive commands to succeed.
        fn run(&self, command: &GitCommand) -> Result<GitOutput, GitExecError> {
            self.commands.borrow_mut().push(command.clone());
            match command.args.as_slice() {
                [git_area, subcommand, ..] if git_area == "worktree" && subcommand == "list" => {
                    Ok(GitOutput::new(
                        Some(0),
                        format!(
                            "worktree /repo\nHEAD 1111111\nbranch refs/heads/main\n\nworktree {}\nHEAD 2222222\ndetached\n",
                            self.linked_worktree_root.to_string_lossy()
                        ),
                        String::new(),
                        0,
                    ))
                }
                [git_area, ..] if git_area == "for-each-ref" => Ok(GitOutput::new(
                    Some(0),
                    "main\nora/12345678\n".to_string(),
                    String::new(),
                    0,
                )),
                [git_area, subcommand, ..]
                    if (git_area == "worktree" && subcommand == "remove")
                        || git_area == "branch" =>
                {
                    Ok(GitOutput::new(Some(0), String::new(), String::new(), 0))
                }
                _ => panic!("unexpected Git command: {:?}", command.args),
            }
        }
    }

    /// Verifies exact-root fallback removes a detached checkout before its expected branch.
    #[test]
    fn removes_a_detached_worktree_at_the_exact_expected_root() {
        let cleaner =
            GitTaskGitResourceCleaner::new(DetachedWorktreeRunner::new(PathBuf::from("/expected")));

        let report = cleaner.cleanup_task_git_resources(cleanup_request("/expected"));

        assert_eq!(
            report,
            TaskGitResourceCleanupReport {
                worktree: GitResourceCleanupOutcome::Removed,
                branch: GitResourceCleanupOutcome::Removed,
            }
        );
        assert_eq!(
            cleaner
                .git
                .runner()
                .commands()
                .into_iter()
                .map(|command| command.args)
                .collect::<Vec<_>>(),
            vec![
                vec!["worktree", "list", "--porcelain"],
                vec!["worktree", "list", "--porcelain"],
                vec!["worktree", "remove", "/expected", "--force"],
                vec!["for-each-ref", "--format=%(refname:short)", "refs/heads"],
                vec!["branch", "-D", "ora/12345678"],
            ]
        );
    }

    /// Verifies a different linked root is never treated as the deleted task's checkout.
    #[test]
    fn refuses_a_detached_worktree_outside_the_exact_expected_root() {
        let cleaner =
            GitTaskGitResourceCleaner::new(DetachedWorktreeRunner::new(PathBuf::from("/other")));

        let report = cleaner.cleanup_task_git_resources(cleanup_request("/expected"));

        assert_eq!(
            report,
            TaskGitResourceCleanupReport {
                worktree: GitResourceCleanupOutcome::AlreadyAbsent,
                branch: GitResourceCleanupOutcome::Removed,
            }
        );
        assert_eq!(
            cleaner
                .git
                .runner()
                .commands()
                .into_iter()
                .map(|command| command.args)
                .collect::<Vec<_>>(),
            vec![
                vec!["worktree", "list", "--porcelain"],
                vec!["worktree", "list", "--porcelain"],
                vec!["for-each-ref", "--format=%(refname:short)", "refs/heads"],
                vec!["branch", "-D", "ora/12345678"],
            ]
        );
    }

    /// Builds one cleanup request with a caller-selected deterministic root.
    fn cleanup_request(expected_worktree_root: &str) -> TaskGitResourceCleanupRequest {
        TaskGitResourceCleanupRequest {
            repository_root: PathBuf::from("/repo"),
            expected_worktree_root: PathBuf::from(expected_worktree_root),
            branch_name: "ora/12345678".to_string(),
        }
    }
}
