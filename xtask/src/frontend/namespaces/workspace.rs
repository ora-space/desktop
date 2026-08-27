//! Endpoint declarations for the workspace generated-client namespace.

use crate::frontend::FrontendEndpoint;

const NAMESPACE: &str = "workspace";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "listWorkspaces",
        namespace: NAMESPACE,
        member_name: "list",
        request_type: "ListWorkspacesRequest",
        response_type: "ListWorkspacesResponse",
    },
    FrontendEndpoint {
        operation_name: "getWorkspaceDiff",
        namespace: NAMESPACE,
        member_name: "getDiff",
        request_type: "GetWorkspaceDiffRequest",
        response_type: "GetWorkspaceDiffResponse",
    },
    FrontendEndpoint {
        operation_name: "commitWorkspaceChanges",
        namespace: NAMESPACE,
        member_name: "commitChanges",
        request_type: "CommitWorkspaceChangesRequest",
        response_type: "CommitWorkspaceChangesResponse",
    },
    FrontendEndpoint {
        operation_name: "pushWorkspaceBranch",
        namespace: NAMESPACE,
        member_name: "pushBranch",
        request_type: "PushWorkspaceBranchRequest",
        response_type: "PushWorkspaceBranchResponse",
    },
];
