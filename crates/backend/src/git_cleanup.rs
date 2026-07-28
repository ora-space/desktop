use gitlancer::git::branch::{BranchDeletionMode, DeleteBranchRequest};
use gitlancer::git::worktree::{
    DeleteWorktreeRequest, ResolveWorktreeByBranchRequest, WorktreeDeletionMode,
};
use gitlancer::{BranchName, CliGitRunner, DomainError, Git, GitRunner, GitlancerError, RepoRoot};
use ora_application::branch_name_for_task;
use ora_db::GitCleanupTarget;
use ora_logging::ora_error;

/// Identifies which aggregate deletion initiated a best-effort Git cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AggregateDeletionKind {
    Project,
    Task,
}

impl AggregateDeletionKind {
    /// Returns the stable operation field shared by cleanup log events.
    fn operation(self) -> &'static str {
        match self {
            Self::Project => "delete_project",
            Self::Task => "delete_task",
        }
    }
}

/// Best-effort removes every validated Ora-owned worktree and branch after database commit.
pub(crate) fn cleanup_git_resources(
    deletion_kind: AggregateDeletionKind,
    targets: &[GitCleanupTarget],
) {
    cleanup_git_resources_with_runner(deletion_kind, targets, &Git::new(CliGitRunner));
}

/// Runs cleanup through an injected Git runner so failure ordering remains directly testable.
fn cleanup_git_resources_with_runner<Runner: GitRunner>(
    deletion_kind: AggregateDeletionKind,
    targets: &[GitCleanupTarget],
    git: &Git<Runner>,
) {
    for target in targets {
        let expected_branch_name = branch_name_for_task(&target.task_id);
        if target.branch_name != expected_branch_name {
            log_cleanup_failure(
                deletion_kind,
                target,
                "ownership_validation",
                format!(
                    "stored branch does not match expected Ora-owned branch {expected_branch_name}"
                ),
            );
            continue;
        }

        let repository = gitlancer::Repository::new(RepoRoot::new(&target.repository_root));
        match git.resolve_worktree_by_branch(ResolveWorktreeByBranchRequest {
            repository: &repository,
            branch_name: &target.branch_name,
        }) {
            Ok(worktree) => {
                if let Err(error) = git.delete_worktree(DeleteWorktreeRequest {
                    repository: &repository,
                    worktree: &worktree,
                    mode: WorktreeDeletionMode::Force,
                }) {
                    log_cleanup_failure(deletion_kind, target, "worktree", error.to_string());
                }
            }
            Err(GitlancerError::Domain(DomainError::NotAWorktree(_))) => {}
            Err(error) => {
                log_cleanup_failure(deletion_kind, target, "worktree", error.to_string());
            }
        }

        match git.delete_branch(DeleteBranchRequest {
            repository: &repository,
            branch_name: BranchName::new(&target.branch_name),
            mode: BranchDeletionMode::Force,
        }) {
            Ok(_) | Err(GitlancerError::Domain(DomainError::BranchNotFound { .. })) => {}
            Err(error) => {
                log_cleanup_failure(deletion_kind, target, "branch", error.to_string());
            }
        }
    }
}

/// Emits one structured failure without exposing best-effort cleanup through the API response.
fn log_cleanup_failure(
    deletion_kind: AggregateDeletionKind,
    target: &GitCleanupTarget,
    stage: &'static str,
    error: String,
) {
    ora_error!(
        message = "aggregate Git cleanup failed",
        operation = deletion_kind.operation(),
        cleanup_stage = stage,
        project_id = target.project_id.to_string(),
        task_id = target.task_id.to_string(),
        branch_name = target.branch_name.clone(),
        error.message = error
    );
}

#[cfg(test)]
mod tests {
    use super::{AggregateDeletionKind, cleanup_git_resources_with_runner};
    use gitlancer::{Git, GitCommand, GitExecError, GitOutput, GitRunner};
    use ora_db::GitCleanupTarget;
    use ora_domain::{ProjectId, TaskId};
    use ora_logging::with_trace_logging;
    use pretty_assertions::assert_eq;
    use std::cell::RefCell;
    use std::path::PathBuf;

    /// Records commands while allowing worktree removal to fail independently of branch deletion.
    #[derive(Debug, Default)]
    struct FailingWorktreeRunner {
        commands: RefCell<Vec<GitCommand>>,
    }

    impl FailingWorktreeRunner {
        /// Returns every Git command attempted by the cleanup flow.
        fn commands(&self) -> Vec<GitCommand> {
            self.commands.borrow().clone()
        }
    }

    impl GitRunner for FailingWorktreeRunner {
        /// Supplies stable discovery output and fails only the worktree removal command.
        fn run(&self, command: &GitCommand) -> Result<GitOutput, GitExecError> {
            self.commands.borrow_mut().push(command.clone());
            match command.args.as_slice() {
                [git_area, subcommand, ..]
                    if git_area == "worktree" && subcommand == "list" =>
                {
                    Ok(GitOutput::new(
                        Some(0),
                        "worktree /repo\nHEAD 1111111\nbranch refs/heads/main\n\nworktree /worktree\nHEAD 2222222\nbranch refs/heads/ora/12345678\n"
                            .to_string(),
                        String::new(),
                        0,
                    ))
                }
                [git_area, subcommand, ..]
                    if git_area == "worktree" && subcommand == "remove" =>
                {
                    Err(GitExecError::NonZeroExit {
                        code: Some(1),
                        args: command.args.clone(),
                        stdout: String::new(),
                        stderr: "worktree is locked".to_string(),
                    })
                }
                [git_area, ..] if git_area == "for-each-ref" => Ok(GitOutput::new(
                    Some(0),
                    "main\nora/12345678\n".to_string(),
                    String::new(),
                    0,
                )),
                [git_area, ..] if git_area == "branch" => {
                    Ok(GitOutput::new(Some(0), String::new(), String::new(), 0))
                }
                _ => panic!("unexpected Git command: {:?}", command.args),
            }
        }
    }

    /// Verifies a failed worktree removal cannot prevent the independent branch attempt.
    #[test]
    fn continues_with_branch_cleanup_after_worktree_failure() {
        with_trace_logging(|| {
            let git = Git::new(FailingWorktreeRunner::default());

            cleanup_git_resources_with_runner(
                AggregateDeletionKind::Task,
                &[cleanup_target("ora/12345678")],
                &git,
            );

            assert_eq!(
                git.runner()
                    .commands()
                    .into_iter()
                    .map(|command| command.args)
                    .collect::<Vec<_>>(),
                vec![
                    vec![
                        "worktree".to_string(),
                        "list".to_string(),
                        "--porcelain".to_string(),
                    ],
                    vec![
                        "worktree".to_string(),
                        "remove".to_string(),
                        "/worktree".to_string(),
                        "--force".to_string(),
                    ],
                    vec![
                        "for-each-ref".to_string(),
                        "--format=%(refname:short)".to_string(),
                        "refs/heads".to_string(),
                    ],
                    vec![
                        "branch".to_string(),
                        "-D".to_string(),
                        "ora/12345678".to_string(),
                    ],
                ]
            );
        });
    }

    /// Verifies an unexpected persisted branch cannot trigger any destructive Git command.
    #[test]
    fn rejects_cleanup_targets_outside_the_task_branch_namespace() {
        with_trace_logging(|| {
            let git = Git::new(FailingWorktreeRunner::default());

            cleanup_git_resources_with_runner(
                AggregateDeletionKind::Project,
                &[cleanup_target("feature/user-work")],
                &git,
            );

            assert_eq!(git.runner().commands(), Vec::<GitCommand>::new());
        });
    }

    /// Builds one target whose task id has the production eight-character branch prefix.
    fn cleanup_target(branch_name: &str) -> GitCleanupTarget {
        GitCleanupTarget {
            project_id: ProjectId::new("project-1"),
            task_id: TaskId::new("12345678-task"),
            repository_root: PathBuf::from("/repo"),
            branch_name: branch_name.to_string(),
        }
    }
}
