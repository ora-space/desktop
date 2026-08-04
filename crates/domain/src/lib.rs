mod agent_definition;
mod artifact;
mod audit_fields;
mod error;
mod ids;
mod project;
mod project_work_context;
mod session;
mod skill;
mod task;
mod virtual_entry;
mod virtual_folder;
mod worktree;

#[cfg(test)]
mod tests;

pub use agent_definition::AgentDefinition;
pub use artifact::Artifact;
pub use audit_fields::AuditFields;
pub use error::DomainModelError;
pub use ids::{
    AgentDefinitionId, ArtifactId, ProjectId, ProjectWorkContextId, SessionId, SkillId, TaskId,
    VirtualEntryId, VirtualFolderId, WorktreeId,
};
pub use project::Project;
pub use project_work_context::{ProjectWorkContext, ProjectWorkContextSurface};
pub use session::{AgentCli, Session, SessionStatus};
pub use skill::{
    Skill, SkillDescriptionError, SkillNameError, validate_skill_description, validate_skill_name,
};
pub use task::{Task, TaskStatus};
pub use virtual_entry::{VirtualEntry, VirtualEntryKind};
pub use virtual_folder::VirtualFolder;
pub use worktree::{Worktree, WorktreeActivity};
