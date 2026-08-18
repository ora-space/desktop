//! Endpoint declarations for the workflow generated-client namespace.

use crate::frontend::FrontendEndpoint;

const NAMESPACE: &str = "workflow";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "createWorkflow",
        namespace: NAMESPACE,
        member_name: "create",
        request_type: "CreateWorkflowRequest",
        response_type: "CreateWorkflowResponse",
    },
    FrontendEndpoint {
        operation_name: "getWorkflow",
        namespace: NAMESPACE,
        member_name: "get",
        request_type: "GetWorkflowRequest",
        response_type: "GetWorkflowResponse",
    },
    FrontendEndpoint {
        operation_name: "listWorkflows",
        namespace: NAMESPACE,
        member_name: "list",
        request_type: "ListWorkflowsRequest",
        response_type: "ListWorkflowsResponse",
    },
    FrontendEndpoint {
        operation_name: "updateWorkflow",
        namespace: NAMESPACE,
        member_name: "update",
        request_type: "UpdateWorkflowRequest",
        response_type: "UpdateWorkflowResponse",
    },
    FrontendEndpoint {
        operation_name: "deleteWorkflow",
        namespace: NAMESPACE,
        member_name: "delete",
        request_type: "DeleteWorkflowRequest",
        response_type: "DeleteWorkflowResponse",
    },
    FrontendEndpoint {
        operation_name: "getDraft",
        namespace: NAMESPACE,
        member_name: "getDraft",
        request_type: "GetDraftRequest",
        response_type: "GetDraftResponse",
    },
    FrontendEndpoint {
        operation_name: "updateDraft",
        namespace: NAMESPACE,
        member_name: "updateDraft",
        request_type: "UpdateDraftRequest",
        response_type: "UpdateDraftResponse",
    },
    FrontendEndpoint {
        operation_name: "publishWorkflow",
        namespace: NAMESPACE,
        member_name: "publish",
        request_type: "PublishWorkflowRequest",
        response_type: "PublishWorkflowResponse",
    },
    FrontendEndpoint {
        operation_name: "rollbackWorkflow",
        namespace: NAMESPACE,
        member_name: "rollback",
        request_type: "RollbackWorkflowRequest",
        response_type: "RollbackWorkflowResponse",
    },
    FrontendEndpoint {
        operation_name: "activateWorkflow",
        namespace: NAMESPACE,
        member_name: "activate",
        request_type: "ActivateWorkflowRequest",
        response_type: "ActivateWorkflowResponse",
    },
    FrontendEndpoint {
        operation_name: "listVersions",
        namespace: NAMESPACE,
        member_name: "listVersions",
        request_type: "ListVersionsRequest",
        response_type: "ListVersionsResponse",
    },
    FrontendEndpoint {
        operation_name: "getVersion",
        namespace: NAMESPACE,
        member_name: "getVersion",
        request_type: "GetVersionRequest",
        response_type: "GetVersionResponse",
    },
    FrontendEndpoint {
        operation_name: "deleteSnapshot",
        namespace: NAMESPACE,
        member_name: "deleteSnapshot",
        request_type: "DeleteSnapshotRequest",
        response_type: "DeleteSnapshotResponse",
    },
    FrontendEndpoint {
        operation_name: "getWorkflowSnapshot",
        namespace: NAMESPACE,
        member_name: "getSnapshot",
        request_type: "GetWorkflowSnapshotRequest",
        response_type: "GetWorkflowSnapshotResponse",
    },
];
