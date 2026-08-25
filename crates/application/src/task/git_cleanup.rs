use crate::BoxRepositorySource;
use crate::task::branch::branch_name_for_workspace;
use ora_domain::GitCleanupJob;
use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

/// Names the cleanup stage an error or log line belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupStage {
    Worktree,
    Branch,
}

impl CleanupStage {
    /// Returns the stable text used in structured log fields.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Worktree => "worktree",
            Self::Branch => "branch",
        }
    }
}

/// Reports one resource confirmed gone: either removed now or already absent.
///
/// "Already absent" is a positive confirmation (the resource provably does not
/// exist), never a default for "could not resolve".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceRemoval {
    Removed,
    AlreadyAbsent,
}

/// Reports the worktree stage outcome, which unlike the branch stage can end
/// in "a directory occupies the recorded path but Git metadata cannot prove
/// Ora still owns it" — that checkout must not be force-removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeRemoval {
    Removed,
    AlreadyAbsent,
    OwnershipLost,
}

/// Captures a cleanup operation failure with its stage and typed source chain.
#[derive(Debug, Error)]
#[error("git cleanup {stage} stage failed: {context}", stage = stage.as_str())]
pub struct GitCleanupError {
    pub stage: CleanupStage,
    pub context: &'static str,
    #[source]
    pub source: BoxRepositorySource,
}

impl GitCleanupError {
    /// Wraps one failed Git operation while keeping the stage and source chain.
    pub fn new(
        stage: CleanupStage,
        context: &'static str,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            stage,
            context,
            source: Box::new(source),
        }
    }

    /// Renders the full cause chain into one line for persisted job errors.
    ///
    /// `Display` alone would drop the underlying Git stderr (`Permission
    /// denied`, `is not empty`, …), which is exactly what an operator needs to
    /// diagnose a half-failed removal from `last_error`.
    pub fn error_chain(&self) -> String {
        let mut rendered = self.to_string();
        let mut source = std::error::Error::source(self);
        while let Some(cause) = source {
            rendered.push_str(&format!(": {cause}"));
            source = cause.source();
        }
        rendered
    }
}

/// Identifies the worktree resources one cleanup job must remove.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveTaskWorktreeRequest {
    pub repository_root: PathBuf,
    pub branch_name: String,
    /// Checkout path persisted at provisioning time; ownership evidence for
    /// resolving detached worktrees.
    pub checkout_root: Option<PathBuf>,
    /// Exact probe path derived from the currently configured worktree root,
    /// used only for legacy rows without a persisted checkout path.
    pub legacy_checkout_probe: Option<PathBuf>,
}

/// Identifies the local branch one cleanup job must remove.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveTaskBranchRequest {
    pub repository_root: PathBuf,
    pub branch_name: String,
}

/// Executes the physical Git removals for one cleanup job.
///
/// Implementations must be idempotent (`AlreadyAbsent` on replay), must never
/// remove a checkout they cannot prove Ora owns (`OwnershipLost` instead), and
/// must keep the two resources independent: a worktree failure never prevents
/// the branch attempt, which the caller drives as a separate call.
pub trait TaskGitResourceCleaner {
    /// Resolves and force-removes the task's linked worktree.
    fn remove_worktree(
        &self,
        request: RemoveTaskWorktreeRequest,
    ) -> Result<WorktreeRemoval, GitCleanupError>;

    /// Force-deletes the task's Ora-owned local branch.
    fn remove_branch(
        &self,
        request: RemoveTaskBranchRequest,
    ) -> Result<ResourceRemoval, GitCleanupError>;
}

/// Names the persisted state transition one finished cleanup execution maps to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupJobDisposition {
    Completed,
    Retry { error: String },
    ManualAttention { reason: String },
}

/// Validates the safety invariants a job must satisfy before any Git command runs.
///
/// A violation is a data-corruption signal, not a retryable failure: the job
/// identity was written by Ora itself, so a mismatch means the row cannot be
/// trusted as a deletion target.
pub fn validate_cleanup_identity(job: &GitCleanupJob) -> Result<(), String> {
    if Uuid::parse_str(job.workspace_id.as_ref()).is_err() {
        return Err(format!(
            "workspace id is not a full UUID: {}",
            job.workspace_id.as_ref()
        ));
    }
    let expected_branch = branch_name_for_workspace(&job.workspace_id);
    if job.branch_name != expected_branch {
        return Err(format!(
            "branch name {} does not match the branch derived from the workspace id ({expected_branch})",
            job.branch_name
        ));
    }
    Ok(())
}

/// Reduces both stage results into the job's persisted state transition.
///
/// Pure by design: the worker executes Git, this function alone decides what
/// the outcome means. Ownership loss dominates (manual attention, never
/// retried), any error is retryable, and only two positively-confirmed
/// removals complete the job.
pub fn reduce_cleanup_outcomes(
    worktree: &Result<WorktreeRemoval, GitCleanupError>,
    branch: &Result<ResourceRemoval, GitCleanupError>,
) -> CleanupJobDisposition {
    if let Ok(WorktreeRemoval::OwnershipLost) = worktree {
        let mut reason =
            "non-empty checkout exists but Git metadata cannot prove Ora ownership".to_string();
        if let Err(branch_error) = branch {
            reason.push_str(&format!(
                "; branch stage also failed: {}",
                branch_error.error_chain()
            ));
        }
        return CleanupJobDisposition::ManualAttention { reason };
    }

    let mut errors = Vec::new();
    if let Err(error) = worktree {
        errors.push(error.error_chain());
    }
    if let Err(error) = branch {
        errors.push(error.error_chain());
    }
    if errors.is_empty() {
        CleanupJobDisposition::Completed
    } else {
        CleanupJobDisposition::Retry {
            error: errors.join("; "),
        }
    }
}

/// Derives the exact legacy probe path for jobs without a persisted checkout root.
///
/// The probe is only trusted as a full-workspace-id directory under the currently
/// configured worktree root; anything else would be ownership guessing.
pub fn legacy_checkout_probe(configured_worktree_root: &Path, job: &GitCleanupJob) -> PathBuf {
    configured_worktree_root.join(job.workspace_id.as_ref())
}

#[cfg(test)]
mod tests {
    use super::{
        CleanupJobDisposition, CleanupStage, GitCleanupError, ResourceRemoval, WorktreeRemoval,
        reduce_cleanup_outcomes, validate_cleanup_identity,
    };
    use ora_domain::{GitCleanupJob, GitCleanupJobId, ProjectId, WorkspaceId};
    use pretty_assertions::assert_eq;

    fn job(task_id: &str, branch: &str) -> GitCleanupJob {
        GitCleanupJob::pending(
            GitCleanupJobId::new("job-1"),
            ProjectId::new("project-1"),
            WorkspaceId::new(task_id),
            "/repo",
            None,
            branch,
            1,
        )
    }

    fn stage_error(stage: CleanupStage) -> GitCleanupError {
        GitCleanupError::new(stage, "failed to run git", std::io::Error::other("boom"))
    }

    /// Verifies a well-formed UUID workspace id with its derived branch passes validation.
    #[test]
    fn accepts_matching_identity() {
        let job = job("1c56dee5-4d3f-4f83-bb47-6d9d0dbf5b1a", "ora/1c56dee5");
        assert_eq!(validate_cleanup_identity(&job), Ok(()));
    }

    /// Verifies non-UUID ids and mismatched branches are rejected as untrusted identity.
    #[test]
    fn rejects_malformed_identity() {
        assert!(validate_cleanup_identity(&job("not-a-uuid", "ora/not-a-uu")).is_err());
        assert!(
            validate_cleanup_identity(&job("1c56dee5-4d3f-4f83-bb47-6d9d0dbf5b1a", "ora/deadbeef"))
                .is_err()
        );
    }

    /// Verifies every stage-result combination reduces to the designed transition.
    #[test]
    fn reduces_stage_combinations_exhaustively() {
        use CleanupJobDisposition as D;

        let worktree_ok = [WorktreeRemoval::Removed, WorktreeRemoval::AlreadyAbsent];
        let branch_ok = [ResourceRemoval::Removed, ResourceRemoval::AlreadyAbsent];

        // Both stages confirmed gone -> completed.
        for worktree in worktree_ok {
            for branch in branch_ok {
                assert_eq!(
                    reduce_cleanup_outcomes(&Ok(worktree), &Ok(branch)),
                    D::Completed
                );
            }
        }

        // Ownership loss dominates every branch result.
        for branch in [
            Ok(ResourceRemoval::Removed),
            Err(stage_error(CleanupStage::Branch)),
        ] {
            assert!(matches!(
                reduce_cleanup_outcomes(&Ok(WorktreeRemoval::OwnershipLost), &branch),
                D::ManualAttention { .. }
            ));
        }

        // Any error without ownership loss is retryable, and both errors are reported.
        assert!(matches!(
            reduce_cleanup_outcomes(
                &Err(stage_error(CleanupStage::Worktree)),
                &Ok(ResourceRemoval::Removed)
            ),
            D::Retry { .. }
        ));
        let both = reduce_cleanup_outcomes(
            &Err(stage_error(CleanupStage::Worktree)),
            &Err(stage_error(CleanupStage::Branch)),
        );
        match both {
            D::Retry { error } => {
                assert!(error.contains("worktree stage"));
                assert!(error.contains("branch stage"));
            }
            other => panic!("expected retry, got {other:?}"),
        }
    }
}
