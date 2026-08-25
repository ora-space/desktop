use crate::{
    DomainModelError, GitCleanupJobId, ProjectId, WorkspaceId, WorktreeProvisioningLeaseId,
};
use serde::{Deserialize, Serialize};

/// Maximum stored length of a cleanup job's `last_error`, so repeated failures
/// cannot grow the table with unbounded error text.
pub const MAX_CLEANUP_JOB_ERROR_CHARS: usize = 1024;

/// Models the lifecycle of one durable Git cleanup job.
///
/// `Pending` covers both "never attempted" and "waiting for its next retry";
/// `Completed` is the only business-success terminal state; `ManualAttention`
/// records that automatic processing stopped and an operator must intervene.
/// There is deliberately no persisted `Running` state: execution ownership is
/// an in-process concern, and a crash mid-execution must leave the job pending
/// so restart reconciliation replays it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GitCleanupJobState {
    Pending,
    Completed,
    ManualAttention,
}

impl GitCleanupJobState {
    /// Returns the stable text stored in the database for this state.
    pub fn database_value(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Completed => "completed",
            Self::ManualAttention => "manual_attention",
        }
    }

    /// Converts a persisted state string into the strongly typed state.
    pub fn from_database_value(value: &str) -> Result<Self, DomainModelError> {
        match value {
            "pending" => Ok(Self::Pending),
            "completed" => Ok(Self::Completed),
            "manual_attention" => Ok(Self::ManualAttention),
            other => Err(DomainModelError::InvalidGitCleanupJobState(
                other.to_string(),
            )),
        }
    }
}

/// Represents one durable Git cleanup job persisted in the same transaction as
/// the aggregate deletion that produced it.
///
/// One job owns the Git resource pair (linked worktree + local branch) of one
/// worktree-backed workspace, so sibling workspaces fail and retry independently.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitCleanupJob {
    pub id: GitCleanupJobId,
    pub project_id: ProjectId,
    pub workspace_id: WorkspaceId,
    /// Root path of the project's Git repository as persisted at creation time.
    pub repository_root: String,
    /// Exact checkout path persisted when the worktree was provisioned; `None`
    /// for rows created before checkout paths were recorded.
    pub checkout_root: Option<String>,
    pub branch_name: String,
    pub state: GitCleanupJobState,
    pub attempts: i64,
    /// Earliest timestamp (millis) at which the worker may execute this job.
    pub next_attempt_at: i64,
    pub last_attempt_at: Option<i64>,
    pub last_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl GitCleanupJob {
    /// Creates a pending job that is immediately eligible for execution.
    pub fn pending(
        id: GitCleanupJobId,
        project_id: ProjectId,
        workspace_id: WorkspaceId,
        repository_root: impl Into<String>,
        checkout_root: Option<String>,
        branch_name: impl Into<String>,
        now: i64,
    ) -> Self {
        Self {
            id,
            project_id,
            workspace_id,
            repository_root: repository_root.into(),
            checkout_root,
            branch_name: branch_name.into(),
            state: GitCleanupJobState::Pending,
            attempts: 0,
            next_attempt_at: now,
            last_attempt_at: None,
            last_error: None,
            created_at: now,
            updated_at: now,
        }
    }
}

/// Truncates cleanup error text to the persisted maximum length on a char boundary.
pub fn truncate_cleanup_error(error: impl Into<String>) -> String {
    let error = error.into();
    match error.char_indices().nth(MAX_CLEANUP_JOB_ERROR_CHARS) {
        Some((byte_index, _)) => error[..byte_index].to_string(),
        None => error,
    }
}

/// Represents a write-ahead lease covering one in-flight worktree provisioning.
///
/// The lease is written before `git worktree add` runs and deleted in the same
/// transaction that persists the workspace and worktree rows. The provisioning flow
/// renews `lease_expires_at` while slow Git work is running, so an expired
/// lease proves its owner died (or gave up) and the provisioned Git resources
/// can be reclaimed through a regular cleanup job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeProvisioningLease {
    pub id: WorktreeProvisioningLeaseId,
    pub project_id: ProjectId,
    pub workspace_id: WorkspaceId,
    pub repository_root: String,
    pub checkout_root: String,
    pub branch_name: String,
    pub lease_expires_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

impl WorktreeProvisioningLease {
    /// Creates a lease covering one provisioning attempt until the given expiry.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: WorktreeProvisioningLeaseId,
        project_id: ProjectId,
        workspace_id: WorkspaceId,
        repository_root: impl Into<String>,
        checkout_root: impl Into<String>,
        branch_name: impl Into<String>,
        lease_expires_at: i64,
        now: i64,
    ) -> Self {
        Self {
            id,
            project_id,
            workspace_id,
            repository_root: repository_root.into(),
            checkout_root: checkout_root.into(),
            branch_name: branch_name.into(),
            lease_expires_at,
            created_at: now,
            updated_at: now,
        }
    }

    /// Converts an expired lease into the cleanup job that reclaims its Git resources.
    pub fn into_cleanup_job(self, job_id: GitCleanupJobId, now: i64) -> GitCleanupJob {
        GitCleanupJob::pending(
            job_id,
            self.project_id,
            self.workspace_id,
            self.repository_root,
            Some(self.checkout_root),
            self.branch_name,
            now,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{GitCleanupJobState, MAX_CLEANUP_JOB_ERROR_CHARS, truncate_cleanup_error};
    use pretty_assertions::assert_eq;

    /// Verifies the persisted state strings round-trip through the typed state.
    #[test]
    fn job_state_round_trips_through_database_values() {
        for state in [
            GitCleanupJobState::Pending,
            GitCleanupJobState::Completed,
            GitCleanupJobState::ManualAttention,
        ] {
            assert_eq!(
                GitCleanupJobState::from_database_value(state.database_value()),
                Ok(state)
            );
        }
        assert!(GitCleanupJobState::from_database_value("running").is_err());
    }

    /// Verifies error truncation respects char boundaries and the configured maximum.
    #[test]
    fn truncates_error_text_to_the_configured_maximum() {
        let long_error = "错".repeat(MAX_CLEANUP_JOB_ERROR_CHARS + 10);
        assert_eq!(
            truncate_cleanup_error(long_error).chars().count(),
            MAX_CLEANUP_JOB_ERROR_CHARS
        );
        assert_eq!(truncate_cleanup_error("short"), "short");
    }
}
