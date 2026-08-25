use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Describes the public task payload shared across adapter responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task.ts")]
pub struct Task {
    pub id: String,
    pub project_id: String,
    pub workspace_id: String,
    pub title: String,
}

/// Carries the app-facing payload for task creation requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task.ts")]
pub struct CreateTaskRequest {
    pub project_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub base_branch: Option<String>,
}

/// Returns the created task after a successful create request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task.ts")]
pub struct CreateTaskResponse {
    pub task: Task,
}

/// Identifies which task to fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task.ts")]
pub struct GetTaskRequest {
    pub task_id: String,
}

/// Returns one task payload after a successful fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task.ts")]
pub struct GetTaskResponse {
    pub task: Task,
}

/// Requests the active workspace for one task without exposing checkout paths to callers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task.ts")]
pub struct GetTaskWorkspaceRequest {
    pub task_id: String,
}

/// Describes the absolute checkout root and branch the backend resolved for one task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task.ts")]
pub struct TaskWorkspace {
    pub root_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub branch_name: Option<String>,
}

/// Returns one task-owned workspace without exposing repository internals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task.ts")]
pub struct GetTaskWorkspaceResponse {
    pub workspace: TaskWorkspace,
}

/// Requests the full visible task list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task.ts")]
pub struct ListTasksRequest {}

/// Returns the visible task list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task.ts")]
pub struct ListTasksResponse {
    pub tasks: Vec<Task>,
}

/// Carries the full replacement payload for task updates in the first slice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task.ts")]
pub struct UpdateTaskRequest {
    pub task_id: String,
    pub title: String,
}

/// Returns the updated task after a successful update request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task.ts")]
pub struct UpdateTaskResponse {
    pub task: Task,
}

/// Identifies which task to delete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task.ts")]
pub struct DeleteTaskRequest {
    pub task_id: String,
}

/// Returns the deleted task identifier after a successful delete request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task.ts")]
pub struct DeleteTaskResponse {
    pub task_id: String,
    pub workspace_id: String,
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    Task::export(config)?;
    CreateTaskRequest::export(config)?;
    CreateTaskResponse::export(config)?;
    GetTaskRequest::export(config)?;
    GetTaskResponse::export(config)?;
    GetTaskWorkspaceRequest::export(config)?;
    GetTaskWorkspaceResponse::export(config)?;
    TaskWorkspace::export(config)?;
    ListTasksRequest::export(config)?;
    ListTasksResponse::export(config)?;
    UpdateTaskRequest::export(config)?;
    UpdateTaskResponse::export(config)?;
    DeleteTaskRequest::export(config)?;
    DeleteTaskResponse::export(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        CreateTaskRequest, CreateTaskResponse, DeleteTaskRequest, DeleteTaskResponse,
        GetTaskRequest, GetTaskResponse, GetTaskWorkspaceResponse, ListTasksRequest,
        ListTasksResponse, Task, TaskWorkspace, UpdateTaskRequest, UpdateTaskResponse,
    };
    use pretty_assertions::assert_eq;
    use serde::Serialize;
    use serde_json::{Value, json};

    /// Verifies the first task slice serializes to frontend-friendly JSON payloads.
    #[test]
    fn serializes_task_contracts() {
        let task = Task {
            id: "task-1".to_string(),
            project_id: "project-1".to_string(),
            workspace_id: "workspace-1".to_string(),
            title: "Ship handlers".to_string(),
        };
        let create_request = CreateTaskRequest {
            project_id: "project-1".to_string(),
            title: "Ship handlers".to_string(),
            base_branch: Some("main".to_string()),
        };
        let get_request = GetTaskRequest {
            task_id: "task-1".to_string(),
        };
        let list_request = ListTasksRequest {};
        let update_request = UpdateTaskRequest {
            task_id: "task-1".to_string(),
            title: "Ship updated handlers".to_string(),
        };
        let delete_request = DeleteTaskRequest {
            task_id: "task-1".to_string(),
        };

        assert_serialized_json(
            &task,
            json!({
                "id": "task-1",
                "projectId": "project-1",
                "workspaceId": "workspace-1",
                "title": "Ship handlers",
            }),
        );
        assert_serialized_json(
            &create_request,
            json!({
                "projectId": "project-1",
                "title": "Ship handlers",
                "baseBranch": "main",
            }),
        );
        assert_serialized_json(
            &CreateTaskResponse { task: task.clone() },
            json!({
                "task": {
                    "id": "task-1",
                    "projectId": "project-1",
                    "workspaceId": "workspace-1",
                    "title": "Ship handlers",
                },
            }),
        );
        assert_serialized_json(&get_request, json!({ "taskId": "task-1" }));
        assert_serialized_json(
            &GetTaskResponse { task: task.clone() },
            json!({
                "task": {
                    "id": "task-1",
                    "projectId": "project-1",
                    "workspaceId": "workspace-1",
                    "title": "Ship handlers",
                },
            }),
        );
        assert_serialized_json(&list_request, json!({}));
        assert_serialized_json(
            &ListTasksResponse {
                tasks: vec![task.clone()],
            },
            json!({
                "tasks": [
                    {
                        "id": "task-1",
                        "projectId": "project-1",
                        "workspaceId": "workspace-1",
                        "title": "Ship handlers",
                    },
                ],
            }),
        );
        assert_serialized_json(
            &update_request,
            json!({
                "taskId": "task-1",
                "title": "Ship updated handlers",
            }),
        );
        assert_serialized_json(
            &UpdateTaskResponse { task },
            json!({
                "task": {
                    "id": "task-1",
                    "projectId": "project-1",
                    "workspaceId": "workspace-1",
                    "title": "Ship handlers",
                },
            }),
        );
        assert_serialized_json(&delete_request, json!({ "taskId": "task-1" }));
        assert_serialized_json(
            &DeleteTaskResponse {
                task_id: "task-1".to_string(),
                workspace_id: "workspace-1".to_string(),
            },
            json!({ "taskId": "task-1", "workspaceId": "workspace-1" }),
        );
    }

    /// Confirms the shared task view remains the single reusable payload across responses.
    #[test]
    fn preserves_shared_task_shape_across_responses() {
        let task = Task {
            id: "task-1".to_string(),
            project_id: "project-1".to_string(),
            workspace_id: "workspace-1".to_string(),
            title: "Ship handlers".to_string(),
        };

        assert_eq!(
            CreateTaskResponse { task: task.clone() },
            CreateTaskResponse { task: task.clone() }
        );
        assert_eq!(
            GetTaskResponse { task: task.clone() },
            GetTaskResponse { task: task.clone() }
        );
        assert_eq!(
            ListTasksResponse {
                tasks: vec![task.clone()],
            },
            ListTasksResponse {
                tasks: vec![task.clone()],
            }
        );
        assert_eq!(
            UpdateTaskResponse { task: task.clone() },
            UpdateTaskResponse { task }
        );
    }

    /// Verifies main-checkout and detached contexts omit the optional branch without changing the root shape.
    #[test]
    fn serializes_task_workspace_without_a_branch() {
        assert_serialized_json(
            &GetTaskWorkspaceResponse {
                workspace: TaskWorkspace {
                    root_path: "C:/projects/ora".to_string(),
                    branch_name: None,
                },
            },
            json!({ "workspace": { "rootPath": "C:/projects/ora" } }),
        );
    }

    /// Serializes one value and compares the full JSON payload so field names stay stable.
    fn assert_serialized_json(value: &impl Serialize, expected: Value) {
        assert_eq!(serde_json::to_value(value).unwrap(), expected);
    }
}
