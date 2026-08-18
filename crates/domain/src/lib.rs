mod agent_definition;
mod audit_fields;
mod error;
mod git_cleanup;
mod ids;
mod namespace;
mod plugin;
mod project;
mod session;
mod session_title;
mod skill;
mod task;
mod task_diff_comment;
mod workflow;
mod workflow_run;
mod worktree;

#[cfg(test)]
mod tests;

pub use agent_definition::AgentDefinition;
pub use audit_fields::AuditFields;
pub use error::DomainModelError;
pub use git_cleanup::{
    GitCleanupJob, GitCleanupJobState, MAX_CLEANUP_JOB_ERROR_CHARS, WorktreeProvisioningLease,
    truncate_cleanup_error,
};
pub use ids::{
    AgentDefinitionId, GitCleanupJobId, PluginId, ProjectId, SessionId, SkillId, TaskDiffCommentId,
    TaskId, WorkflowId, WorkflowNodeRunId, WorkflowRunId, WorkflowSnapshotId, WorktreeId,
    WorktreeProvisioningLeaseId,
};
pub use namespace::Namespace;
pub use plugin::{PluginEnabledState, PluginState};
pub use project::Project;
pub use session::{AgentCli, HistoryState, Session, SessionStatus};
pub use session_title::{MAX_SESSION_TITLE_CHARS, SessionTitle, SessionTitleError};
pub use skill::{
    BACKUP_DIR_NAME, JOURNAL_DIR_NAME, STAGING_DIR_NAME, Skill, SkillDescriptionError,
    SkillNameError, validate_skill_description, validate_skill_name,
};
pub use task::{Task, TaskType};
pub use task_diff_comment::{
    TaskDiffAnchor, TaskDiffComment, TaskDiffCommentKind, TaskDiffSide, TaskDiffThreadStatus,
};
pub use workflow::{
    CreatedWorkflow, Workflow, WorkflowDetail, WorkflowSnapshot, WorkflowSummary, WorkflowVersion,
};
pub use workflow_run::{
    WorkflowNodeRun, WorkflowNodeStatus, WorkflowRun, WorkflowRunDetail, WorkflowRunStatus,
    WorkflowRunSummary,
};
pub use worktree::{Worktree, WorktreeActivity, WorktreeBaseline};
