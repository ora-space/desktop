//! Endpoint declarations for the workflowRun generated-client namespace.

use crate::frontend::{FrontendEndpoint, FrontendResponseMode};

const NAMESPACE: &str = "workflowRun";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "createWorkflowRun",
        namespace: NAMESPACE,
        member_name: "create",
        request_type: "CreateWorkflowRunRequest",
        response_type: "CreateWorkflowRunResponse",
        response_mode: FrontendResponseMode::Unary,
    },
    FrontendEndpoint {
        operation_name: "getWorkflowRun",
        namespace: NAMESPACE,
        member_name: "get",
        request_type: "GetWorkflowRunRequest",
        response_type: "GetWorkflowRunResponse",
        response_mode: FrontendResponseMode::Unary,
    },
    FrontendEndpoint {
        operation_name: "listWorkflowRuns",
        namespace: NAMESPACE,
        member_name: "list",
        request_type: "ListWorkflowRunsRequest",
        response_type: "ListWorkflowRunsResponse",
        response_mode: FrontendResponseMode::Unary,
    },
    FrontendEndpoint {
        operation_name: "listWorkflowRunsByWorkflow",
        namespace: NAMESPACE,
        member_name: "listByWorkflow",
        request_type: "ListWorkflowRunsByWorkflowRequest",
        response_type: "ListWorkflowRunsByWorkflowResponse",
        response_mode: FrontendResponseMode::Unary,
    },
    FrontendEndpoint {
        operation_name: "listWorkflowNodeRuns",
        namespace: NAMESPACE,
        member_name: "listNodeRuns",
        request_type: "ListWorkflowNodeRunsRequest",
        response_type: "ListWorkflowNodeRunsResponse",
        response_mode: FrontendResponseMode::Unary,
    },
    FrontendEndpoint {
        operation_name: "deleteWorkflowRun",
        namespace: NAMESPACE,
        member_name: "delete",
        request_type: "DeleteWorkflowRunRequest",
        response_type: "DeleteWorkflowRunResponse",
        response_mode: FrontendResponseMode::Unary,
    },
    FrontendEndpoint {
        operation_name: "renameWorkflowRun",
        namespace: NAMESPACE,
        member_name: "rename",
        request_type: "RenameWorkflowRunRequest",
        response_type: "RenameWorkflowRunResponse",
        response_mode: FrontendResponseMode::Unary,
    },
    FrontendEndpoint {
        operation_name: "startWorkflowRun",
        namespace: NAMESPACE,
        member_name: "start",
        request_type: "StartWorkflowRunRequest",
        response_type: "StartWorkflowRunResponse",
        response_mode: FrontendResponseMode::Unary,
    },
    FrontendEndpoint {
        operation_name: "cancelWorkflowRun",
        namespace: NAMESPACE,
        member_name: "cancel",
        request_type: "CancelWorkflowRunRequest",
        response_type: "CancelWorkflowRunResponse",
        response_mode: FrontendResponseMode::Unary,
    },
    FrontendEndpoint {
        operation_name: "restartWorkflowRun",
        namespace: NAMESPACE,
        member_name: "restart",
        request_type: "RestartWorkflowRunRequest",
        response_type: "RestartWorkflowRunResponse",
        response_mode: FrontendResponseMode::Unary,
    },
    FrontendEndpoint {
        operation_name: "updateWorkflowRunInput",
        namespace: NAMESPACE,
        member_name: "updateInput",
        request_type: "UpdateWorkflowRunInputRequest",
        response_type: "UpdateWorkflowRunInputResponse",
        response_mode: FrontendResponseMode::Unary,
    },
    FrontendEndpoint {
        operation_name: "completeWorkflowNode",
        namespace: NAMESPACE,
        member_name: "completeNode",
        request_type: "CompleteWorkflowNodeRequest",
        response_type: "CompleteWorkflowNodeResponse",
        response_mode: FrontendResponseMode::Unary,
    },
];
