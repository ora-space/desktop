mod artifact;
mod audit_fields;
mod error;
mod ids;
mod project;
mod project_work_context;
mod session;
mod task;
mod virtual_entry;
mod virtual_folder;
mod worktree;

#[cfg(test)]
mod tests;

pub use artifact::Artifact;
pub use audit_fields::AuditFields;
pub use error::DomainModelError;
pub use ids::{
    ArtifactId, ProjectId, ProjectWorkContextId, SessionId, TaskId, VirtualEntryId,
    VirtualFolderId, WorktreeId,
};
pub use project::Project;
pub use project_work_context::{ProjectWorkContext, ProjectWorkContextSurface};
pub use session::{AgentId, Session, SessionStatus};
pub use task::{Task, TaskStatus};
pub use virtual_entry::{VirtualEntry, VirtualEntryKind};
pub use virtual_folder::VirtualFolder;
pub use worktree::{Worktree, WorktreeActivity};
