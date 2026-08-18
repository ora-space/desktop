//! Endpoint declarations for the workflowRun generated-client namespace.

use crate::frontend::FrontendEndpoint;

const NAMESPACE: &str = "workflowRun";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "createWorkflowRun",
        namespace: NAMESPACE,
        member_name: "create",
        request_type: "CreateWorkflowRunRequest",
        response_type: "CreateWorkflowRunResponse",
    },
    FrontendEndpoint {
        operation_name: "getWorkflowRun",
        namespace: NAMESPACE,
        member_name: "get",
        request_type: "GetWorkflowRunRequest",
        response_type: "GetWorkflowRunResponse",
    },
    FrontendEndpoint {
        operation_name: "listWorkflowRuns",
        namespace: NAMESPACE,
        member_name: "list",
        request_type: "ListWorkflowRunsRequest",
        response_type: "ListWorkflowRunsResponse",
    },
    FrontendEndpoint {
        operation_name: "listWorkflowRunsByWorkflow",
        namespace: NAMESPACE,
        member_name: "listByWorkflow",
        request_type: "ListWorkflowRunsByWorkflowRequest",
        response_type: "ListWorkflowRunsByWorkflowResponse",
    },
    FrontendEndpoint {
        operation_name: "listWorkflowNodeRuns",
        namespace: NAMESPACE,
        member_name: "listNodeRuns",
        request_type: "ListWorkflowNodeRunsRequest",
        response_type: "ListWorkflowNodeRunsResponse",
    },
    FrontendEndpoint {
        operation_name: "deleteWorkflowRun",
        namespace: NAMESPACE,
        member_name: "delete",
        request_type: "DeleteWorkflowRunRequest",
        response_type: "DeleteWorkflowRunResponse",
    },
    FrontendEndpoint {
        operation_name: "startWorkflowRun",
        namespace: NAMESPACE,
        member_name: "start",
        request_type: "StartWorkflowRunRequest",
        response_type: "StartWorkflowRunResponse",
    },
    FrontendEndpoint {
        operation_name: "cancelWorkflowRun",
        namespace: NAMESPACE,
        member_name: "cancel",
        request_type: "CancelWorkflowRunRequest",
        response_type: "CancelWorkflowRunResponse",
    },
    FrontendEndpoint {
        operation_name: "restartWorkflowRun",
        namespace: NAMESPACE,
        member_name: "restart",
        request_type: "RestartWorkflowRunRequest",
        response_type: "RestartWorkflowRunResponse",
    },
    FrontendEndpoint {
        operation_name: "updateWorkflowRunInput",
        namespace: NAMESPACE,
        member_name: "updateInput",
        request_type: "UpdateWorkflowRunInputRequest",
        response_type: "UpdateWorkflowRunInputResponse",
    },
];
