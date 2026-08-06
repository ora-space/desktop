use thiserror::Error;

/// Enumerates domain-model conversion failures that adapters must handle explicitly.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DomainModelError {
    #[error("worktree baseline commit must not be empty")]
    EmptyWorktreeBaseline,
    #[error("invalid project work context surface value: {0}")]
    InvalidProjectWorkContextSurface(String),
    #[error("invalid task status value: {0}")]
    InvalidTaskStatus(i64),
    #[error("invalid task type value: {0}")]
    InvalidTaskType(i64),
    #[error("invalid workflow run status value: {0}")]
    InvalidWorkflowRunStatus(i64),
    #[error("invalid workflow node status value: {0}")]
    InvalidWorkflowNodeStatus(i64),
    #[error("invalid worktree activity value: {0}")]
    InvalidWorktreeActivity(i64),
    #[error("invalid virtual entry kind value: {0}")]
    InvalidVirtualEntryKind(i64),
    #[error("invalid session status value: {0}")]
    InvalidSessionStatus(i64),
    #[error("invalid agent CLI value: {0}")]
    InvalidAgentCli(String),
    #[error("skill name must not be blank")]
    EmptySkillName,
    #[error("invalid skill name: {name}")]
    InvalidSkillName { name: String },
    #[error("skill name exceeds the single path segment limit")]
    SkillNameTooLong,
    #[error("skill description must not be blank")]
    EmptySkillDescription,
    #[error("skill description exceeds 4096 bytes")]
    SkillDescriptionTooLarge,
    #[error("agent definition name must not be blank")]
    EmptyAgentDefinitionName,
    #[error("workflow name must not be blank")]
    EmptyWorkflowName,
}
