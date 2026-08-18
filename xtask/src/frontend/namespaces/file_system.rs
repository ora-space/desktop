//! Endpoint declarations for the fileSystem generated-client namespace.

use crate::frontend::FrontendEndpoint;

const NAMESPACE: &str = "fileSystem";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "listWorkspaceDirectory",
        namespace: NAMESPACE,
        member_name: "listWorkspaceDirectory",

        request_type: "ListWorkspaceDirectoryRequest",
        response_type: "ListWorkspaceDirectoryResponse",
    },
    FrontendEndpoint {
        operation_name: "readWorkspaceFile",
        namespace: NAMESPACE,
        member_name: "readWorkspaceFile",
        request_type: "ReadWorkspaceFileRequest",
        response_type: "ReadWorkspaceFileResponse",
    },
    FrontendEndpoint {
        operation_name: "searchWorkspace",
        namespace: NAMESPACE,
        member_name: "searchWorkspace",
        request_type: "SearchWorkspaceRequest",
        response_type: "SearchWorkspaceResponse",
    },
    FrontendEndpoint {
        operation_name: "watchWorkspace",
        namespace: NAMESPACE,
        member_name: "watchWorkspace",
        request_type: "WatchWorkspaceRequest",
        response_type: "WorkspaceFileEventBatch",
    },
];
