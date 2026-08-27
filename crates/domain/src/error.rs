use thiserror::Error;

use crate::SessionTitleError;

/// Enumerates domain-model conversion failures that adapters must handle explicitly.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DomainModelError {
    #[error("namespace must not be blank")]
    EmptyNamespace,
    #[error("worktree baseline commit must not be empty")]
    EmptyWorktreeBaseline,
    #[error("invalid workspace kind value: {0}")]
    InvalidWorkspaceKind(String),
    #[error("invalid workspace lifecycle value: {0}")]
    InvalidWorkspaceLifecycle(String),
    #[error("invalid workspace location kind value: {0}")]
    InvalidWorkspaceLocationKind(String),
    #[error("invalid workspace provisioner kind value: {0}")]
    InvalidWorkspaceProvisionerKind(String),
    #[error("invalid workspace provisioning state value: {0}")]
    InvalidWorkspaceProvisioningState(String),
    #[error("invalid workflow run status value: {0}")]
    InvalidWorkflowRunStatus(i64),
    #[error("invalid workflow node status value: {0}")]
    InvalidWorkflowNodeStatus(i64),
    #[error("invalid worktree activity value: {0}")]
    InvalidWorktreeActivity(i64),
    #[error("invalid git cleanup job state value: {0}")]
    InvalidGitCleanupJobState(String),
    #[error("invalid session status value: {0}")]
    InvalidSessionStatus(i64),
    #[error("invalid agent reference: {0}")]
    InvalidAgentRef(String),
    #[error("invalid session title: {0}")]
    InvalidSessionTitle(#[from] SessionTitleError),
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
