use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Describes the public task status shared across adapter boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task.ts")]
pub enum TaskStatus {
    Todo,
    Doing,
    Done,
}

/// Selects the filesystem context used when a task starts an agent session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "task.ts")]
pub enum TaskWorkspaceMode {
    /// Creates an isolated linked Git worktree owned by the task.
    #[default]
    Worktree,
    /// Uses the owning project's root directory directly without creating a worktree.
    ProjectRoot,
}

/// Describes the public task payload shared across adapter responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task.ts")]
pub struct Task {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub status: TaskStatus,
    pub workspace_mode: TaskWorkspaceMode,
}

/// Carries the app-facing payload for task creation requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task.ts")]
pub struct CreateTaskRequest {
    pub project_id: String,
    pub title: String,
    pub status: TaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub workspace_mode: Option<TaskWorkspaceMode>,
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
    pub branch_name: String,
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
    pub status: TaskStatus,
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
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    TaskStatus::export(config)?;
    TaskWorkspaceMode::export(config)?;
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
        GetTaskRequest, GetTaskResponse, ListTasksRequest, ListTasksResponse, Task, TaskStatus,
        TaskWorkspaceMode, UpdateTaskRequest, UpdateTaskResponse,
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
            title: "Ship handlers".to_string(),
            status: TaskStatus::Doing,
            workspace_mode: TaskWorkspaceMode::Worktree,
        };
        let create_request = CreateTaskRequest {
            project_id: "project-1".to_string(),
            title: "Ship handlers".to_string(),
            status: TaskStatus::Todo,
            workspace_mode: None,
        };
        let get_request = GetTaskRequest {
            task_id: "task-1".to_string(),
        };
        let list_request = ListTasksRequest {};
        let update_request = UpdateTaskRequest {
            task_id: "task-1".to_string(),
            title: "Ship updated handlers".to_string(),
            status: TaskStatus::Done,
        };
        let delete_request = DeleteTaskRequest {
            task_id: "task-1".to_string(),
        };

        assert_serialized_json(
            &task,
            json!({
                "id": "task-1",
                "projectId": "project-1",
                "title": "Ship handlers",
                "status": "doing",
                "workspaceMode": "worktree",
            }),
        );
        assert_serialized_json(
            &create_request,
            json!({
                "projectId": "project-1",
                "title": "Ship handlers",
                "status": "todo",
            }),
        );
        assert_serialized_json(
            &CreateTaskResponse { task: task.clone() },
            json!({
                "task": {
                    "id": "task-1",
                    "projectId": "project-1",
                "title": "Ship handlers",
                "status": "doing",
                "workspaceMode": "worktree",
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
                "title": "Ship handlers",
                "status": "doing",
                "workspaceMode": "worktree",
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
                        "title": "Ship handlers",
                        "status": "doing",
                        "workspaceMode": "worktree",
                    },
                ],
            }),
        );
        assert_serialized_json(
            &update_request,
            json!({
                "taskId": "task-1",
                "title": "Ship updated handlers",
                "status": "done",
            }),
        );
        assert_serialized_json(
            &UpdateTaskResponse { task },
            json!({
                "task": {
                    "id": "task-1",
                    "projectId": "project-1",
                    "title": "Ship handlers",
                    "status": "doing",
                    "workspaceMode": "worktree",
                },
            }),
        );
        assert_serialized_json(&delete_request, json!({ "taskId": "task-1" }));
        assert_serialized_json(
            &DeleteTaskResponse {
                task_id: "task-1".to_string(),
            },
            json!({ "taskId": "task-1" }),
        );
    }

    /// Confirms the shared task view remains the single reusable payload across responses.
    #[test]
    fn preserves_shared_task_shape_across_responses() {
        let task = Task {
            id: "task-1".to_string(),
            project_id: "project-1".to_string(),
            title: "Ship handlers".to_string(),
            status: TaskStatus::Todo,
            workspace_mode: TaskWorkspaceMode::Worktree,
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

    /// Serializes one value and compares the full JSON payload so field names stay stable.
    fn assert_serialized_json(value: &impl Serialize, expected: Value) {
        assert_eq!(serde_json::to_value(value).unwrap(), expected);
    }
}
