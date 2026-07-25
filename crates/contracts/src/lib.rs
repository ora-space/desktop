pub mod acp;

mod agent;
mod file_system;
mod frontend;
mod git;
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
    FrontendPathParam, FrontendQueryParam, FrontendResponseMode, GIT_IDENTITY_PATH, PROJECT_PATH,
    PROJECT_WORK_CONTEXT_OPEN_PATH, PROJECT_WORK_CONTEXT_RENEW_PATH, PROJECTS_PATH,
    SESSION_LOAD_PATH, SESSION_PATH, SESSION_PERMISSION_RESPONSE_PATH, SESSION_PROMPT_PATH,
    SESSION_STOP_PATH, SESSIONS_PATH, SKILL_PATH, SKILLS_PATH, TASK_PATH, TASKS_PATH,
    frontend_endpoints,
};
pub use git::{GetGitIdentityRequest, GitIdentityResponse};
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
    AgentCli, CreateSessionRequest, CreateSessionResponse, DeleteSessionRequest,
    DeleteSessionResponse, GetSessionRequest, GetSessionResponse, ListSessionsRequest,
    ListSessionsResponse, LoadSessionEvent, LoadSessionRequest, PromptSessionEvent,
    PromptSessionRequest, RespondToPermissionRequest, RespondToPermissionResponse, Session,
    SessionPermissionRequest, SessionStatus, StopSessionRequest, StopSessionResponse,
};
pub use skill::{
    CreateSkillRequest, CreateSkillResponse, DeleteSkillRequest, DeleteSkillResponse,
    GetSkillRequest, GetSkillResponse, ListSkillsRequest, ListSkillsResponse, Skill,
    UpdateSkillRequest, UpdateSkillResponse,
};
use std::path::Path;
pub use task::{
    CreateTaskRequest, CreateTaskResponse, DeleteTaskRequest, DeleteTaskResponse, GetTaskRequest,
    GetTaskResponse, ListTasksRequest, ListTasksResponse, Task, TaskStatus, TaskWorkspaceMode,
    UpdateTaskRequest, UpdateTaskResponse,
};
use ts_rs::{Config, ExportError};

/// Exports every contract DTO family into the shared TypeScript package for frontend consumers.
///
/// Each module owns the exhaustive list of its own TypeScript bindings, so adding a new contract
/// type only requires registering it next to its definition rather than in this aggregation point.
pub fn export_typescript_bindings_to(
    output_directory: impl AsRef<Path>,
) -> Result<(), ExportError> {
    let config = Config::new().with_out_dir(output_directory.as_ref());

    acp::export(&config)?;
    agent::export(&config)?;
    file_system::export(&config)?;
    git::export(&config)?;
    project::export(&config)?;
    project_work_context::export(&config)?;
    session::export(&config)?;
    skill::export(&config)?;
    task::export(&config)?;

    Ok(())
}
