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
mod task_diff_comment;
mod virtual_entry;
mod virtual_folder;
mod workflow;
mod workflow_run;
mod worktree;

#[cfg(test)]
mod tests;

pub use agent_definition::AgentDefinition;
pub use artifact::Artifact;
pub use audit_fields::AuditFields;
pub use error::DomainModelError;
pub use ids::{
    AgentDefinitionId, ArtifactId, ProjectId, ProjectWorkContextId, SessionId, SkillId,
    TaskDiffCommentId, TaskId, VirtualEntryId, VirtualFolderId, WorkflowId, WorkflowNodeRunId,
    WorkflowRunId, WorkflowSnapshotId, WorktreeId,
};
pub use project::Project;
pub use project_work_context::{ProjectWorkContext, ProjectWorkContextSurface};
pub use session::{AgentCli, HistoryState, Session, SessionStatus};
pub use skill::{
    Skill, SkillDescriptionError, SkillNameError, validate_skill_description, validate_skill_name,
};
pub use task::{Task, TaskStatus, TaskType};
pub use task_diff_comment::{
    TaskDiffAnchor, TaskDiffComment, TaskDiffCommentKind, TaskDiffSide, TaskDiffThreadStatus,
};
pub use virtual_entry::{VirtualEntry, VirtualEntryKind};
pub use virtual_folder::VirtualFolder;
pub use workflow::{
    CreatedWorkflow, Workflow, WorkflowDetail, WorkflowSnapshot, WorkflowSummary, WorkflowVersion,
};
pub use workflow_run::{
    WorkflowNodeRun, WorkflowNodeStatus, WorkflowRun, WorkflowRunDetail, WorkflowRunStatus,
    WorkflowRunSummary,
};
pub use worktree::{Worktree, WorktreeActivity, WorktreeBaseline};
