pub mod acp;

mod agent;
mod error;
mod file_system;
mod frontend;
mod git;
mod project;
mod project_work_context;
mod session;
mod skill;
mod skill_import;
mod task;
mod task_diff;
mod workflow;
mod workflow_run;

pub use agent::{
    Agent, CreateAgentRequest, CreateAgentResponse, DeleteAgentRequest, DeleteAgentResponse,
    GetAgentRequest, GetAgentResponse, ListAgentsRequest, ListAgentsResponse, UpdateAgentRequest,
    UpdateAgentResponse,
};
pub use error::{
    ContractError, EmptyErrorParams, OpenLocationFailedParams, OpenLocationTarget, PublicError,
    RequestId, SkillFolderConflictParams, SkillUploadTooLargeParams, SkillUploadTooManyFilesParams,
    TaskBaseBranchNotFoundParams,
};
pub use file_system::{
    FileSystemBreadcrumb, FileSystemEntry, FileSystemEntryKind, ListDirectoryRequest,
    ListDirectoryResponse, ListWorkspaceDirectoryRequest, ListWorkspaceDirectoryResponse,
    ReadWorkspaceFileRequest, ReadWorkspaceFileResponse, SearchWorkspaceRequest,
    SearchWorkspaceResponse, WatchWorkspaceRequest, WorkspaceEntry, WorkspaceEntryKind,
    WorkspaceFileChange, WorkspaceFileEventBatch, WorkspaceSearchKind, WorkspaceSearchResult,
};
pub use frontend::{
    AGENT_PATH, AGENTS_PATH, FILE_SYSTEM_DIRECTORY_PATH, FrontendEndpoint, FrontendHttpMethod,
    FrontendPathParam, FrontendQueryParam, FrontendResponseMode, GIT_IDENTITY_PATH,
    PROJECT_BRANCHES_PATH, PROJECT_PATH, PROJECT_WORK_CONTEXT_OPEN_PATH,
    PROJECT_WORK_CONTEXT_RENEW_PATH, PROJECTS_PATH, SESSION_ATTACH_PATH, SESSION_CONFIG_PATH,
    SESSION_LOAD_PATH, SESSION_PATH, SESSION_PERMISSION_RESPONSE_PATH, SESSION_PROMPT_PATH,
    SESSION_RESUME_HISTORY_PATH, SESSION_STOP_PATH, SESSION_SWITCH_AGENT_PATH, SESSION_WARM_PATH,
    SESSIONS_PATH, SKILL_IMPORT_COMMIT_PATH, SKILL_IMPORT_PATH, SKILL_IMPORTS_PATH, SKILL_PATH,
    SKILLS_PATH, TASK_COMMIT_PATH, TASK_DIFF_COMMENT_REPLIES_PATH, TASK_DIFF_COMMENT_STATUS_PATH,
    TASK_DIFF_COMMENTS_PATH, TASK_DIFF_PATH, TASK_PATH, TASK_PUSH_PATH, TASK_WORKSPACE_PATH,
    TASKS_PATH, WORKFLOW_ACTIVATE_PATH, WORKFLOW_DRAFT_PATH, WORKFLOW_PATH, WORKFLOW_PUBLISH_PATH,
    WORKFLOW_ROLLBACK_PATH, WORKFLOW_VERSION_PATH, WORKFLOW_VERSIONS_PATH, WORKFLOWS_PATH,
    WORKSPACE_DIRECTORY_PATH, WORKSPACE_FILE_PATH, WORKSPACE_SEARCH_PATH, WORKSPACE_WATCH_PATH,
    frontend_endpoints,
};
pub use git::{GetGitIdentityRequest, GitIdentityResponse};
pub use project::{
    CreateProjectRequest, CreateProjectResponse, DeleteProjectRequest, DeleteProjectResponse,
    GetProjectRequest, GetProjectResponse, ListProjectBranchesRequest, ListProjectBranchesResponse,
    ListProjectsRequest, ListProjectsResponse, Project, ProjectBranch, UpdateProjectRequest,
    UpdateProjectResponse,
};
pub use project_work_context::{
    OpenProjectWorkContextRequest, OpenProjectWorkContextResponse, ProjectWorkContext,
    ProjectWorkContextSurface, RenewProjectWorkContextRequest, RenewProjectWorkContextResponse,
};
pub use session::{
    AgentCli, AttachSessionRequest, AttachSessionResponse, DeleteSessionRequest,
    DeleteSessionResponse, GetSessionRequest, GetSessionResponse, ListSessionsRequest,
    ListSessionsResponse, LoadSessionEvent, LoadSessionRequest, PromptSessionEvent,
    PromptSessionRequest, RespondToPermissionRequest, RespondToPermissionResponse,
    ResumeSessionHistoryRequest, ResumeSessionHistoryResponse, Session, SessionHistoryState,
    SessionPermissionRequest, SessionStatus, SetSessionConfigRequest, SetSessionConfigResponse,
    StopSessionRequest, StopSessionResponse, SwitchSessionAgentRequest, SwitchSessionAgentResponse,
    WarmSessionRequest, WarmSessionResponse, WarmSessionTarget,
};
pub use skill::{
    CreateSkillRequest, CreateSkillResponse, DeleteSkillRequest, DeleteSkillResponse,
    GetSkillRequest, GetSkillResponse, ListSkillsRequest, ListSkillsResponse, Skill,
    UpdateSkillRequest, UpdateSkillResponse,
};
pub use skill_import::{
    CancelSkillImportRequest, CancelSkillImportResponse, CommitSkillImportRequest,
    CommitSkillImportResponse, GetSkillImportSessionRequest, GetSkillImportSessionResponse,
    PrepareSkillImportRequest, PrepareSkillImportResponse, SkillConflictInfo, SkillImportCandidate,
    SkillImportCandidateStatus, SkillImportConflictDecision, SkillImportDecision,
    SkillImportProgress, SkillImportResult, SkillImportResultStatus, SkillImportSession,
    SkillImportSessionStatus, SkillImportSource,
};
use std::path::Path;
pub use task::{
    CreateTaskRequest, CreateTaskResponse, DeleteTaskRequest, DeleteTaskResponse, GetTaskRequest,
    GetTaskResponse, GetTaskWorkspaceRequest, GetTaskWorkspaceResponse, ListTasksRequest,
    ListTasksResponse, Task, TaskStatus, TaskType, TaskWorkspace, TaskWorkspaceMode,
    UpdateTaskRequest, UpdateTaskResponse,
};
pub use task_diff::{
    CommitTaskChangesRequest, CommitTaskChangesResponse, CreateTaskDiffCommentRequest,
    CreateTaskDiffCommentResponse, GetTaskDiffRequest, GetTaskDiffResponse,
    ListTaskDiffCommentsRequest, ListTaskDiffCommentsResponse, PushTaskBranchRequest,
    PushTaskBranchResponse, ReplyTaskDiffCommentRequest, ReplyTaskDiffCommentResponse,
    SetTaskDiffCommentStatusRequest, SetTaskDiffCommentStatusResponse, TaskDiffComment,
    TaskDiffCommentAnchor, TaskDiffCommentKind, TaskDiffScope, TaskDiffSide, TaskDiffThreadStatus,
};
use ts_rs::{Config, ExportError};
pub use workflow::{
    ActivateWorkflowRequest, ActivateWorkflowResponse, CreateWorkflowRequest,
    CreateWorkflowResponse, DeleteSnapshotRequest, DeleteSnapshotResponse, DeleteWorkflowRequest,
    DeleteWorkflowResponse, GetDraftRequest, GetDraftResponse, GetVersionRequest,
    GetVersionResponse, GetWorkflowRequest, GetWorkflowResponse, ListVersionsRequest,
    ListVersionsResponse, ListWorkflowsRequest, ListWorkflowsResponse, PublishWorkflowRequest,
    PublishWorkflowResponse, RollbackWorkflowRequest, RollbackWorkflowResponse, UpdateDraftRequest,
    UpdateDraftResponse, UpdateWorkflowRequest, UpdateWorkflowResponse, Workflow, WorkflowSnapshot,
    WorkflowSummary, WorkflowVersion,
};
pub use workflow_run::{
    CreateWorkflowRunRequest, CreateWorkflowRunResponse, DeleteWorkflowRunRequest,
    DeleteWorkflowRunResponse, GetWorkflowRunRequest, GetWorkflowRunResponse,
    ListWorkflowNodeRunsRequest, ListWorkflowNodeRunsResponse, ListWorkflowRunsRequest,
    ListWorkflowRunsResponse, WorkflowNodeRun, WorkflowNodeStatus, WorkflowRun, WorkflowRunStatus,
    WorkflowRunSummary,
};

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
    error::export(&config)?;
    file_system::export(&config)?;
    git::export(&config)?;
    project::export(&config)?;
    project_work_context::export(&config)?;
    session::export(&config)?;
    skill::export(&config)?;
    skill_import::export(&config)?;
    task::export(&config)?;
    task_diff::export(&config)?;
    workflow::export(&config)?;
    workflow_run::export(&config)?;

    Ok(())
}
