use crate::app_state::AppState;
use crate::error::WebApiError;
use axum::Json;
use axum::extract::{Path, Query, State};
use ora_contracts::{
    CommitTaskChangesRequest, CommitTaskChangesResponse, CreateTaskDiffCommentRequest,
    CreateTaskDiffCommentResponse, GetTaskDiffRequest, GetTaskDiffResponse,
    ListTaskDiffCommentsRequest, ListTaskDiffCommentsResponse, PushTaskBranchRequest,
    PushTaskBranchResponse, ReplyTaskDiffCommentRequest, ReplyTaskDiffCommentResponse,
    SetTaskDiffCommentStatusRequest, SetTaskDiffCommentStatusResponse, TaskDiffCommentAnchor,
    TaskDiffScope, TaskDiffThreadStatus,
};
use serde::Deserialize;
use std::sync::Arc;

/// Carries the task identifier used by task diff routes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDiffPath {
    task_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDiffQuery {
    scope: TaskDiffScope,
}

/// Carries task and comment identifiers used by reply and status routes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDiffCommentPath {
    task_id: String,
    comment_id: String,
}

/// Carries the transport body for creating a root diff discussion.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskDiffCommentBody {
    anchor: TaskDiffCommentAnchor,
    body: String,
}

/// Carries the transport body for replying to a diff discussion.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplyTaskDiffCommentBody {
    body: String,
}

/// Carries the transport body for resolving or reopening a diff discussion.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetTaskDiffCommentStatusBody {
    status: TaskDiffThreadStatus,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitTaskChangesBody {
    message: String,
}

/// Returns a standard unified patch for one task worktree.
pub async fn get_task_diff(
    State(app_state): State<AppState>,
    Path(path): Path<TaskDiffPath>,
    Query(query): Query<TaskDiffQuery>,
) -> Result<Json<GetTaskDiffResponse>, WebApiError> {
    let api = Arc::clone(app_state.task_diff_api());
    run_blocking(move || {
        api.get_diff(GetTaskDiffRequest {
            task_id: path.task_id,
            scope: query.scope,
        })
    })
    .await
}

/// Commits every current change in one task-owned worktree.
pub async fn commit_task_changes(
    State(app_state): State<AppState>,
    Path(path): Path<TaskDiffPath>,
    Json(body): Json<CommitTaskChangesBody>,
) -> Result<Json<CommitTaskChangesResponse>, WebApiError> {
    let api = Arc::clone(app_state.task_diff_api());
    run_blocking(move || {
        api.commit_changes(CommitTaskChangesRequest {
            task_id: path.task_id,
            message: body.message,
        })
    })
    .await
}

/// Pushes the current task branch to its default remote.
pub async fn push_task_branch(
    State(app_state): State<AppState>,
    Path(path): Path<TaskDiffPath>,
) -> Result<Json<PushTaskBranchResponse>, WebApiError> {
    let api = Arc::clone(app_state.task_diff_api());
    run_blocking(move || {
        api.push_branch(PushTaskBranchRequest {
            task_id: path.task_id,
        })
    })
    .await
}

/// Lists every persisted discussion message for one task diff.
pub async fn list_task_diff_comments(
    State(app_state): State<AppState>,
    Path(path): Path<TaskDiffPath>,
) -> Result<Json<ListTaskDiffCommentsResponse>, WebApiError> {
    let api = Arc::clone(app_state.task_diff_api());
    run_blocking(move || {
        api.list_comments(ListTaskDiffCommentsRequest {
            task_id: path.task_id,
        })
    })
    .await
}

/// Creates one line-anchored task diff discussion.
pub async fn create_task_diff_comment(
    State(app_state): State<AppState>,
    Path(path): Path<TaskDiffPath>,
    Json(body): Json<CreateTaskDiffCommentBody>,
) -> Result<Json<CreateTaskDiffCommentResponse>, WebApiError> {
    let api = Arc::clone(app_state.task_diff_api());
    run_blocking(move || {
        api.create_comment(CreateTaskDiffCommentRequest {
            task_id: path.task_id,
            anchor: body.anchor,
            body: body.body,
        })
    })
    .await
}

/// Adds one reply under an existing task diff discussion message.
pub async fn reply_task_diff_comment(
    State(app_state): State<AppState>,
    Path(path): Path<TaskDiffCommentPath>,
    Json(body): Json<ReplyTaskDiffCommentBody>,
) -> Result<Json<ReplyTaskDiffCommentResponse>, WebApiError> {
    let api = Arc::clone(app_state.task_diff_api());
    run_blocking(move || {
        api.reply_comment(ReplyTaskDiffCommentRequest {
            task_id: path.task_id,
            comment_id: path.comment_id,
            body: body.body,
        })
    })
    .await
}

/// Resolves or reopens one root task diff discussion.
pub async fn set_task_diff_comment_status(
    State(app_state): State<AppState>,
    Path(path): Path<TaskDiffCommentPath>,
    Json(body): Json<SetTaskDiffCommentStatusBody>,
) -> Result<Json<SetTaskDiffCommentStatusResponse>, WebApiError> {
    let api = Arc::clone(app_state.task_diff_api());
    run_blocking(move || {
        api.set_comment_status(SetTaskDiffCommentStatusRequest {
            task_id: path.task_id,
            comment_id: path.comment_id,
            status: body.status,
        })
    })
    .await
}

/// Runs synchronous Git and SQLite application services outside Tokio's asynchronous worker pool.
async fn run_blocking<Response, Operation>(
    operation: Operation,
) -> Result<Json<Response>, WebApiError>
where
    Response: Send + 'static,
    Operation: FnOnce() -> Result<Response, ora_application::ApplicationError> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| WebApiError::internal_error(format!("task diff worker failed: {error}")))?
        .map(Json)
        .map_err(WebApiError::from)
}
