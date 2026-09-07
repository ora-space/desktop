//! Endpoint declarations for the project generated-client namespace.

use crate::frontend::{FrontendEndpoint, FrontendResponseMode};

const NAMESPACE: &str = "project";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "createProject",
        namespace: NAMESPACE,
        member_name: "create",
        request_type: "CreateProjectRequest",
        response_type: "CreateProjectResponse",
        response_mode: FrontendResponseMode::Unary,
    },
    FrontendEndpoint {
        operation_name: "getProject",
        namespace: NAMESPACE,
        member_name: "get",
        request_type: "GetProjectRequest",
        response_type: "GetProjectResponse",
        response_mode: FrontendResponseMode::Unary,
    },
    FrontendEndpoint {
        operation_name: "listProjects",
        namespace: NAMESPACE,
        member_name: "list",
        request_type: "ListProjectsRequest",
        response_type: "ListProjectsResponse",
        response_mode: FrontendResponseMode::Unary,
    },
    FrontendEndpoint {
        operation_name: "listProjectBranches",
        namespace: NAMESPACE,
        member_name: "listBranches",
        request_type: "ListProjectBranchesRequest",
        response_type: "ListProjectBranchesResponse",
        response_mode: FrontendResponseMode::Unary,
    },
    FrontendEndpoint {
        operation_name: "updateProject",
        namespace: NAMESPACE,
        member_name: "update",
        request_type: "UpdateProjectRequest",
        response_type: "UpdateProjectResponse",
        response_mode: FrontendResponseMode::Unary,
    },
    FrontendEndpoint {
        operation_name: "deleteProject",
        namespace: NAMESPACE,
        member_name: "delete",
        request_type: "DeleteProjectRequest",
        response_type: "DeleteProjectResponse",
        response_mode: FrontendResponseMode::Unary,
    },
];
