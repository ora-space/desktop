//! Endpoint declarations for the fileSystem generated-client namespace.

use crate::frontend::{FrontendEndpoint, FrontendHttpMethod, NO_PATH_PARAMS, TASK_PATH_PARAMS};
use ora_contracts::{
    FILE_SYSTEM_DIRECTORY_PATH, WORKSPACE_DIRECTORY_PATH, WORKSPACE_FILE_PATH,
    WORKSPACE_SEARCH_PATH, WORKSPACE_WATCH_PATH,
};

const NAMESPACE: &str = "fileSystem";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "listDirectory",
        namespace: NAMESPACE,
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
        namespace: NAMESPACE,
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
        namespace: NAMESPACE,
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
        namespace: NAMESPACE,
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
        namespace: NAMESPACE,
        member_name: "watchWorkspace",
        method: FrontendHttpMethod::Get,
        path_template: WORKSPACE_WATCH_PATH,
        request_type: "WatchWorkspaceRequest",
        response_type: "WorkspaceFileEventBatch",
        path_params: TASK_PATH_PARAMS,
        has_json_body: false,
    },
];
