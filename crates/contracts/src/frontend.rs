use serde::Serialize;

/// Enumerates the HTTP methods supported by the generated frontend SDK.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum FrontendHttpMethod {
    Get,
    Post,
    Put,
    Delete,
}

/// Selects whether an endpoint returns one value or an ordered event stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FrontendResponseMode {
    Unary,
    Stream,
}

/// Describes one request field that the transport must interpolate into the URL path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendPathParam {
    pub rust_field_name: &'static str,
    pub wire_name: &'static str,
}

/// Describes one optional request field serialized into an endpoint query string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendQueryParam {
    pub rust_field_name: &'static str,
    pub wire_name: &'static str,
}

/// Describes one frontend-facing HTTP operation exported from `ora-contracts`.
///
/// `namespace` and `member_name` place the operation on the generated client
/// (`client.project.create`); `operation_name` stays the flat wire-level identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendEndpoint {
    pub operation_name: &'static str,
    pub namespace: &'static str,
    pub member_name: &'static str,
    pub method: FrontendHttpMethod,
    pub path_template: &'static str,
    pub request_type: &'static str,
    pub response_type: &'static str,
    pub path_params: &'static [FrontendPathParam],
    pub has_json_body: bool,
}

impl FrontendEndpoint {
    /// Returns optional query parameters without forcing unrelated endpoints to repeat empty metadata.
    pub fn query_params(&self) -> &'static [FrontendQueryParam] {
        match self.operation_name {
            "listDirectory" => FILE_SYSTEM_DIRECTORY_QUERY_PARAMS,
            "getTaskDiff" => TASK_DIFF_QUERY_PARAMS,
            "listWorkflowRuns" => WORKFLOW_RUN_PROJECT_QUERY_PARAMS,
            _ => NO_QUERY_PARAMS,
        }
    }

    /// Returns the transport mode explicitly owned by the Rust endpoint catalog.
    pub fn response_mode(&self) -> FrontendResponseMode {
        match self.operation_name {
            "loadSession" | "promptSession" | "watchWorkspace" => FrontendResponseMode::Stream,
            _ => FrontendResponseMode::Unary,
        }
    }
}

pub const PROJECTS_PATH: &str = "/api/projects";
pub const PROJECT_PATH: &str = "/api/projects/{projectId}";
pub const PROJECT_BRANCHES_PATH: &str = "/api/projects/{projectId}/branches";
pub const PROJECT_WORK_CONTEXT_OPEN_PATH: &str = "/api/project-work-contexts/open";
pub const PROJECT_WORK_CONTEXT_RENEW_PATH: &str = "/api/project-work-contexts/renew";
pub const TASKS_PATH: &str = "/api/tasks";
pub const TASK_PATH: &str = "/api/tasks/{taskId}";
pub const TASK_WORKSPACE_PATH: &str = "/api/tasks/{taskId}/workspace";
pub const TASK_DIFF_PATH: &str = "/api/tasks/{taskId}/diff";
pub const TASK_COMMIT_PATH: &str = "/api/tasks/{taskId}/git/commit";
pub const TASK_PUSH_PATH: &str = "/api/tasks/{taskId}/git/push";
pub const TASK_DIFF_COMMENTS_PATH: &str = "/api/tasks/{taskId}/diff/comments";
pub const TASK_DIFF_COMMENT_REPLIES_PATH: &str =
    "/api/tasks/{taskId}/diff/comments/{commentId}/replies";
pub const TASK_DIFF_COMMENT_STATUS_PATH: &str =
    "/api/tasks/{taskId}/diff/comments/{commentId}/status";
pub const SESSIONS_PATH: &str = "/api/sessions";
pub const SESSION_PATH: &str = "/api/sessions/{sessionId}";
pub const SESSION_LOAD_PATH: &str = "/api/sessions/{sessionId}/load";
pub const SESSION_PROMPT_PATH: &str = "/api/sessions/{sessionId}/prompt";
pub const SESSION_PERMISSION_RESPONSE_PATH: &str = "/api/sessions/{sessionId}/permissions/respond";
pub const SESSION_STOP_PATH: &str = "/api/sessions/{sessionId}/stop";
pub const SESSION_SWITCH_AGENT_PATH: &str = "/api/sessions/{sessionId}/agent";
pub const SESSION_RESUME_HISTORY_PATH: &str = "/api/sessions/{sessionId}/history/resume";
pub const SESSION_WARM_PATH: &str = "/api/sessions/warm";
pub const SESSION_CONFIG_PATH: &str = "/api/sessions/{sessionId}/config";
pub const SESSION_ATTACH_PATH: &str = "/api/sessions/{sessionId}/attach";
pub const SKILLS_PATH: &str = "/api/skills";
pub const SKILL_PATH: &str = "/api/skills/{skillId}";
pub const SKILL_IMPORTS_PATH: &str = "/api/skill-imports";
pub const SKILL_IMPORT_PATH: &str = "/api/skill-imports/{sessionId}";
pub const SKILL_IMPORT_COMMIT_PATH: &str = "/api/skill-imports/{sessionId}/commit";
pub const AGENTS_PATH: &str = "/api/agents";
pub const AGENT_PATH: &str = "/api/agents/{agentId}";
pub const FILE_SYSTEM_DIRECTORY_PATH: &str = "/api/file-system/directory";
pub const WORKSPACE_DIRECTORY_PATH: &str = "/api/tasks/{taskId}/files/list";
pub const WORKSPACE_FILE_PATH: &str = "/api/tasks/{taskId}/files/read";
pub const WORKSPACE_SEARCH_PATH: &str = "/api/tasks/{taskId}/files/search";
pub const WORKSPACE_WATCH_PATH: &str = "/api/tasks/{taskId}/files/watch";
pub const GIT_IDENTITY_PATH: &str = "/api/git/identity";
pub const WORKFLOWS_PATH: &str = "/api/workflows";
pub const WORKFLOW_PATH: &str = "/api/workflows/{workflowId}";
pub const WORKFLOW_DRAFT_PATH: &str = "/api/workflows/{workflowId}/draft";
pub const WORKFLOW_PUBLISH_PATH: &str = "/api/workflows/{workflowId}/publish";
pub const WORKFLOW_ROLLBACK_PATH: &str = "/api/workflows/{workflowId}/rollback";
pub const WORKFLOW_ACTIVATE_PATH: &str = "/api/workflows/{workflowId}/activate";
pub const WORKFLOW_VERSIONS_PATH: &str = "/api/workflows/{workflowId}/versions";
pub const WORKFLOW_VERSION_PATH: &str = "/api/workflows/{workflowId}/versions/{version}";
pub const WORKFLOW_RUNS_PATH: &str = "/api/workflow-runs";
pub const WORKFLOW_RUN_PATH: &str = "/api/workflow-runs/{runId}";
pub const WORKFLOW_RUN_NODES_PATH: &str = "/api/workflow-runs/{runId}/nodes";

const PROJECT_ID_PATH_PARAM: FrontendPathParam = FrontendPathParam {
    rust_field_name: "project_id",
    wire_name: "projectId",
};
const TASK_ID_PATH_PARAM: FrontendPathParam = FrontendPathParam {
    rust_field_name: "task_id",
    wire_name: "taskId",
};
const COMMENT_ID_PATH_PARAM: FrontendPathParam = FrontendPathParam {
    rust_field_name: "comment_id",
    wire_name: "commentId",
};
const SESSION_ID_PATH_PARAM: FrontendPathParam = FrontendPathParam {
    rust_field_name: "session_id",
    wire_name: "sessionId",
};
const SKILL_ID_PATH_PARAM: FrontendPathParam = FrontendPathParam {
    rust_field_name: "skill_id",
    wire_name: "skillId",
};
const SKILL_IMPORT_SESSION_ID_PATH_PARAM: FrontendPathParam = FrontendPathParam {
    rust_field_name: "session_id",
    wire_name: "sessionId",
};
const AGENT_ID_PATH_PARAM: FrontendPathParam = FrontendPathParam {
    rust_field_name: "agent_id",
    wire_name: "agentId",
};
const WORKFLOW_ID_PATH_PARAM: FrontendPathParam = FrontendPathParam {
    rust_field_name: "workflow_id",
    wire_name: "workflowId",
};
const WORKFLOW_VERSION_PATH_PARAM: FrontendPathParam = FrontendPathParam {
    rust_field_name: "version",
    wire_name: "version",
};
const WORKFLOW_RUN_ID_PATH_PARAM: FrontendPathParam = FrontendPathParam {
    rust_field_name: "run_id",
    wire_name: "runId",
};
const WORKFLOW_RUN_PROJECT_QUERY_PARAM: FrontendQueryParam = FrontendQueryParam {
    rust_field_name: "project_id",
    wire_name: "projectId",
};
const FILE_SYSTEM_DIRECTORY_PATH_QUERY_PARAM: FrontendQueryParam = FrontendQueryParam {
    rust_field_name: "path",
    wire_name: "path",
};
const TASK_DIFF_SCOPE_QUERY_PARAM: FrontendQueryParam = FrontendQueryParam {
    rust_field_name: "scope",
    wire_name: "scope",
};

const PROJECT_NAMESPACE: &str = "project";
const PROJECT_WORK_CONTEXT_NAMESPACE: &str = "projectWorkContext";
const TASK_NAMESPACE: &str = "task";
const SESSION_NAMESPACE: &str = "session";
const SKILL_NAMESPACE: &str = "skill";
const SKILL_IMPORT_NAMESPACE: &str = "skillImport";
const AGENT_NAMESPACE: &str = "agent";
const FILE_SYSTEM_NAMESPACE: &str = "fileSystem";
const GIT_NAMESPACE: &str = "gitIdentity";
const WORKFLOW_NAMESPACE: &str = "workflow";
const WORKFLOW_RUN_NAMESPACE: &str = "workflowRun";

const PROJECT_PATH_PARAMS: &[FrontendPathParam] = &[PROJECT_ID_PATH_PARAM];
const TASK_PATH_PARAMS: &[FrontendPathParam] = &[TASK_ID_PATH_PARAM];
const TASK_COMMENT_PATH_PARAMS: &[FrontendPathParam] = &[TASK_ID_PATH_PARAM, COMMENT_ID_PATH_PARAM];
const SESSION_PATH_PARAMS: &[FrontendPathParam] = &[SESSION_ID_PATH_PARAM];
const SKILL_PATH_PARAMS: &[FrontendPathParam] = &[SKILL_ID_PATH_PARAM];
const SKILL_IMPORT_PATH_PARAMS: &[FrontendPathParam] = &[SKILL_IMPORT_SESSION_ID_PATH_PARAM];
const AGENT_PATH_PARAMS: &[FrontendPathParam] = &[AGENT_ID_PATH_PARAM];
const NO_PATH_PARAMS: &[FrontendPathParam] = &[];
const WORKFLOW_PATH_PARAMS: &[FrontendPathParam] = &[WORKFLOW_ID_PATH_PARAM];
const WORKFLOW_VERSION_PATH_PARAMS: &[FrontendPathParam] =
    &[WORKFLOW_ID_PATH_PARAM, WORKFLOW_VERSION_PATH_PARAM];
const WORKFLOW_RUN_PATH_PARAMS: &[FrontendPathParam] = &[WORKFLOW_RUN_ID_PATH_PARAM];
const WORKFLOW_RUN_PROJECT_QUERY_PARAMS: &[FrontendQueryParam] =
    &[WORKFLOW_RUN_PROJECT_QUERY_PARAM];
const FILE_SYSTEM_DIRECTORY_QUERY_PARAMS: &[FrontendQueryParam] =
    &[FILE_SYSTEM_DIRECTORY_PATH_QUERY_PARAM];
const TASK_DIFF_QUERY_PARAMS: &[FrontendQueryParam] = &[TASK_DIFF_SCOPE_QUERY_PARAM];
const NO_QUERY_PARAMS: &[FrontendQueryParam] = &[];

const FRONTEND_ENDPOINTS: &[FrontendEndpoint] = &[
    // =============================================================================
    // project
    // =============================================================================
    FrontendEndpoint {
        operation_name: "createProject",
        namespace: PROJECT_NAMESPACE,
        member_name: "create",
        method: FrontendHttpMethod::Post,
        path_template: PROJECTS_PATH,
        request_type: "CreateProjectRequest",
        response_type: "CreateProjectResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "getProject",
        namespace: PROJECT_NAMESPACE,
        member_name: "get",
        method: FrontendHttpMethod::Get,
        path_template: PROJECT_PATH,
        request_type: "GetProjectRequest",
        response_type: "GetProjectResponse",
        path_params: PROJECT_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "listProjects",
        namespace: PROJECT_NAMESPACE,
        member_name: "list",
        method: FrontendHttpMethod::Get,
        path_template: PROJECTS_PATH,
        request_type: "ListProjectsRequest",
        response_type: "ListProjectsResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "listProjectBranches",
        namespace: PROJECT_NAMESPACE,
        member_name: "listBranches",
        method: FrontendHttpMethod::Get,
        path_template: PROJECT_BRANCHES_PATH,
        request_type: "ListProjectBranchesRequest",
        response_type: "ListProjectBranchesResponse",
        path_params: PROJECT_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "updateProject",
        namespace: PROJECT_NAMESPACE,
        member_name: "update",
        method: FrontendHttpMethod::Put,
        path_template: PROJECT_PATH,
        request_type: "UpdateProjectRequest",
        response_type: "UpdateProjectResponse",
        path_params: PROJECT_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "deleteProject",
        namespace: PROJECT_NAMESPACE,
        member_name: "delete",
        method: FrontendHttpMethod::Delete,
        path_template: PROJECT_PATH,
        request_type: "DeleteProjectRequest",
        response_type: "DeleteProjectResponse",
        path_params: PROJECT_PATH_PARAMS,
        has_json_body: false,
    },
    // =============================================================================
    // projectWorkContext
    // =============================================================================
    FrontendEndpoint {
        operation_name: "openProjectWorkContext",
        namespace: PROJECT_WORK_CONTEXT_NAMESPACE,
        member_name: "open",
        method: FrontendHttpMethod::Post,
        path_template: PROJECT_WORK_CONTEXT_OPEN_PATH,
        request_type: "OpenProjectWorkContextRequest",
        response_type: "OpenProjectWorkContextResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "renewProjectWorkContext",
        namespace: PROJECT_WORK_CONTEXT_NAMESPACE,
        member_name: "renew",
        method: FrontendHttpMethod::Post,
        path_template: PROJECT_WORK_CONTEXT_RENEW_PATH,
        request_type: "RenewProjectWorkContextRequest",
        response_type: "RenewProjectWorkContextResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: true,
    },
    // =============================================================================
    // task
    // =============================================================================
    FrontendEndpoint {
        operation_name: "createTask",
        namespace: TASK_NAMESPACE,
        member_name: "create",
        method: FrontendHttpMethod::Post,
        path_template: TASKS_PATH,
        request_type: "CreateTaskRequest",
        response_type: "CreateTaskResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "getTask",
        namespace: TASK_NAMESPACE,
        member_name: "get",
        method: FrontendHttpMethod::Get,
        path_template: TASK_PATH,
        request_type: "GetTaskRequest",
        response_type: "GetTaskResponse",
        path_params: TASK_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "listTasks",
        namespace: TASK_NAMESPACE,
        member_name: "list",
        method: FrontendHttpMethod::Get,
        path_template: TASKS_PATH,
        request_type: "ListTasksRequest",
        response_type: "ListTasksResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "updateTask",
        namespace: TASK_NAMESPACE,
        member_name: "update",
        method: FrontendHttpMethod::Put,
        path_template: TASK_PATH,
        request_type: "UpdateTaskRequest",
        response_type: "UpdateTaskResponse",
        path_params: TASK_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "deleteTask",
        namespace: TASK_NAMESPACE,
        member_name: "delete",
        method: FrontendHttpMethod::Delete,
        path_template: TASK_PATH,
        request_type: "DeleteTaskRequest",
        response_type: "DeleteTaskResponse",
        path_params: TASK_PATH_PARAMS,
        has_json_body: false,
    },
    // =============================================================================
    // session
    // =============================================================================
    FrontendEndpoint {
        operation_name: "getTaskWorkspace",
        namespace: TASK_NAMESPACE,
        member_name: "getWorkspace",
        method: FrontendHttpMethod::Get,
        path_template: TASK_WORKSPACE_PATH,
        request_type: "GetTaskWorkspaceRequest",
        response_type: "GetTaskWorkspaceResponse",
        path_params: TASK_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "getTaskDiff",
        namespace: TASK_NAMESPACE,
        member_name: "getDiff",
        method: FrontendHttpMethod::Get,
        path_template: TASK_DIFF_PATH,
        request_type: "GetTaskDiffRequest",
        response_type: "GetTaskDiffResponse",
        path_params: TASK_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "commitTaskChanges",
        namespace: TASK_NAMESPACE,
        member_name: "commitChanges",
        method: FrontendHttpMethod::Post,
        path_template: TASK_COMMIT_PATH,
        request_type: "CommitTaskChangesRequest",
        response_type: "CommitTaskChangesResponse",
        path_params: TASK_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "pushTaskBranch",
        namespace: TASK_NAMESPACE,
        member_name: "pushBranch",
        method: FrontendHttpMethod::Post,
        path_template: TASK_PUSH_PATH,
        request_type: "PushTaskBranchRequest",
        response_type: "PushTaskBranchResponse",
        path_params: TASK_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "listTaskDiffComments",
        namespace: TASK_NAMESPACE,
        member_name: "listDiffComments",
        method: FrontendHttpMethod::Get,
        path_template: TASK_DIFF_COMMENTS_PATH,
        request_type: "ListTaskDiffCommentsRequest",
        response_type: "ListTaskDiffCommentsResponse",
        path_params: TASK_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "createTaskDiffComment",
        namespace: TASK_NAMESPACE,
        member_name: "createDiffComment",
        method: FrontendHttpMethod::Post,
        path_template: TASK_DIFF_COMMENTS_PATH,
        request_type: "CreateTaskDiffCommentRequest",
        response_type: "CreateTaskDiffCommentResponse",
        path_params: TASK_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "replyTaskDiffComment",
        namespace: TASK_NAMESPACE,
        member_name: "replyDiffComment",
        method: FrontendHttpMethod::Post,
        path_template: TASK_DIFF_COMMENT_REPLIES_PATH,
        request_type: "ReplyTaskDiffCommentRequest",
        response_type: "ReplyTaskDiffCommentResponse",
        path_params: TASK_COMMENT_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "setTaskDiffCommentStatus",
        namespace: TASK_NAMESPACE,
        member_name: "setDiffCommentStatus",
        method: FrontendHttpMethod::Put,
        path_template: TASK_DIFF_COMMENT_STATUS_PATH,
        request_type: "SetTaskDiffCommentStatusRequest",
        response_type: "SetTaskDiffCommentStatusResponse",
        path_params: TASK_COMMENT_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "warmSession",
        namespace: SESSION_NAMESPACE,
        member_name: "warm",
        method: FrontendHttpMethod::Post,
        path_template: SESSION_WARM_PATH,
        request_type: "WarmSessionRequest",
        response_type: "WarmSessionResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "setSessionConfig",
        namespace: SESSION_NAMESPACE,
        member_name: "setConfig",
        method: FrontendHttpMethod::Post,
        path_template: SESSION_CONFIG_PATH,
        request_type: "SetSessionConfigRequest",
        response_type: "SetSessionConfigResponse",
        path_params: SESSION_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "attachSession",
        namespace: SESSION_NAMESPACE,
        member_name: "attach",
        method: FrontendHttpMethod::Post,
        path_template: SESSION_ATTACH_PATH,
        request_type: "AttachSessionRequest",
        response_type: "AttachSessionResponse",
        path_params: SESSION_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "getSession",
        namespace: SESSION_NAMESPACE,
        member_name: "get",
        method: FrontendHttpMethod::Get,
        path_template: SESSION_PATH,
        request_type: "GetSessionRequest",
        response_type: "GetSessionResponse",
        path_params: SESSION_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "listSessions",
        namespace: SESSION_NAMESPACE,
        member_name: "list",
        method: FrontendHttpMethod::Get,
        path_template: SESSIONS_PATH,
        request_type: "ListSessionsRequest",
        response_type: "ListSessionsResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "loadSession",
        namespace: SESSION_NAMESPACE,
        member_name: "load",
        method: FrontendHttpMethod::Post,
        path_template: SESSION_LOAD_PATH,
        request_type: "LoadSessionRequest",
        response_type: "LoadSessionEvent",
        path_params: SESSION_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "promptSession",
        namespace: SESSION_NAMESPACE,
        member_name: "prompt",
        method: FrontendHttpMethod::Post,
        path_template: SESSION_PROMPT_PATH,
        request_type: "PromptSessionRequest",
        response_type: "PromptSessionEvent",
        path_params: SESSION_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "respondToSessionPermission",
        namespace: SESSION_NAMESPACE,
        member_name: "respondToPermission",
        method: FrontendHttpMethod::Post,
        path_template: SESSION_PERMISSION_RESPONSE_PATH,
        request_type: "RespondToPermissionRequest",
        response_type: "RespondToPermissionResponse",
        path_params: SESSION_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "stopSession",
        namespace: SESSION_NAMESPACE,
        member_name: "stop",
        method: FrontendHttpMethod::Post,
        path_template: SESSION_STOP_PATH,
        request_type: "StopSessionRequest",
        response_type: "StopSessionResponse",
        path_params: SESSION_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "switchSessionAgent",
        namespace: SESSION_NAMESPACE,
        member_name: "switchAgent",
        method: FrontendHttpMethod::Post,
        path_template: SESSION_SWITCH_AGENT_PATH,
        request_type: "SwitchSessionAgentRequest",
        response_type: "SwitchSessionAgentResponse",
        path_params: SESSION_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "resumeSessionHistory",
        namespace: SESSION_NAMESPACE,
        member_name: "resumeHistory",
        method: FrontendHttpMethod::Post,
        path_template: SESSION_RESUME_HISTORY_PATH,
        request_type: "ResumeSessionHistoryRequest",
        response_type: "ResumeSessionHistoryResponse",
        path_params: SESSION_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "deleteSession",
        namespace: SESSION_NAMESPACE,
        member_name: "delete",
        method: FrontendHttpMethod::Delete,
        path_template: SESSION_PATH,
        request_type: "DeleteSessionRequest",
        response_type: "DeleteSessionResponse",
        path_params: SESSION_PATH_PARAMS,
        has_json_body: false,
    },
    // =============================================================================
    // skill
    // =============================================================================
    FrontendEndpoint {
        operation_name: "createSkill",
        namespace: SKILL_NAMESPACE,
        member_name: "create",
        method: FrontendHttpMethod::Post,
        path_template: SKILLS_PATH,
        request_type: "CreateSkillRequest",
        response_type: "CreateSkillResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "getSkill",
        namespace: SKILL_NAMESPACE,
        member_name: "get",
        method: FrontendHttpMethod::Get,
        path_template: SKILL_PATH,
        request_type: "GetSkillRequest",
        response_type: "GetSkillResponse",
        path_params: SKILL_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "listSkills",
        namespace: SKILL_NAMESPACE,
        member_name: "list",
        method: FrontendHttpMethod::Get,
        path_template: SKILLS_PATH,
        request_type: "ListSkillsRequest",
        response_type: "ListSkillsResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "updateSkill",
        namespace: SKILL_NAMESPACE,
        member_name: "update",
        method: FrontendHttpMethod::Put,
        path_template: SKILL_PATH,
        request_type: "UpdateSkillRequest",
        response_type: "UpdateSkillResponse",
        path_params: SKILL_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "deleteSkill",
        namespace: SKILL_NAMESPACE,
        member_name: "delete",
        method: FrontendHttpMethod::Delete,
        path_template: SKILL_PATH,
        request_type: "DeleteSkillRequest",
        response_type: "DeleteSkillResponse",
        path_params: SKILL_PATH_PARAMS,
        has_json_body: false,
    },
    // =============================================================================
    // agent
    // =============================================================================
    FrontendEndpoint {
        operation_name: "prepareSkillImport",
        namespace: SKILL_IMPORT_NAMESPACE,
        member_name: "prepare",
        method: FrontendHttpMethod::Post,
        path_template: SKILL_IMPORTS_PATH,
        request_type: "PrepareSkillImportRequest",
        response_type: "PrepareSkillImportResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "getSkillImport",
        namespace: SKILL_IMPORT_NAMESPACE,
        member_name: "get",
        method: FrontendHttpMethod::Get,
        path_template: SKILL_IMPORT_PATH,
        request_type: "GetSkillImportSessionRequest",
        response_type: "GetSkillImportSessionResponse",
        path_params: SKILL_IMPORT_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "commitSkillImport",
        namespace: SKILL_IMPORT_NAMESPACE,
        member_name: "commit",
        method: FrontendHttpMethod::Post,
        path_template: SKILL_IMPORT_COMMIT_PATH,
        request_type: "CommitSkillImportRequest",
        response_type: "CommitSkillImportResponse",
        path_params: SKILL_IMPORT_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "cancelSkillImport",
        namespace: SKILL_IMPORT_NAMESPACE,
        member_name: "cancel",
        method: FrontendHttpMethod::Delete,
        path_template: SKILL_IMPORT_PATH,
        request_type: "CancelSkillImportRequest",
        response_type: "CancelSkillImportResponse",
        path_params: SKILL_IMPORT_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "createAgent",
        namespace: AGENT_NAMESPACE,
        member_name: "create",
        method: FrontendHttpMethod::Post,
        path_template: AGENTS_PATH,
        request_type: "CreateAgentRequest",
        response_type: "CreateAgentResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "getAgent",
        namespace: AGENT_NAMESPACE,
        member_name: "get",
        method: FrontendHttpMethod::Get,
        path_template: AGENT_PATH,
        request_type: "GetAgentRequest",
        response_type: "GetAgentResponse",
        path_params: AGENT_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "listAgents",
        namespace: AGENT_NAMESPACE,
        member_name: "list",
        method: FrontendHttpMethod::Get,
        path_template: AGENTS_PATH,
        request_type: "ListAgentsRequest",
        response_type: "ListAgentsResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "updateAgent",
        namespace: AGENT_NAMESPACE,
        member_name: "update",
        method: FrontendHttpMethod::Put,
        path_template: AGENT_PATH,
        request_type: "UpdateAgentRequest",
        response_type: "UpdateAgentResponse",
        path_params: AGENT_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "deleteAgent",
        namespace: AGENT_NAMESPACE,
        member_name: "delete",
        method: FrontendHttpMethod::Delete,
        path_template: AGENT_PATH,
        request_type: "DeleteAgentRequest",
        response_type: "DeleteAgentResponse",
        path_params: AGENT_PATH_PARAMS,
        has_json_body: false,
    },
    // =============================================================================
    // fileSystem
    // =============================================================================
    FrontendEndpoint {
        operation_name: "listDirectory",
        namespace: FILE_SYSTEM_NAMESPACE,
        member_name: "listDirectory",
        method: FrontendHttpMethod::Get,
        path_template: FILE_SYSTEM_DIRECTORY_PATH,
        request_type: "ListDirectoryRequest",
        response_type: "ListDirectoryResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "listWorkspaceDirectory",
        namespace: FILE_SYSTEM_NAMESPACE,
        member_name: "listWorkspaceDirectory",
        method: FrontendHttpMethod::Post,
        path_template: WORKSPACE_DIRECTORY_PATH,
        request_type: "ListWorkspaceDirectoryRequest",
        response_type: "ListWorkspaceDirectoryResponse",
        path_params: TASK_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "readWorkspaceFile",
        namespace: FILE_SYSTEM_NAMESPACE,
        member_name: "readWorkspaceFile",
        method: FrontendHttpMethod::Post,
        path_template: WORKSPACE_FILE_PATH,
        request_type: "ReadWorkspaceFileRequest",
        response_type: "ReadWorkspaceFileResponse",
        path_params: TASK_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "searchWorkspace",
        namespace: FILE_SYSTEM_NAMESPACE,
        member_name: "searchWorkspace",
        method: FrontendHttpMethod::Post,
        path_template: WORKSPACE_SEARCH_PATH,
        request_type: "SearchWorkspaceRequest",
        response_type: "SearchWorkspaceResponse",
        path_params: TASK_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "watchWorkspace",
        namespace: FILE_SYSTEM_NAMESPACE,
        member_name: "watchWorkspace",
        method: FrontendHttpMethod::Get,
        path_template: WORKSPACE_WATCH_PATH,
        request_type: "WatchWorkspaceRequest",
        response_type: "WorkspaceFileEventBatch",
        path_params: TASK_PATH_PARAMS,
        has_json_body: false,
    },
    // =============================================================================
    // gitIdentity
    // =============================================================================
    FrontendEndpoint {
        operation_name: "getGitIdentity",
        namespace: GIT_NAMESPACE,
        member_name: "get",
        method: FrontendHttpMethod::Get,
        path_template: GIT_IDENTITY_PATH,
        request_type: "GetGitIdentityRequest",
        response_type: "GitIdentityResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "createWorkflow",
        namespace: WORKFLOW_NAMESPACE,
        member_name: "create",
        method: FrontendHttpMethod::Post,
        path_template: WORKFLOWS_PATH,
        request_type: "CreateWorkflowRequest",
        response_type: "CreateWorkflowResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "getWorkflow",
        namespace: WORKFLOW_NAMESPACE,
        member_name: "get",
        method: FrontendHttpMethod::Get,
        path_template: WORKFLOW_PATH,
        request_type: "GetWorkflowRequest",
        response_type: "GetWorkflowResponse",
        path_params: WORKFLOW_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "listWorkflows",
        namespace: WORKFLOW_NAMESPACE,
        member_name: "list",
        method: FrontendHttpMethod::Get,
        path_template: WORKFLOWS_PATH,
        request_type: "ListWorkflowsRequest",
        response_type: "ListWorkflowsResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "updateWorkflow",
        namespace: WORKFLOW_NAMESPACE,
        member_name: "update",
        method: FrontendHttpMethod::Put,
        path_template: WORKFLOW_PATH,
        request_type: "UpdateWorkflowRequest",
        response_type: "UpdateWorkflowResponse",
        path_params: WORKFLOW_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "deleteWorkflow",
        namespace: WORKFLOW_NAMESPACE,
        member_name: "delete",
        method: FrontendHttpMethod::Delete,
        path_template: WORKFLOW_PATH,
        request_type: "DeleteWorkflowRequest",
        response_type: "DeleteWorkflowResponse",
        path_params: WORKFLOW_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "getDraft",
        namespace: WORKFLOW_NAMESPACE,
        member_name: "getDraft",
        method: FrontendHttpMethod::Get,
        path_template: WORKFLOW_DRAFT_PATH,
        request_type: "GetDraftRequest",
        response_type: "GetDraftResponse",
        path_params: WORKFLOW_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "updateDraft",
        namespace: WORKFLOW_NAMESPACE,
        member_name: "updateDraft",
        method: FrontendHttpMethod::Put,
        path_template: WORKFLOW_DRAFT_PATH,
        request_type: "UpdateDraftRequest",
        response_type: "UpdateDraftResponse",
        path_params: WORKFLOW_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "publishWorkflow",
        namespace: WORKFLOW_NAMESPACE,
        member_name: "publish",
        method: FrontendHttpMethod::Post,
        path_template: WORKFLOW_PUBLISH_PATH,
        request_type: "PublishWorkflowRequest",
        response_type: "PublishWorkflowResponse",
        path_params: WORKFLOW_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "rollbackWorkflow",
        namespace: WORKFLOW_NAMESPACE,
        member_name: "rollback",
        method: FrontendHttpMethod::Post,
        path_template: WORKFLOW_ROLLBACK_PATH,
        request_type: "RollbackWorkflowRequest",
        response_type: "RollbackWorkflowResponse",
        path_params: WORKFLOW_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "activateWorkflow",
        namespace: WORKFLOW_NAMESPACE,
        member_name: "activate",
        method: FrontendHttpMethod::Post,
        path_template: WORKFLOW_ACTIVATE_PATH,
        request_type: "ActivateWorkflowRequest",
        response_type: "ActivateWorkflowResponse",
        path_params: WORKFLOW_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "listVersions",
        namespace: WORKFLOW_NAMESPACE,
        member_name: "listVersions",
        method: FrontendHttpMethod::Get,
        path_template: WORKFLOW_VERSIONS_PATH,
        request_type: "ListVersionsRequest",
        response_type: "ListVersionsResponse",
        path_params: WORKFLOW_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "getVersion",
        namespace: WORKFLOW_NAMESPACE,
        member_name: "getVersion",
        method: FrontendHttpMethod::Get,
        path_template: WORKFLOW_VERSION_PATH,
        request_type: "GetVersionRequest",
        response_type: "GetVersionResponse",
        path_params: WORKFLOW_VERSION_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "deleteSnapshot",
        namespace: WORKFLOW_NAMESPACE,
        member_name: "deleteSnapshot",
        method: FrontendHttpMethod::Delete,
        path_template: WORKFLOW_VERSION_PATH,
        request_type: "DeleteSnapshotRequest",
        response_type: "DeleteSnapshotResponse",
        path_params: WORKFLOW_VERSION_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "createWorkflowRun",
        namespace: WORKFLOW_RUN_NAMESPACE,
        member_name: "create",
        method: FrontendHttpMethod::Post,
        path_template: WORKFLOW_RUNS_PATH,
        request_type: "CreateWorkflowRunRequest",
        response_type: "CreateWorkflowRunResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "getWorkflowRun",
        namespace: WORKFLOW_RUN_NAMESPACE,
        member_name: "get",
        method: FrontendHttpMethod::Get,
        path_template: WORKFLOW_RUN_PATH,
        request_type: "GetWorkflowRunRequest",
        response_type: "GetWorkflowRunResponse",
        path_params: WORKFLOW_RUN_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "listWorkflowRuns",
        namespace: WORKFLOW_RUN_NAMESPACE,
        member_name: "list",
        method: FrontendHttpMethod::Get,
        path_template: WORKFLOW_RUNS_PATH,
        request_type: "ListWorkflowRunsRequest",
        response_type: "ListWorkflowRunsResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "listWorkflowNodeRuns",
        namespace: WORKFLOW_RUN_NAMESPACE,
        member_name: "listNodeRuns",
        method: FrontendHttpMethod::Get,
        path_template: WORKFLOW_RUN_NODES_PATH,
        request_type: "ListWorkflowNodeRunsRequest",
        response_type: "ListWorkflowNodeRunsResponse",
        path_params: WORKFLOW_RUN_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "deleteWorkflowRun",
        namespace: WORKFLOW_RUN_NAMESPACE,
        member_name: "delete",
        method: FrontendHttpMethod::Delete,
        path_template: WORKFLOW_RUN_PATH,
        request_type: "DeleteWorkflowRunRequest",
        response_type: "DeleteWorkflowRunResponse",
        path_params: WORKFLOW_RUN_PATH_PARAMS,
        has_json_body: false,
    },
];

/// Returns the Rust-owned endpoint metadata exported to the generated frontend SDK.
pub fn frontend_endpoints() -> &'static [FrontendEndpoint] {
    FRONTEND_ENDPOINTS
}

#[cfg(test)]
mod tests {
    use super::{
        FrontendEndpoint, FrontendHttpMethod, FrontendPathParam, FrontendQueryParam, TASK_PATH,
        frontend_endpoints,
    };
    use pretty_assertions::assert_eq;
    use std::collections::BTreeSet;

    /// Verifies directory listing paths are encoded as optional GET query parameters.
    #[test]
    fn exposes_directory_query_parameter_metadata() {
        let list_directory = frontend_endpoints()
            .iter()
            .find(|endpoint| endpoint.operation_name == "listDirectory")
            .unwrap_or_else(|| panic!("missing listDirectory endpoint"));

        assert_eq!(
            list_directory.query_params(),
            &[FrontendQueryParam {
                rust_field_name: "path",
                wire_name: "path",
            }]
        );
    }

    /// Verifies update operations describe the path/body split needed by the generated client.
    #[test]
    fn preserves_path_params_for_update_routes() {
        let update_task = frontend_endpoints()
            .iter()
            .find(|endpoint| endpoint.operation_name == "updateTask")
            .copied()
            .unwrap_or_else(|| panic!("missing updateTask endpoint"));

        assert_eq!(
            update_task,
            FrontendEndpoint {
                operation_name: "updateTask",
                namespace: "task",
                member_name: "update",
                method: FrontendHttpMethod::Put,
                path_template: TASK_PATH,
                request_type: "UpdateTaskRequest",
                response_type: "UpdateTaskResponse",
                path_params: &[FrontendPathParam {
                    rust_field_name: "task_id",
                    wire_name: "taskId",
                }],
                has_json_body: true,
            }
        );
    }

    /// Verifies every namespace member is unique so no operation is shadowed on the generated client.
    #[test]
    fn exports_unique_namespace_members() {
        let mut seen_members = BTreeSet::new();

        for endpoint in frontend_endpoints() {
            assert_eq!(
                seen_members.insert((endpoint.namespace, endpoint.member_name)),
                true,
                "duplicate client member {}.{}",
                endpoint.namespace,
                endpoint.member_name
            );
        }
    }

    /// Verifies the exported endpoint manifest omits backend-owned worktree operations.
    #[test]
    fn omits_worktree_endpoints_from_frontend_manifest() {
        assert_eq!(
            frontend_endpoints()
                .iter()
                .all(|endpoint| !endpoint.operation_name.contains("Worktree")),
            true
        );
    }

    /// Verifies catalogs publish separate collection and identifier resource routes.
    #[test]
    fn exports_skill_and_agent_crud_endpoints() {
        assert!(
            frontend_endpoints()
                .iter()
                .any(|endpoint| endpoint.operation_name == "updateSkill"
                    && endpoint.path_template == "/api/skills/{skillId}")
        );
        assert!(
            frontend_endpoints()
                .iter()
                .any(|endpoint| endpoint.operation_name == "updateAgent"
                    && endpoint.path_template == "/api/agents/{agentId}")
        );
    }
}
