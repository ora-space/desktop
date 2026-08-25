pub(crate) mod branch;
mod git_cleanup;
mod git_resource_cleaner;
mod handlers;
mod id_generator;
mod mapper;
mod ports;
mod provisioning;
mod worktree_provisioner;

#[cfg(test)]
mod tests;

pub use branch::{branch_name_for_workspace, workspace_branch_prefix};
pub use git_cleanup::{
    CleanupJobDisposition, CleanupStage, GitCleanupError, RemoveTaskBranchRequest,
    RemoveTaskWorktreeRequest, ResourceRemoval, TaskGitResourceCleaner, WorktreeRemoval,
    legacy_checkout_probe, reduce_cleanup_outcomes, validate_cleanup_identity,
};
pub use git_resource_cleaner::GitTaskResourceCleaner;
pub use handlers::{CreateTaskHandler, GetTaskHandler, ListTasksHandler, UpdateTaskHandler};
pub use id_generator::UuidTaskIdGenerator;
pub use ports::{
    CreateTaskWorktreeRequest, CreateTaskWorktreeResponse, DeleteTaskWorktreeRequest,
    TaskIdGenerator, TaskRepository, TaskWorktreeDeletionMode, TaskWorktreeProvisioner,
    TaskWorktreeProvisionerError,
};
pub use provisioning::{
    PROVISIONING_LEASE_DURATION_MS, ProvisioningLeaseRenewal, TaskWorkspaceCommit,
    WorkspaceCommitOutcome, WorktreeProvisioningLeaseStore,
};
pub use worktree_provisioner::GitTaskWorktreeProvisioner;
