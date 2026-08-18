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
    FrontendEndpoint {
        operation_name: "getTaskDiff",
        namespace: NAMESPACE,
        member_name: "getDiff",
        request_type: "GetTaskDiffRequest",
        response_type: "GetTaskDiffResponse",
    },
    FrontendEndpoint {
        operation_name: "commitTaskChanges",
        namespace: NAMESPACE,
        member_name: "commitChanges",
        request_type: "CommitTaskChangesRequest",
        response_type: "CommitTaskChangesResponse",
    },
    FrontendEndpoint {
        operation_name: "pushTaskBranch",
        namespace: NAMESPACE,
        member_name: "pushBranch",
        request_type: "PushTaskBranchRequest",
        response_type: "PushTaskBranchResponse",
    },
    FrontendEndpoint {
        operation_name: "listTaskDiffComments",
        namespace: NAMESPACE,
        member_name: "listDiffComments",
        request_type: "ListTaskDiffCommentsRequest",
        response_type: "ListTaskDiffCommentsResponse",
    },
    FrontendEndpoint {
        operation_name: "createTaskDiffComment",
        namespace: NAMESPACE,
        member_name: "createDiffComment",
        request_type: "CreateTaskDiffCommentRequest",
        response_type: "CreateTaskDiffCommentResponse",
    },
    FrontendEndpoint {
        operation_name: "replyTaskDiffComment",
        namespace: NAMESPACE,
        member_name: "replyDiffComment",
        request_type: "ReplyTaskDiffCommentRequest",
        response_type: "ReplyTaskDiffCommentResponse",
    },
    FrontendEndpoint {
        operation_name: "setTaskDiffCommentStatus",
        namespace: NAMESPACE,
        member_name: "setDiffCommentStatus",
        request_type: "SetTaskDiffCommentStatusRequest",
        response_type: "SetTaskDiffCommentStatusResponse",
    },
];
