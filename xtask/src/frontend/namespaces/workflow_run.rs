//! Endpoint declarations for the workflowRun generated-client namespace.

use crate::frontend::{
    FrontendEndpoint, FrontendHttpMethod, NO_PATH_PARAMS, WORKFLOW_RUN_PATH_PARAMS,
};
use ora_contracts::{WORKFLOW_RUN_NODES_PATH, WORKFLOW_RUN_PATH, WORKFLOW_RUNS_PATH};

const NAMESPACE: &str = "workflowRun";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "createWorkflowRun",
        namespace: NAMESPACE,
        member_name: "create",
        method: FrontendHttpMethod::Post,
        path_template: WORKFLOW_RUNS_PATH,
        request_type: "CreateWorkflowRunRequest",
        response_type: "CreateWorkflowRunResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "getWorkflowRun",
        namespace: NAMESPACE,
        member_name: "get",
        method: FrontendHttpMethod::Get,
        path_template: WORKFLOW_RUN_PATH,
        request_type: "GetWorkflowRunRequest",
        response_type: "GetWorkflowRunResponse",
        path_params: WORKFLOW_RUN_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "listWorkflowRuns",
        namespace: NAMESPACE,
        member_name: "list",
        method: FrontendHttpMethod::Get,
        path_template: WORKFLOW_RUNS_PATH,
        request_type: "ListWorkflowRunsRequest",
        response_type: "ListWorkflowRunsResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "listWorkflowRunsByWorkflow",
        namespace: NAMESPACE,
        member_name: "listByWorkflow",
        method: FrontendHttpMethod::Get,
        path_template: WORKFLOW_RUNS_PATH,
        request_type: "ListWorkflowRunsByWorkflowRequest",
        response_type: "ListWorkflowRunsByWorkflowResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "listWorkflowNodeRuns",
        namespace: NAMESPACE,
        member_name: "listNodeRuns",
        method: FrontendHttpMethod::Get,
        path_template: WORKFLOW_RUN_NODES_PATH,
        request_type: "ListWorkflowNodeRunsRequest",
        response_type: "ListWorkflowNodeRunsResponse",
        path_params: WORKFLOW_RUN_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "deleteWorkflowRun",
        namespace: NAMESPACE,
        member_name: "delete",
        method: FrontendHttpMethod::Delete,
        path_template: WORKFLOW_RUN_PATH,
        request_type: "DeleteWorkflowRunRequest",
        response_type: "DeleteWorkflowRunResponse",
        path_params: WORKFLOW_RUN_PATH_PARAMS,
        has_json_body: false,
    },
];
