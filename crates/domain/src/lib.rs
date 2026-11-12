mod agent_definition;
mod agent_ref;
mod audit_fields;
mod error;
mod git_cleanup;
mod ids;
mod namespace;
mod plugin_id;
mod plugin_namespace;
mod project;
mod prompt_inactivity_policy;
mod scheduled_artifact;
mod scheduled_run;
mod scheduled_task;
mod session;
mod session_title;
mod skill;
mod task;
mod workflow;
mod workflow_run;
mod workspace;
mod worktree;

#[cfg(test)]
mod tests;

pub use agent_definition::AgentDefinition;
pub use agent_ref::AgentRef;
pub use audit_fields::AuditFields;
pub use error::DomainModelError;
pub use git_cleanup::{
    GitCleanupJob, GitCleanupJobState, MAX_CLEANUP_JOB_ERROR_CHARS, WorktreeProvisioningLease,
    truncate_cleanup_error,
};
pub use ids::{
    AgentDefinitionId, GitCleanupJobId, ProjectId, ScheduledRunId, ScheduledTaskId, SessionId,
    SkillId, TaskId, WorkflowId, WorkflowNodeRunId, WorkflowRunId, WorkflowScopeId,
    WorkflowSnapshotId, WorkspaceId, WorktreeProvisioningLeaseId,
};
pub use namespace::Namespace;
pub use plugin_id::{PluginId, PluginIdError, PluginIdSegment};
pub use plugin_namespace::{MAX_PLUGIN_NAMESPACE_BYTES, PluginNamespace};
pub use project::Project;
pub use prompt_inactivity_policy::PromptInactivityPolicy;
pub use scheduled_artifact::{
    MAX_SCHEDULED_ARTIFACT_BYTES, MAX_SCHEDULED_ARTIFACT_RUNS_PER_TASK,
    MAX_SCHEDULED_ARTIFACT_TOTAL_BYTES, MAX_SCHEDULED_ARTIFACTS_PER_RUN,
    SCHEDULED_ARTIFACT_DIRECTORY, ScheduledArtifactKind, ScheduledArtifactRejection,
    ScheduledArtifactRejectionReason, ScheduledRunArtifact,
};
pub use scheduled_run::{
    MAX_CONCURRENT_LOCAL_SCHEDULED_RUNS, MAX_SCHEDULED_OCCURRENCE_LATENESS_MILLIS,
    MAX_SCHEDULED_RUN_VERDICT_DETAIL_CHARS, MAX_SCHEDULED_TASK_HISTORY_ENTRIES,
    ScheduledMissReason, ScheduledMissedOccurrence, ScheduledOccurrence, ScheduledOccurrenceTiming,
    ScheduledRun, ScheduledRunFailureReason, ScheduledRunSnapshot, ScheduledRunStatus,
    ScheduledRunTransition, ScheduledRunVerdict, ScheduledRunVerdictAnnotation,
    ScheduledTaskHistoryEntry, parse_scheduled_run_verdict,
};
pub use scheduled_task::{
    FULL_AUTO_IN_WORKTREE_POLICY, ScheduledTask, ScheduledTaskApproval,
    ScheduledTaskExecutionLocation, ScheduledTaskSchedule, ScheduledTaskWeekday,
};
pub use session::{
    HistoryState, Session, SessionHostCapabilities, SessionMcpSelection, SessionStatus,
};
pub use session_title::{MAX_SESSION_TITLE_CHARS, SessionTitle, SessionTitleError};
pub use skill::{
    BACKUP_DIR_NAME, JOURNAL_DIR_NAME, STAGING_DIR_NAME, Skill, SkillDescriptionError,
    SkillNameError, SkillOrigin, validate_skill_description, validate_skill_name,
};
pub use task::Task;
pub use workflow::{
    CreatedWorkflow, Workflow, WorkflowDetail, WorkflowSnapshot, WorkflowSummary, WorkflowVersion,
};
pub use workflow_run::{
    WorkflowExecutionScope, WorkflowNodeRun, WorkflowNodeStatus, WorkflowRun, WorkflowRunDetail,
    WorkflowRunStatus, WorkflowRunSummary, WorkflowScopeStatus,
};
pub use workspace::{
    Workspace, WorkspaceKind, WorkspaceLifecycle, WorkspaceLocation, WorkspaceProvisionerKind,
    WorkspaceProvisioning, WorkspaceProvisioningState,
};
pub use worktree::{Worktree, WorktreeActivity, WorktreeBaseline};
