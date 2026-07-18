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
            _ => NO_QUERY_PARAMS,
        }
    }
}

pub const PROJECTS_PATH: &str = "/api/projects";
pub const PROJECT_PATH: &str = "/api/projects/{projectId}";
pub const PROJECT_WORK_CONTEXT_OPEN_PATH: &str = "/api/project-work-contexts/open";
pub const PROJECT_WORK_CONTEXT_RENEW_PATH: &str = "/api/project-work-contexts/renew";
pub const TASKS_PATH: &str = "/api/tasks";
pub const TASK_PATH: &str = "/api/tasks/{taskId}";
pub const SESSIONS_PATH: &str = "/api/sessions";
pub const SESSION_PATH: &str = "/api/sessions/{sessionId}";
pub const SKILLS_PATH: &str = "/api/skills";
pub const SKILL_PATH: &str = "/api/skills/{skillId}";
pub const AGENTS_PATH: &str = "/api/agents";
pub const AGENT_PATH: &str = "/api/agents/{agentId}";
pub const FILE_SYSTEM_DIRECTORY_PATH: &str = "/api/file-system/directory";

const PROJECT_ID_PATH_PARAM: FrontendPathParam = FrontendPathParam {
    rust_field_name: "project_id",
    wire_name: "projectId",
};
const TASK_ID_PATH_PARAM: FrontendPathParam = FrontendPathParam {
    rust_field_name: "task_id",
    wire_name: "taskId",
};
const SESSION_ID_PATH_PARAM: FrontendPathParam = FrontendPathParam {
    rust_field_name: "session_id",
    wire_name: "sessionId",
};
const SKILL_ID_PATH_PARAM: FrontendPathParam = FrontendPathParam {
    rust_field_name: "skill_id",
    wire_name: "skillId",
};
const AGENT_ID_PATH_PARAM: FrontendPathParam = FrontendPathParam {
    rust_field_name: "agent_id",
    wire_name: "agentId",
};
const FILE_SYSTEM_DIRECTORY_PATH_QUERY_PARAM: FrontendQueryParam = FrontendQueryParam {
    rust_field_name: "path",
    wire_name: "path",
};

const PROJECT_NAMESPACE: &str = "project";
const PROJECT_WORK_CONTEXT_NAMESPACE: &str = "projectWorkContext";
const TASK_NAMESPACE: &str = "task";
const SESSION_NAMESPACE: &str = "session";
const SKILL_NAMESPACE: &str = "skill";
const AGENT_NAMESPACE: &str = "agent";
const FILE_SYSTEM_NAMESPACE: &str = "fileSystem";

const PROJECT_PATH_PARAMS: &[FrontendPathParam] = &[PROJECT_ID_PATH_PARAM];
const TASK_PATH_PARAMS: &[FrontendPathParam] = &[TASK_ID_PATH_PARAM];
const SESSION_PATH_PARAMS: &[FrontendPathParam] = &[SESSION_ID_PATH_PARAM];
const SKILL_PATH_PARAMS: &[FrontendPathParam] = &[SKILL_ID_PATH_PARAM];
const AGENT_PATH_PARAMS: &[FrontendPathParam] = &[AGENT_ID_PATH_PARAM];
const NO_PATH_PARAMS: &[FrontendPathParam] = &[];
const FILE_SYSTEM_DIRECTORY_QUERY_PARAMS: &[FrontendQueryParam] =
    &[FILE_SYSTEM_DIRECTORY_PATH_QUERY_PARAM];
const NO_QUERY_PARAMS: &[FrontendQueryParam] = &[];

const FRONTEND_ENDPOINTS: &[FrontendEndpoint] = &[
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
    FrontendEndpoint {
        operation_name: "createSession",
        namespace: SESSION_NAMESPACE,
        member_name: "create",
        method: FrontendHttpMethod::Post,
        path_template: SESSIONS_PATH,
        request_type: "CreateSessionRequest",
        response_type: "CreateSessionResponse",
        path_params: NO_PATH_PARAMS,
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
        operation_name: "updateSession",
        namespace: SESSION_NAMESPACE,
        member_name: "update",
        method: FrontendHttpMethod::Put,
        path_template: SESSION_PATH,
        request_type: "UpdateSessionRequest",
        response_type: "UpdateSessionResponse",
        path_params: SESSION_PATH_PARAMS,
        has_json_body: true,
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
];

/// Returns the Rust-owned endpoint metadata exported to the generated frontend SDK.
pub fn frontend_endpoints() -> &'static [FrontendEndpoint] {
    FRONTEND_ENDPOINTS
}

#[cfg(test)]
mod tests {
    use super::{
        AGENT_PATH, AGENTS_PATH, FILE_SYSTEM_DIRECTORY_PATH, FrontendEndpoint, FrontendHttpMethod,
        FrontendPathParam, FrontendQueryParam, PROJECT_PATH, PROJECT_WORK_CONTEXT_OPEN_PATH,
        PROJECT_WORK_CONTEXT_RENEW_PATH, PROJECTS_PATH, SESSION_PATH, SESSIONS_PATH, SKILL_PATH,
        SKILLS_PATH, TASK_PATH, TASKS_PATH, frontend_endpoints,
    };
    use pretty_assertions::assert_eq;
    use std::collections::BTreeSet;

    /// Verifies the exported endpoint manifest matches the current CRUD route surface.
    #[test]
    fn exports_frontend_endpoint_manifest() {
        assert_eq!(
            frontend_endpoints(),
            &[
                FrontendEndpoint {
                    operation_name: "createProject",
                    namespace: "project",
                    member_name: "create",
                    method: FrontendHttpMethod::Post,
                    path_template: PROJECTS_PATH,
                    request_type: "CreateProjectRequest",
                    response_type: "CreateProjectResponse",
                    path_params: &[],
                    has_json_body: true,
                },
                FrontendEndpoint {
                    operation_name: "getProject",
                    namespace: "project",
                    member_name: "get",
                    method: FrontendHttpMethod::Get,
                    path_template: PROJECT_PATH,
                    request_type: "GetProjectRequest",
                    response_type: "GetProjectResponse",
                    path_params: &[FrontendPathParam {
                        rust_field_name: "project_id",
                        wire_name: "projectId",
                    }],
                    has_json_body: false,
                },
                FrontendEndpoint {
                    operation_name: "listProjects",
                    namespace: "project",
                    member_name: "list",
                    method: FrontendHttpMethod::Get,
                    path_template: PROJECTS_PATH,
                    request_type: "ListProjectsRequest",
                    response_type: "ListProjectsResponse",
                    path_params: &[],
                    has_json_body: false,
                },
                FrontendEndpoint {
                    operation_name: "updateProject",
                    namespace: "project",
                    member_name: "update",
                    method: FrontendHttpMethod::Put,
                    path_template: PROJECT_PATH,
                    request_type: "UpdateProjectRequest",
                    response_type: "UpdateProjectResponse",
                    path_params: &[FrontendPathParam {
                        rust_field_name: "project_id",
                        wire_name: "projectId",
                    }],
                    has_json_body: true,
                },
                FrontendEndpoint {
                    operation_name: "deleteProject",
                    namespace: "project",
                    member_name: "delete",
                    method: FrontendHttpMethod::Delete,
                    path_template: PROJECT_PATH,
                    request_type: "DeleteProjectRequest",
                    response_type: "DeleteProjectResponse",
                    path_params: &[FrontendPathParam {
                        rust_field_name: "project_id",
                        wire_name: "projectId",
                    }],
                    has_json_body: false,
                },
                FrontendEndpoint {
                    operation_name: "openProjectWorkContext",
                    namespace: "projectWorkContext",
                    member_name: "open",
                    method: FrontendHttpMethod::Post,
                    path_template: PROJECT_WORK_CONTEXT_OPEN_PATH,
                    request_type: "OpenProjectWorkContextRequest",
                    response_type: "OpenProjectWorkContextResponse",
                    path_params: &[],
                    has_json_body: true,
                },
                FrontendEndpoint {
                    operation_name: "renewProjectWorkContext",
                    namespace: "projectWorkContext",
                    member_name: "renew",
                    method: FrontendHttpMethod::Post,
                    path_template: PROJECT_WORK_CONTEXT_RENEW_PATH,
                    request_type: "RenewProjectWorkContextRequest",
                    response_type: "RenewProjectWorkContextResponse",
                    path_params: &[],
                    has_json_body: true,
                },
                FrontendEndpoint {
                    operation_name: "createTask",
                    namespace: "task",
                    member_name: "create",
                    method: FrontendHttpMethod::Post,
                    path_template: TASKS_PATH,
                    request_type: "CreateTaskRequest",
                    response_type: "CreateTaskResponse",
                    path_params: &[],
                    has_json_body: true,
                },
                FrontendEndpoint {
                    operation_name: "getTask",
                    namespace: "task",
                    member_name: "get",
                    method: FrontendHttpMethod::Get,
                    path_template: TASK_PATH,
                    request_type: "GetTaskRequest",
                    response_type: "GetTaskResponse",
                    path_params: &[FrontendPathParam {
                        rust_field_name: "task_id",
                        wire_name: "taskId",
                    }],
                    has_json_body: false,
                },
                FrontendEndpoint {
                    operation_name: "listTasks",
                    namespace: "task",
                    member_name: "list",
                    method: FrontendHttpMethod::Get,
                    path_template: TASKS_PATH,
                    request_type: "ListTasksRequest",
                    response_type: "ListTasksResponse",
                    path_params: &[],
                    has_json_body: false,
                },
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
                },
                FrontendEndpoint {
                    operation_name: "deleteTask",
                    namespace: "task",
                    member_name: "delete",
                    method: FrontendHttpMethod::Delete,
                    path_template: TASK_PATH,
                    request_type: "DeleteTaskRequest",
                    response_type: "DeleteTaskResponse",
                    path_params: &[FrontendPathParam {
                        rust_field_name: "task_id",
                        wire_name: "taskId",
                    }],
                    has_json_body: false,
                },
                FrontendEndpoint {
                    operation_name: "createSession",
                    namespace: "session",
                    member_name: "create",
                    method: FrontendHttpMethod::Post,
                    path_template: SESSIONS_PATH,
                    request_type: "CreateSessionRequest",
                    response_type: "CreateSessionResponse",
                    path_params: &[],
                    has_json_body: true,
                },
                FrontendEndpoint {
                    operation_name: "getSession",
                    namespace: "session",
                    member_name: "get",
                    method: FrontendHttpMethod::Get,
                    path_template: SESSION_PATH,
                    request_type: "GetSessionRequest",
                    response_type: "GetSessionResponse",
                    path_params: &[FrontendPathParam {
                        rust_field_name: "session_id",
                        wire_name: "sessionId",
                    }],
                    has_json_body: false,
                },
                FrontendEndpoint {
                    operation_name: "listSessions",
                    namespace: "session",
                    member_name: "list",
                    method: FrontendHttpMethod::Get,
                    path_template: SESSIONS_PATH,
                    request_type: "ListSessionsRequest",
                    response_type: "ListSessionsResponse",
                    path_params: &[],
                    has_json_body: false,
                },
                FrontendEndpoint {
                    operation_name: "updateSession",
                    namespace: "session",
                    member_name: "update",
                    method: FrontendHttpMethod::Put,
                    path_template: SESSION_PATH,
                    request_type: "UpdateSessionRequest",
                    response_type: "UpdateSessionResponse",
                    path_params: &[FrontendPathParam {
                        rust_field_name: "session_id",
                        wire_name: "sessionId",
                    }],
                    has_json_body: true,
                },
                FrontendEndpoint {
                    operation_name: "deleteSession",
                    namespace: "session",
                    member_name: "delete",
                    method: FrontendHttpMethod::Delete,
                    path_template: SESSION_PATH,
                    request_type: "DeleteSessionRequest",
                    response_type: "DeleteSessionResponse",
                    path_params: &[FrontendPathParam {
                        rust_field_name: "session_id",
                        wire_name: "sessionId",
                    }],
                    has_json_body: false,
                },
                FrontendEndpoint {
                    operation_name: "createSkill",
                    namespace: "skill",
                    member_name: "create",
                    method: FrontendHttpMethod::Post,
                    path_template: SKILLS_PATH,
                    request_type: "CreateSkillRequest",
                    response_type: "CreateSkillResponse",
                    path_params: &[],
                    has_json_body: true,
                },
                FrontendEndpoint {
                    operation_name: "getSkill",
                    namespace: "skill",
                    member_name: "get",
                    method: FrontendHttpMethod::Get,
                    path_template: SKILL_PATH,
                    request_type: "GetSkillRequest",
                    response_type: "GetSkillResponse",
                    path_params: &[FrontendPathParam {
                        rust_field_name: "skill_id",
                        wire_name: "skillId"
                    }],
                    has_json_body: false,
                },
                FrontendEndpoint {
                    operation_name: "listSkills",
                    namespace: "skill",
                    member_name: "list",
                    method: FrontendHttpMethod::Get,
                    path_template: SKILLS_PATH,
                    request_type: "ListSkillsRequest",
                    response_type: "ListSkillsResponse",
                    path_params: &[],
                    has_json_body: false,
                },
                FrontendEndpoint {
                    operation_name: "updateSkill",
                    namespace: "skill",
                    member_name: "update",
                    method: FrontendHttpMethod::Put,
                    path_template: SKILL_PATH,
                    request_type: "UpdateSkillRequest",
                    response_type: "UpdateSkillResponse",
                    path_params: &[FrontendPathParam {
                        rust_field_name: "skill_id",
                        wire_name: "skillId"
                    }],
                    has_json_body: true,
                },
                FrontendEndpoint {
                    operation_name: "deleteSkill",
                    namespace: "skill",
                    member_name: "delete",
                    method: FrontendHttpMethod::Delete,
                    path_template: SKILL_PATH,
                    request_type: "DeleteSkillRequest",
                    response_type: "DeleteSkillResponse",
                    path_params: &[FrontendPathParam {
                        rust_field_name: "skill_id",
                        wire_name: "skillId"
                    }],
                    has_json_body: false,
                },
                FrontendEndpoint {
                    operation_name: "createAgent",
                    namespace: "agent",
                    member_name: "create",
                    method: FrontendHttpMethod::Post,
                    path_template: AGENTS_PATH,
                    request_type: "CreateAgentRequest",
                    response_type: "CreateAgentResponse",
                    path_params: &[],
                    has_json_body: true,
                },
                FrontendEndpoint {
                    operation_name: "getAgent",
                    namespace: "agent",
                    member_name: "get",
                    method: FrontendHttpMethod::Get,
                    path_template: AGENT_PATH,
                    request_type: "GetAgentRequest",
                    response_type: "GetAgentResponse",
                    path_params: &[FrontendPathParam {
                        rust_field_name: "agent_id",
                        wire_name: "agentId"
                    }],
                    has_json_body: false,
                },
                FrontendEndpoint {
                    operation_name: "listAgents",
                    namespace: "agent",
                    member_name: "list",
                    method: FrontendHttpMethod::Get,
                    path_template: AGENTS_PATH,
                    request_type: "ListAgentsRequest",
                    response_type: "ListAgentsResponse",
                    path_params: &[],
                    has_json_body: false,
                },
                FrontendEndpoint {
                    operation_name: "updateAgent",
                    namespace: "agent",
                    member_name: "update",
                    method: FrontendHttpMethod::Put,
                    path_template: AGENT_PATH,
                    request_type: "UpdateAgentRequest",
                    response_type: "UpdateAgentResponse",
                    path_params: &[FrontendPathParam {
                        rust_field_name: "agent_id",
                        wire_name: "agentId"
                    }],
                    has_json_body: true,
                },
                FrontendEndpoint {
                    operation_name: "deleteAgent",
                    namespace: "agent",
                    member_name: "delete",
                    method: FrontendHttpMethod::Delete,
                    path_template: AGENT_PATH,
                    request_type: "DeleteAgentRequest",
                    response_type: "DeleteAgentResponse",
                    path_params: &[FrontendPathParam {
                        rust_field_name: "agent_id",
                        wire_name: "agentId"
                    }],
                    has_json_body: false,
                },
                FrontendEndpoint {
                    operation_name: "listDirectory",
                    namespace: "fileSystem",
                    member_name: "listDirectory",
                    method: FrontendHttpMethod::Get,
                    path_template: FILE_SYSTEM_DIRECTORY_PATH,
                    request_type: "ListDirectoryRequest",
                    response_type: "ListDirectoryResponse",
                    path_params: &[],
                    has_json_body: false,
                },
            ]
        );
    }

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
