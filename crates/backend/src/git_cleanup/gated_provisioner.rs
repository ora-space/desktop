use super::KeyedResourceLocks;
use ora_application::{
    CreateTaskWorktreeRequest, CreateTaskWorktreeResponse, DeleteTaskWorktreeRequest,
    TaskWorktreeProvisioner, TaskWorktreeProvisionerError,
};
use std::sync::Arc;

/// Serializes a provisioner's Git mutations with cleanup through the shared
/// per-repository gate.
///
/// Only the mutating operations take the gate: `git worktree add/remove`
/// contend on Git's own lock files with concurrent cleanup of sibling tasks,
/// while validation and branch listing are read-only and stay gate-free so the
/// gate never throttles harmless lookups.
pub(crate) struct GatedWorktreeProvisioner<Inner> {
    inner: Inner,
    gates: Arc<KeyedResourceLocks>,
    repository_key: String,
}

impl<Inner> GatedWorktreeProvisioner<Inner> {
    pub(crate) fn new(
        inner: Inner,
        gates: Arc<KeyedResourceLocks>,
        repository_key: String,
    ) -> Self {
        Self {
            inner,
            gates,
            repository_key,
        }
    }
}

impl<Inner> TaskWorktreeProvisioner for GatedWorktreeProvisioner<Inner>
where
    Inner: TaskWorktreeProvisioner,
{
    fn validate_repository(&self) -> Result<(), TaskWorktreeProvisionerError> {
        self.inner.validate_repository()
    }

    fn task_branch_exists(&self, branch_name: &str) -> Result<bool, TaskWorktreeProvisionerError> {
        self.inner.task_branch_exists(branch_name)
    }

    fn create_task_worktree(
        &self,
        request: CreateTaskWorktreeRequest,
    ) -> Result<CreateTaskWorktreeResponse, TaskWorktreeProvisionerError> {
        let _gate = self.gates.acquire_exclusive(self.repository_key.clone());
        self.inner.create_task_worktree(request)
    }

    fn delete_task_worktree(
        &self,
        request: DeleteTaskWorktreeRequest,
    ) -> Result<(), TaskWorktreeProvisionerError> {
        let _gate = self.gates.acquire_exclusive(self.repository_key.clone());
        self.inner.delete_task_worktree(request)
    }
}
