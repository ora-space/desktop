mod agent;
mod file_system;
mod frontend;
mod project;
mod project_work_context;
mod session;
mod skill;
mod task;

pub use agent::{
    Agent, CreateAgentRequest, CreateAgentResponse, DeleteAgentRequest, DeleteAgentResponse,
    GetAgentRequest, GetAgentResponse, ListAgentsRequest, ListAgentsResponse, UpdateAgentRequest,
    UpdateAgentResponse,
};
pub use file_system::{
    FileSystemBreadcrumb, FileSystemEntry, FileSystemEntryKind, ListDirectoryRequest,
    ListDirectoryResponse,
};
pub use frontend::{
    AGENT_PATH, AGENTS_PATH, FILE_SYSTEM_DIRECTORY_PATH, FrontendEndpoint, FrontendHttpMethod,
    FrontendPathParam, FrontendQueryParam, PROJECT_PATH, PROJECT_WORK_CONTEXT_OPEN_PATH,
    PROJECT_WORK_CONTEXT_RENEW_PATH, PROJECTS_PATH, SESSION_PATH, SESSIONS_PATH, SKILL_PATH,
    SKILLS_PATH, TASK_PATH, TASKS_PATH, frontend_endpoints,
};
pub use project::{
    CreateProjectRequest, CreateProjectResponse, DeleteProjectRequest, DeleteProjectResponse,
    GetProjectRequest, GetProjectResponse, ListProjectsRequest, ListProjectsResponse, Project,
    UpdateProjectRequest, UpdateProjectResponse,
};
pub use project_work_context::{
    OpenProjectWorkContextRequest, OpenProjectWorkContextResponse, ProjectWorkContext,
    ProjectWorkContextSurface, RenewProjectWorkContextRequest, RenewProjectWorkContextResponse,
};
pub use session::{
    CreateSessionRequest, CreateSessionResponse, DeleteSessionRequest, DeleteSessionResponse,
    GetSessionRequest, GetSessionResponse, ListSessionsRequest, ListSessionsResponse, Session,
    SessionStatus, UpdateSessionRequest, UpdateSessionResponse,
};
pub use skill::{
    CreateSkillRequest, CreateSkillResponse, DeleteSkillRequest, DeleteSkillResponse,
    GetSkillRequest, GetSkillResponse, ListSkillsRequest, ListSkillsResponse, Skill,
    UpdateSkillRequest, UpdateSkillResponse,
};
use std::path::Path;
pub use task::{
    CreateTaskRequest, CreateTaskResponse, DeleteTaskRequest, DeleteTaskResponse, GetTaskRequest,
    GetTaskResponse, ListTasksRequest, ListTasksResponse, Task, TaskStatus, UpdateTaskRequest,
    UpdateTaskResponse,
};
use ts_rs::{Config, ExportError, TS};

/// Exports every contract DTO family into the shared TypeScript package for frontend consumers.
pub fn export_typescript_bindings_to(
    output_directory: impl AsRef<Path>,
) -> Result<(), ExportError> {
    let config = Config::new().with_out_dir(output_directory.as_ref());

    Agent::export(&config)?;
    CreateAgentRequest::export(&config)?;
    CreateAgentResponse::export(&config)?;
    GetAgentRequest::export(&config)?;
    GetAgentResponse::export(&config)?;
    ListAgentsRequest::export(&config)?;
    ListAgentsResponse::export(&config)?;
    UpdateAgentRequest::export(&config)?;
    UpdateAgentResponse::export(&config)?;
    DeleteAgentRequest::export(&config)?;
    DeleteAgentResponse::export(&config)?;
    FileSystemEntryKind::export(&config)?;
    FileSystemEntry::export(&config)?;
    FileSystemBreadcrumb::export(&config)?;
    ListDirectoryRequest::export(&config)?;
    ListDirectoryResponse::export(&config)?;
    Project::export(&config)?;
    CreateProjectRequest::export(&config)?;
    CreateProjectResponse::export(&config)?;
    GetProjectRequest::export(&config)?;
    GetProjectResponse::export(&config)?;
    ListProjectsRequest::export(&config)?;
    ListProjectsResponse::export(&config)?;
    UpdateProjectRequest::export(&config)?;
    UpdateProjectResponse::export(&config)?;
    DeleteProjectRequest::export(&config)?;
    DeleteProjectResponse::export(&config)?;
    ProjectWorkContextSurface::export(&config)?;
    ProjectWorkContext::export(&config)?;
    OpenProjectWorkContextRequest::export(&config)?;
    OpenProjectWorkContextResponse::export(&config)?;
    RenewProjectWorkContextRequest::export(&config)?;
    RenewProjectWorkContextResponse::export(&config)?;

    SessionStatus::export(&config)?;
    Session::export(&config)?;
    CreateSessionRequest::export(&config)?;
    CreateSessionResponse::export(&config)?;
    GetSessionRequest::export(&config)?;
    GetSessionResponse::export(&config)?;
    ListSessionsRequest::export(&config)?;
    ListSessionsResponse::export(&config)?;
    UpdateSessionRequest::export(&config)?;
    UpdateSessionResponse::export(&config)?;
    DeleteSessionRequest::export(&config)?;
    DeleteSessionResponse::export(&config)?;

    Skill::export(&config)?;
    CreateSkillRequest::export(&config)?;
    CreateSkillResponse::export(&config)?;
    GetSkillRequest::export(&config)?;
    GetSkillResponse::export(&config)?;
    ListSkillsRequest::export(&config)?;
    ListSkillsResponse::export(&config)?;
    UpdateSkillRequest::export(&config)?;
    UpdateSkillResponse::export(&config)?;
    DeleteSkillRequest::export(&config)?;
    DeleteSkillResponse::export(&config)?;

    TaskStatus::export(&config)?;
    Task::export(&config)?;
    CreateTaskRequest::export(&config)?;
    CreateTaskResponse::export(&config)?;
    GetTaskRequest::export(&config)?;
    GetTaskResponse::export(&config)?;
    ListTasksRequest::export(&config)?;
    ListTasksResponse::export(&config)?;
    UpdateTaskRequest::export(&config)?;
    UpdateTaskResponse::export(&config)?;
    DeleteTaskRequest::export(&config)?;
    DeleteTaskResponse::export(&config)?;

    Ok(())
}
