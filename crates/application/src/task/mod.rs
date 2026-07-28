mod branch;
mod handlers;
mod id_generator;
mod mapper;
mod ports;
mod worktree_provisioner;

#[cfg(test)]
mod tests;

pub use branch::{branch_name_for_task, branch_prefix_for_task};
pub use handlers::{CreateTaskHandler, GetTaskHandler, ListTasksHandler, UpdateTaskHandler};
pub use id_generator::UuidTaskIdGenerator;
pub use ports::{
    CreateTaskWorktreeRequest, DeleteTaskWorktreeRequest, TaskIdGenerator, TaskRepository,
    TaskRepositoryError, TaskWorktreeDeletionMode, TaskWorktreeProvisioner,
    TaskWorktreeProvisionerError,
};
pub use worktree_provisioner::GitTaskWorktreeProvisioner;
