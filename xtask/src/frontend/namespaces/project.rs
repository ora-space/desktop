//! Endpoint declarations for the project generated-client namespace.

use crate::frontend::FrontendEndpoint;

const NAMESPACE: &str = "project";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "createProject",
        namespace: NAMESPACE,
        member_name: "create",
        request_type: "CreateProjectRequest",
        response_type: "CreateProjectResponse",
    },
    FrontendEndpoint {
        operation_name: "getProject",
        namespace: NAMESPACE,
        member_name: "get",
        request_type: "GetProjectRequest",
        response_type: "GetProjectResponse",
    },
    FrontendEndpoint {
        operation_name: "listProjects",
        namespace: NAMESPACE,
        member_name: "list",
        request_type: "ListProjectsRequest",
        response_type: "ListProjectsResponse",
    },
    FrontendEndpoint {
        operation_name: "listProjectBranches",
        namespace: NAMESPACE,
        member_name: "listBranches",
        request_type: "ListProjectBranchesRequest",
        response_type: "ListProjectBranchesResponse",
    },
    FrontendEndpoint {
        operation_name: "updateProject",
        namespace: NAMESPACE,
        member_name: "update",
        request_type: "UpdateProjectRequest",
        response_type: "UpdateProjectResponse",
    },
    FrontendEndpoint {
        operation_name: "deleteProject",
        namespace: NAMESPACE,
        member_name: "delete",
        request_type: "DeleteProjectRequest",
        response_type: "DeleteProjectResponse",
    },
];
