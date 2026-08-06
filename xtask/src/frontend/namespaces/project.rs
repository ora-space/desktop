//! Endpoint declarations for the project generated-client namespace.

use crate::frontend::{FrontendEndpoint, FrontendHttpMethod, NO_PATH_PARAMS, PROJECT_PATH_PARAMS};
use ora_contracts::{PROJECT_BRANCHES_PATH, PROJECT_PATH, PROJECTS_PATH};

const NAMESPACE: &str = "project";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "createProject",
        namespace: NAMESPACE,
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
        namespace: NAMESPACE,
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
        namespace: NAMESPACE,
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
        namespace: NAMESPACE,
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
        namespace: NAMESPACE,
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
        namespace: NAMESPACE,
        member_name: "delete",
        method: FrontendHttpMethod::Delete,
        path_template: PROJECT_PATH,
        request_type: "DeleteProjectRequest",
        response_type: "DeleteProjectResponse",
        path_params: PROJECT_PATH_PARAMS,
        has_json_body: false,
    },
];
