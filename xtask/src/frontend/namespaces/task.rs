//! Endpoint declarations for the task generated-client namespace.

use crate::frontend::FrontendEndpoint;

const NAMESPACE: &str = "task";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "createTask",
        namespace: NAMESPACE,
        member_name: "create",
        request_type: "CreateTaskRequest",
        response_type: "CreateTaskResponse",
    },
    FrontendEndpoint {
        operation_name: "getTask",
        namespace: NAMESPACE,
        member_name: "get",
        request_type: "GetTaskRequest",
        response_type: "GetTaskResponse",
    },
    FrontendEndpoint {
        operation_name: "listTasks",
        namespace: NAMESPACE,
        member_name: "list",
        request_type: "ListTasksRequest",
        response_type: "ListTasksResponse",
    },
    FrontendEndpoint {
        operation_name: "updateTask",
        namespace: NAMESPACE,
        member_name: "update",
        request_type: "UpdateTaskRequest",
        response_type: "UpdateTaskResponse",
    },
    FrontendEndpoint {
        operation_name: "deleteTask",
        namespace: NAMESPACE,
        member_name: "delete",
        request_type: "DeleteTaskRequest",
        response_type: "DeleteTaskResponse",
    },
    FrontendEndpoint {
        operation_name: "getTaskWorkspace",
        namespace: NAMESPACE,
        member_name: "getWorkspace",
        request_type: "GetTaskWorkspaceRequest",
        response_type: "GetTaskWorkspaceResponse",
    },
];
