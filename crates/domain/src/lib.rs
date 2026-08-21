mod agent_cli;
mod agent_definition;
mod agent_ref;
mod audit_fields;
mod error;
mod git_cleanup;
mod ids;
mod namespace;
mod plugin;
mod plugin_id;
mod project;
mod session;
mod session_title;
mod skill;
mod task;
mod workflow;
mod workflow_run;
mod worktree;

#[cfg(test)]
mod tests;

pub use agent_cli::AgentCli;
pub use agent_definition::AgentDefinition;
pub use agent_ref::AgentRef;
pub use audit_fields::AuditFields;
pub use error::DomainModelError;
pub use git_cleanup::{
    GitCleanupJob, GitCleanupJobState, MAX_CLEANUP_JOB_ERROR_CHARS, WorktreeProvisioningLease,
    truncate_cleanup_error,
};
pub use ids::{
    AgentDefinitionId, GitCleanupJobId, ProjectId, SessionId, SkillId, TaskId, WorkflowId,
    WorkflowNodeRunId, WorkflowRunId, WorkflowSnapshotId, WorktreeId, WorktreeProvisioningLeaseId,
};
pub use namespace::Namespace;
pub use plugin::{PluginEnabledState, PluginState};
pub use plugin_id::{PluginId, PluginIdError, PluginIdSegment};
pub use project::Project;
pub use session::{HistoryState, Session, SessionStatus};
pub use session_title::{MAX_SESSION_TITLE_CHARS, SessionTitle, SessionTitleError};
pub use skill::{
    BACKUP_DIR_NAME, JOURNAL_DIR_NAME, STAGING_DIR_NAME, Skill, SkillDescriptionError,
    SkillNameError, validate_skill_description, validate_skill_name,
};
pub use task::{Task, TaskType};
pub use workflow::{
    CreatedWorkflow, Workflow, WorkflowDetail, WorkflowSnapshot, WorkflowSummary, WorkflowVersion,
};
pub use workflow_run::{
    WorkflowNodeRun, WorkflowNodeStatus, WorkflowRun, WorkflowRunDetail, WorkflowRunStatus,
    WorkflowRunSummary,
};
pub use worktree::{Worktree, WorktreeActivity, WorktreeBaseline};
