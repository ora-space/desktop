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

/// Describes one frontend-facing HTTP operation exported from `ora-contracts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendEndpoint {
    pub operation_name: &'static str,
    pub method: FrontendHttpMethod,
    pub path_template: &'static str,
    pub request_type: &'static str,
    pub response_type: &'static str,
    pub path_params: &'static [FrontendPathParam],
    pub has_json_body: bool,
}

pub const PROJECTS_PATH: &str = "/api/projects";
pub const PROJECT_PATH: &str = "/api/projects/{projectId}";
pub const PROJECT_WORK_CONTEXT_OPEN_PATH: &str = "/api/project-work-contexts/open";
pub const PROJECT_WORK_CONTEXT_RENEW_PATH: &str = "/api/project-work-contexts/renew";
pub const TASKS_PATH: &str = "/api/tasks";
pub const TASK_PATH: &str = "/api/tasks/{taskId}";
pub const SESSIONS_PATH: &str = "/api/sessions";
pub const SESSION_PATH: &str = "/api/sessions/{sessionId}";

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

const PROJECT_PATH_PARAMS: &[FrontendPathParam] = &[PROJECT_ID_PATH_PARAM];
const TASK_PATH_PARAMS: &[FrontendPathParam] = &[TASK_ID_PATH_PARAM];
const SESSION_PATH_PARAMS: &[FrontendPathParam] = &[SESSION_ID_PATH_PARAM];
const NO_PATH_PARAMS: &[FrontendPathParam] = &[];

const FRONTEND_ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "createProject",
        method: FrontendHttpMethod::Post,
        path_template: PROJECTS_PATH,
        request_type: "CreateProjectRequest",
        response_type: "CreateProjectResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "getProject",
        method: FrontendHttpMethod::Get,
        path_template: PROJECT_PATH,
        request_type: "GetProjectRequest",
        response_type: "GetProjectResponse",
        path_params: PROJECT_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "listProjects",
        method: FrontendHttpMethod::Get,
        path_template: PROJECTS_PATH,
        request_type: "ListProjectsRequest",
        response_type: "ListProjectsResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "updateProject",
        method: FrontendHttpMethod::Put,
        path_template: PROJECT_PATH,
        request_type: "UpdateProjectRequest",
        response_type: "UpdateProjectResponse",
        path_params: PROJECT_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "deleteProject",
        method: FrontendHttpMethod::Delete,
        path_template: PROJECT_PATH,
        request_type: "DeleteProjectRequest",
        response_type: "DeleteProjectResponse",
        path_params: PROJECT_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "openProjectWorkContext",
        method: FrontendHttpMethod::Post,
        path_template: PROJECT_WORK_CONTEXT_OPEN_PATH,
        request_type: "OpenProjectWorkContextRequest",
        response_type: "OpenProjectWorkContextResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "renewProjectWorkContext",
        method: FrontendHttpMethod::Post,
        path_template: PROJECT_WORK_CONTEXT_RENEW_PATH,
        request_type: "RenewProjectWorkContextRequest",
        response_type: "RenewProjectWorkContextResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "createTask",
        method: FrontendHttpMethod::Post,
        path_template: TASKS_PATH,
        request_type: "CreateTaskRequest",
        response_type: "CreateTaskResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "getTask",
        method: FrontendHttpMethod::Get,
        path_template: TASK_PATH,
        request_type: "GetTaskRequest",
        response_type: "GetTaskResponse",
        path_params: TASK_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "listTasks",
        method: FrontendHttpMethod::Get,
        path_template: TASKS_PATH,
        request_type: "ListTasksRequest",
        response_type: "ListTasksResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "updateTask",
        method: FrontendHttpMethod::Put,
        path_template: TASK_PATH,
        request_type: "UpdateTaskRequest",
        response_type: "UpdateTaskResponse",
        path_params: TASK_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "deleteTask",
        method: FrontendHttpMethod::Delete,
        path_template: TASK_PATH,
        request_type: "DeleteTaskRequest",
        response_type: "DeleteTaskResponse",
        path_params: TASK_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "createSession",
        method: FrontendHttpMethod::Post,
        path_template: SESSIONS_PATH,
        request_type: "CreateSessionRequest",
        response_type: "CreateSessionResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "getSession",
        method: FrontendHttpMethod::Get,
        path_template: SESSION_PATH,
        request_type: "GetSessionRequest",
        response_type: "GetSessionResponse",
        path_params: SESSION_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "listSessions",
        method: FrontendHttpMethod::Get,
        path_template: SESSIONS_PATH,
        request_type: "ListSessionsRequest",
        response_type: "ListSessionsResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "updateSession",
        method: FrontendHttpMethod::Put,
        path_template: SESSION_PATH,
        request_type: "UpdateSessionRequest",
        response_type: "UpdateSessionResponse",
        path_params: SESSION_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "deleteSession",
        method: FrontendHttpMethod::Delete,
        path_template: SESSION_PATH,
        request_type: "DeleteSessionRequest",
        response_type: "DeleteSessionResponse",
        path_params: SESSION_PATH_PARAMS,
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
        FrontendEndpoint, FrontendHttpMethod, FrontendPathParam, PROJECT_PATH,
        PROJECT_WORK_CONTEXT_OPEN_PATH, PROJECT_WORK_CONTEXT_RENEW_PATH, PROJECTS_PATH,
        SESSION_PATH, SESSIONS_PATH, TASK_PATH, TASKS_PATH, frontend_endpoints,
    };
    use pretty_assertions::assert_eq;

    /// Verifies the exported endpoint manifest matches the current CRUD route surface.
    #[test]
    fn exports_frontend_endpoint_manifest() {
        assert_eq!(
            frontend_endpoints(),
            &[
                FrontendEndpoint {
                    operation_name: "createProject",
                    method: FrontendHttpMethod::Post,
                    path_template: PROJECTS_PATH,
                    request_type: "CreateProjectRequest",
                    response_type: "CreateProjectResponse",
                    path_params: &[],
                    has_json_body: true,
                },
                FrontendEndpoint {
                    operation_name: "getProject",
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
                    method: FrontendHttpMethod::Get,
                    path_template: PROJECTS_PATH,
                    request_type: "ListProjectsRequest",
                    response_type: "ListProjectsResponse",
                    path_params: &[],
                    has_json_body: false,
                },
                FrontendEndpoint {
                    operation_name: "updateProject",
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
                    method: FrontendHttpMethod::Post,
                    path_template: PROJECT_WORK_CONTEXT_OPEN_PATH,
                    request_type: "OpenProjectWorkContextRequest",
                    response_type: "OpenProjectWorkContextResponse",
                    path_params: &[],
                    has_json_body: true,
                },
                FrontendEndpoint {
                    operation_name: "renewProjectWorkContext",
                    method: FrontendHttpMethod::Post,
                    path_template: PROJECT_WORK_CONTEXT_RENEW_PATH,
                    request_type: "RenewProjectWorkContextRequest",
                    response_type: "RenewProjectWorkContextResponse",
                    path_params: &[],
                    has_json_body: true,
                },
                FrontendEndpoint {
                    operation_name: "createTask",
                    method: FrontendHttpMethod::Post,
                    path_template: TASKS_PATH,
                    request_type: "CreateTaskRequest",
                    response_type: "CreateTaskResponse",
                    path_params: &[],
                    has_json_body: true,
                },
                FrontendEndpoint {
                    operation_name: "getTask",
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
                    method: FrontendHttpMethod::Get,
                    path_template: TASKS_PATH,
                    request_type: "ListTasksRequest",
                    response_type: "ListTasksResponse",
                    path_params: &[],
                    has_json_body: false,
                },
                FrontendEndpoint {
                    operation_name: "updateTask",
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
                    method: FrontendHttpMethod::Post,
                    path_template: SESSIONS_PATH,
                    request_type: "CreateSessionRequest",
                    response_type: "CreateSessionResponse",
                    path_params: &[],
                    has_json_body: true,
                },
                FrontendEndpoint {
                    operation_name: "getSession",
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
                    method: FrontendHttpMethod::Get,
                    path_template: SESSIONS_PATH,
                    request_type: "ListSessionsRequest",
                    response_type: "ListSessionsResponse",
                    path_params: &[],
                    has_json_body: false,
                },
                FrontendEndpoint {
                    operation_name: "updateSession",
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
            ]
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
}
