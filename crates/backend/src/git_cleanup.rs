use gitlancer::CliGitRunner;
use ora_application::{
    GitResourceCleanupOutcome, GitTaskGitResourceCleaner, TaskGitResourceCleaner,
    TaskGitResourceCleanupRequest, branch_name_for_task,
};
use ora_db::GitCleanupTarget;
use ora_logging::{ora_error, ora_info};
use std::path::Path;
use uuid::Uuid;

/// Identifies which aggregate deletion initiated a best-effort Git cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AggregateDeletionKind {
    Project,
    Task,
}

impl AggregateDeletionKind {
    /// Returns the stable operation field shared by cleanup log events.
    pub(crate) fn operation(self) -> &'static str {
        match self {
            Self::Project => "delete_project",
            Self::Task => "delete_task",
        }
    }
}

/// Carries one committed database response and its post-commit Git cleanup targets.
pub(crate) struct CommittedAggregateDeletion<Response> {
    pub(crate) response: Response,
    pub(crate) git_cleanup_targets: Vec<GitCleanupTarget>,
}

/// Best-effort removes every validated Ora-owned worktree and branch after database commit.
pub(crate) fn cleanup_git_resources(
    deletion_kind: AggregateDeletionKind,
    targets: &[GitCleanupTarget],
    worktree_root: &Path,
) {
    cleanup_git_resources_with_cleaner(
        deletion_kind,
        targets,
        worktree_root,
        &GitTaskGitResourceCleaner::new(CliGitRunner),
    );
}

/// Runs cleanup through an injected application port so backend policy remains testable.
fn cleanup_git_resources_with_cleaner<Cleaner: TaskGitResourceCleaner>(
    deletion_kind: AggregateDeletionKind,
    targets: &[GitCleanupTarget],
    worktree_root: &Path,
    cleaner: &Cleaner,
) {
    for target in targets {
        if Uuid::parse_str(target.task_id.as_ref()).is_err() {
            log_cleanup_failure(
                deletion_kind,
                target,
                "ownership_validation",
                "task id is not a valid UUID".to_string(),
            );
            continue;
        }
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

        let report = cleaner.cleanup_task_git_resources(TaskGitResourceCleanupRequest {
            repository_root: target.repository_root.clone(),
            expected_worktree_root: worktree_root.join(target.task_id.as_ref()),
            branch_name: target.branch_name.clone(),
        });
        log_cleanup_outcome(deletion_kind, target, "worktree", report.worktree);
        log_cleanup_outcome(deletion_kind, target, "branch", report.branch);
    }
}

/// Emits the structured event appropriate for one independent cleanup stage.
fn log_cleanup_outcome(
    deletion_kind: AggregateDeletionKind,
    target: &GitCleanupTarget,
    stage: &'static str,
    outcome: GitResourceCleanupOutcome,
) {
    match outcome {
        GitResourceCleanupOutcome::Removed => {}
        GitResourceCleanupOutcome::AlreadyAbsent => {
            ora_info!(
                message = "aggregate Git resource already absent",
                operation = deletion_kind.operation(),
                cleanup_stage = stage,
                project_id = target.project_id.to_string(),
                task_id = target.task_id.to_string(),
                branch_name = target.branch_name.clone()
            );
        }
        GitResourceCleanupOutcome::Failed { message } => {
            log_cleanup_failure(deletion_kind, target, stage, message);
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
    use super::{AggregateDeletionKind, cleanup_git_resources_with_cleaner};
    use ora_application::{
        GitResourceCleanupOutcome, TaskGitResourceCleaner, TaskGitResourceCleanupReport,
        TaskGitResourceCleanupRequest,
    };
    use ora_db::GitCleanupTarget;
    use ora_domain::{ProjectId, TaskId};
    use ora_logging::with_trace_logging;
    use pretty_assertions::assert_eq;
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    const TASK_ID: &str = "12345678-1234-4234-8234-1234567890ab";

    /// Records validated cleanup requests while returning independent stage failures.
    #[derive(Debug, Default)]
    struct RecordingCleaner {
        requests: RefCell<Vec<TaskGitResourceCleanupRequest>>,
    }

    impl RecordingCleaner {
        /// Returns every cleanup request accepted by the application port.
        fn requests(&self) -> Vec<TaskGitResourceCleanupRequest> {
            self.requests.borrow().clone()
        }
    }

    impl TaskGitResourceCleaner for RecordingCleaner {
        /// Records requests and simulates a failed worktree with a successful branch removal.
        fn cleanup_task_git_resources(
            &self,
            request: TaskGitResourceCleanupRequest,
        ) -> TaskGitResourceCleanupReport {
            self.requests.borrow_mut().push(request);
            TaskGitResourceCleanupReport {
                worktree: GitResourceCleanupOutcome::Failed {
                    message: "worktree is locked".to_string(),
                },
                branch: GitResourceCleanupOutcome::Removed,
            }
        }
    }

    /// Verifies validated targets include the deterministic task checkout root.
    #[test]
    fn delegates_validated_targets_with_the_expected_worktree_root() {
        with_trace_logging(|| {
            let cleaner = RecordingCleaner::default();

            cleanup_git_resources_with_cleaner(
                AggregateDeletionKind::Task,
                &[cleanup_target("ora/12345678")],
                Path::new("/ora/worktrees"),
                &cleaner,
            );

            assert_eq!(
                cleaner.requests(),
                vec![TaskGitResourceCleanupRequest {
                    repository_root: PathBuf::from("/repo"),
                    expected_worktree_root: PathBuf::from(
                        "/ora/worktrees/12345678-1234-4234-8234-1234567890ab",
                    ),
                    branch_name: "ora/12345678".to_string(),
                }]
            );
        });
    }

    /// Verifies an unexpected persisted branch cannot trigger any destructive Git command.
    #[test]
    fn rejects_cleanup_targets_outside_the_task_branch_namespace() {
        with_trace_logging(|| {
            let cleaner = RecordingCleaner::default();

            cleanup_git_resources_with_cleaner(
                AggregateDeletionKind::Project,
                &[cleanup_target("feature/user-work")],
                Path::new("/ora/worktrees"),
                &cleaner,
            );

            assert_eq!(
                cleaner.requests(),
                Vec::<TaskGitResourceCleanupRequest>::new()
            );
        });
    }

    /// Verifies malformed persisted identifiers cannot escape the configured worktree root.
    #[test]
    fn rejects_cleanup_targets_with_non_uuid_task_ids() {
        with_trace_logging(|| {
            let cleaner = RecordingCleaner::default();
            let mut target = cleanup_target("ora/12345678");
            target.task_id = TaskId::new("12345678/../../other-worktree");

            cleanup_git_resources_with_cleaner(
                AggregateDeletionKind::Task,
                &[target],
                Path::new("/ora/worktrees"),
                &cleaner,
            );

            assert_eq!(
                cleaner.requests(),
                Vec::<TaskGitResourceCleanupRequest>::new()
            );
        });
    }

    /// Builds one target whose task id has the production eight-character branch prefix.
    fn cleanup_target(branch_name: &str) -> GitCleanupTarget {
        GitCleanupTarget {
            project_id: ProjectId::new("project-1"),
            task_id: TaskId::new(TASK_ID),
            repository_root: PathBuf::from("/repo"),
            branch_name: branch_name.to_string(),
        }
    }
}
