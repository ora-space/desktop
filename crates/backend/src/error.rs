use ora_application::ApplicationError;
use serde::Serialize;
use std::fmt;

/// Classifies backend failures without coupling the shared layer to HTTP status codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendErrorKind {
    BadRequest,
    NotFound,
    Conflict,
    Unprocessable,
    Internal,
}

/// Carries the stable public error code and message shared by every transport adapter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendError {
    #[serde(skip)]
    kind: BackendErrorKind,
    code: &'static str,
    message: String,
}

impl BackendError {
    /// Creates a backend error from explicit transport-neutral public fields.
    pub fn new(kind: BackendErrorKind, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            code,
            message: message.into(),
        }
    }

    /// Returns the category an adapter can map into its native failure semantics.
    pub fn kind(&self) -> BackendErrorKind {
        self.kind
    }

    /// Returns the stable machine-readable public error code.
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// Returns the human-readable public error message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for BackendError {
    /// Formats the public message without exposing internal source diagnostics.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for BackendError {}

impl From<ApplicationError> for BackendError {
    /// Normalizes application failures into one stable adapter-independent error contract.
    fn from(error: ApplicationError) -> Self {
        match error {
            ApplicationError::SkillNameBlank => Self::new(
                BackendErrorKind::BadRequest,
                "skill_name_blank",
                "skill name must not be blank",
            ),
            ApplicationError::SkillNotFound { skill_id } => Self::new(
                BackendErrorKind::NotFound,
                "skill_not_found",
                format!("skill not found: {skill_id}"),
            ),
            ApplicationError::SkillRepository { .. } => internal(
                "skill_repository_error",
                "skill repository operation failed",
            ),
            ApplicationError::SkillImportInvalid { reason } => Self::new(
                BackendErrorKind::Unprocessable,
                "skill_import_invalid",
                reason,
            ),
            ApplicationError::SkillFolderConflict { name } => Self::new(
                BackendErrorKind::Conflict,
                "skill_folder_conflict",
                format!("skill folder already exists: {name}"),
            ),
            ApplicationError::SkillPackageStorage { .. } => internal(
                "skill_package_storage_error",
                "skill package storage operation failed",
            ),
            ApplicationError::AgentDefinitionNameBlank => Self::new(
                BackendErrorKind::BadRequest,
                "agent_name_blank",
                "agent definition name must not be blank",
            ),
            ApplicationError::AgentDefinitionNotFound { agent_id } => Self::new(
                BackendErrorKind::NotFound,
                "agent_not_found",
                format!("agent definition not found: {agent_id}"),
            ),
            ApplicationError::AgentDefinitionRepository { .. } => internal(
                "agent_repository_error",
                "agent repository operation failed",
            ),
            ApplicationError::ProjectNotFound { project_id } => Self::new(
                BackendErrorKind::NotFound,
                "project_not_found",
                format!("project not found: {project_id}"),
            ),
            ApplicationError::ProjectRepository { .. } => internal(
                "project_repository_error",
                "project repository operation failed",
            ),
            ApplicationError::ProjectOccupied { project_id } => Self::new(
                BackendErrorKind::Conflict,
                "project_occupied",
                format!("project is already occupied: {project_id}"),
            ),
            ApplicationError::ProjectWorkContextNotFound { surface, window_id } => Self::new(
                BackendErrorKind::NotFound,
                "project_work_context_not_found",
                format!("project work context not found for {surface}/{window_id}"),
            ),
            ApplicationError::ProjectWorkContextRepository { .. } => internal(
                "project_work_context_repository_error",
                "project work context repository operation failed",
            ),
            ApplicationError::TaskNotFound { task_id } => Self::new(
                BackendErrorKind::NotFound,
                "task_not_found",
                format!("task not found: {task_id}"),
            ),
            ApplicationError::TaskRepository { .. } => {
                internal("task_repository_error", "task repository operation failed")
            }
            ApplicationError::TaskWorktree { .. } => {
                internal("task_worktree_error", "task worktree operation failed")
            }
            ApplicationError::WorktreeNotFound { worktree_id } => Self::new(
                BackendErrorKind::NotFound,
                "worktree_not_found",
                format!("worktree not found: {worktree_id}"),
            ),
            ApplicationError::WorktreeRepository { .. } => internal(
                "worktree_repository_error",
                "worktree repository operation failed",
            ),
            ApplicationError::SessionNotFound { session_id } => Self::new(
                BackendErrorKind::NotFound,
                "session_not_found",
                format!("session not found: {session_id}"),
            ),
            ApplicationError::SessionRepository { .. } => internal(
                "session_repository_error",
                "session repository operation failed",
            ),
        }
    }
}

/// Builds a sanitized internal failure without leaking repository or filesystem diagnostics.
fn internal(code: &'static str, message: &'static str) -> BackendError {
    BackendError::new(BackendErrorKind::Internal, code, message)
}
