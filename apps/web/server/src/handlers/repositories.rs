use crate::app_state::AppState;
use crate::error::WebApiError;
use axum::Json;
use axum::extract::{Path, Query, State};
use ora_contracts::{
    CheckoutRepositoryBranchRequest, CheckoutRepositoryBranchResponse,
    CommitRepositoryChangesRequest, CommitRepositoryChangesResponse, CreateRepositoryBranchRequest,
    CreateRepositoryBranchResponse, FetchRepositoryRequest, FetchRepositoryResponse,
    GetRepositoryCommitDiffRequest, GetRepositoryCommitDiffResponse, GetRepositoryCommitRequest,
    GetRepositoryCommitResponse, GetRepositorySnapshotRequest, GetRepositorySnapshotResponse,
    GetRepositoryWorkingTreeDiffRequest, GetRepositoryWorkingTreeDiffResponse,
    PullRepositoryRequest, PullRepositoryResponse, PullRepositoryStrategy,
    PushRepositoryBranchRequest, PushRepositoryBranchResponse, RepositoryChangeSelection,
    RepositoryConflictSide, RepositorySyncAction, ResolveRepositoryConflictRequest,
    ResolveRepositoryConflictResponse, ResolveRepositorySyncRequest, ResolveRepositorySyncResponse,
    StageRepositoryChangesRequest, StageRepositoryChangesResponse, UnstageRepositoryChangesRequest,
    UnstageRepositoryChangesResponse,
};
use serde::Deserialize;

/// Carries the project identifier shared by repository snapshot routes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryProjectPath {
    project_id: String,
}

/// Carries the project and commit identifiers needed by a commit detail route.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryCommitPath {
    project_id: String,
    commit_id: String,
}

/// Carries the selected commit's first parent for a bounded patch request.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryCommitDiffQuery {
    parent_commit_id: Option<String>,
    path: String,
}

/// Carries the branch name from mutation request bodies while the project comes from the path.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryBranchBody {
    branch_name: String,
}

/// Carries the user-selected change scope while the project comes from the path.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryChangesBody {
    selection: RepositoryChangeSelection,
}

/// Carries the commit message while the project comes from the path.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryCommitBody {
    message: String,
}

/// Carries the selected merge strategy while the project comes from the path.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryPullBody {
    strategy: PullRepositoryStrategy,
}

/// Carries the selected conflict resolution action while the project comes from the path.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositorySyncBody {
    action: RepositorySyncAction,
}

/// Carries the path and selected Git side while the project comes from the route.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryConflictBody {
    path: String,
    side: RepositoryConflictSide,
}

/// Reads one bounded repository graph snapshot on a blocking worker.
pub async fn get_snapshot(
    State(app_state): State<AppState>,
    Path(path): Path<RepositoryProjectPath>,
) -> Result<Json<GetRepositorySnapshotResponse>, WebApiError> {
    let backend = app_state.backend().clone();
    tokio::task::spawn_blocking(move || {
        backend.get_repository_snapshot(GetRepositorySnapshotRequest {
            project_id: path.project_id,
        })
    })
    .await
    .map_err(|source| WebApiError::internal("repository snapshot worker failed", source))?
    .map(Json)
    .map_err(WebApiError::from)
}

/// Reads one commit and its changed paths on a blocking worker.
pub async fn get_commit(
    State(app_state): State<AppState>,
    Path(path): Path<RepositoryCommitPath>,
) -> Result<Json<GetRepositoryCommitResponse>, WebApiError> {
    let backend = app_state.backend().clone();
    tokio::task::spawn_blocking(move || {
        backend.get_repository_commit(GetRepositoryCommitRequest {
            project_id: path.project_id,
            commit_id: path.commit_id,
        })
    })
    .await
    .map_err(|source| WebApiError::internal("repository commit worker failed", source))?
    .map(Json)
    .map_err(WebApiError::from)
}

/// Reads a historical commit patch on a blocking worker only after a file is opened.
pub async fn get_commit_diff(
    State(app_state): State<AppState>,
    Path(path): Path<RepositoryCommitPath>,
    Query(query): Query<RepositoryCommitDiffQuery>,
) -> Result<Json<GetRepositoryCommitDiffResponse>, WebApiError> {
    let backend = app_state.backend().clone();
    tokio::task::spawn_blocking(move || {
        backend.get_repository_commit_diff(GetRepositoryCommitDiffRequest {
            project_id: path.project_id,
            commit_id: path.commit_id,
            parent_commit_id: query.parent_commit_id,
            path: query.path,
        })
    })
    .await
    .map_err(|source| WebApiError::internal("repository commit diff worker failed", source))?
    .map(Json)
    .map_err(WebApiError::from)
}

/// Reads the current main checkout patch on a blocking worker.
pub async fn get_working_tree_diff(
    State(app_state): State<AppState>,
    Path(path): Path<RepositoryProjectPath>,
) -> Result<Json<GetRepositoryWorkingTreeDiffResponse>, WebApiError> {
    let backend = app_state.backend().clone();
    tokio::task::spawn_blocking(move || {
        backend.get_repository_working_tree_diff(GetRepositoryWorkingTreeDiffRequest {
            project_id: path.project_id,
        })
    })
    .await
    .map_err(|source| WebApiError::internal("repository working tree diff worker failed", source))?
    .map(Json)
    .map_err(WebApiError::from)
}

/// Creates a local repository branch on a blocking worker.
pub async fn create_branch(
    State(app_state): State<AppState>,
    Path(path): Path<RepositoryProjectPath>,
    Json(body): Json<RepositoryBranchBody>,
) -> Result<Json<CreateRepositoryBranchResponse>, WebApiError> {
    let backend = app_state.backend().clone();
    tokio::task::spawn_blocking(move || {
        backend.create_repository_branch(CreateRepositoryBranchRequest {
            project_id: path.project_id,
            branch_name: body.branch_name,
        })
    })
    .await
    .map_err(|source| WebApiError::internal("repository branch creation worker failed", source))?
    .map(Json)
    .map_err(WebApiError::from)
}

/// Checks out an existing repository branch on a blocking worker after the backend's safety checks.
pub async fn checkout_branch(
    State(app_state): State<AppState>,
    Path(path): Path<RepositoryProjectPath>,
    Json(body): Json<RepositoryBranchBody>,
) -> Result<Json<CheckoutRepositoryBranchResponse>, WebApiError> {
    let backend = app_state.backend().clone();
    tokio::task::spawn_blocking(move || {
        backend.checkout_repository_branch(CheckoutRepositoryBranchRequest {
            project_id: path.project_id,
            branch_name: body.branch_name,
        })
    })
    .await
    .map_err(|source| WebApiError::internal("repository branch checkout worker failed", source))?
    .map(Json)
    .map_err(WebApiError::from)
}

/// Fetches remote repository refs on a blocking worker.
pub async fn fetch(
    State(app_state): State<AppState>,
    Path(path): Path<RepositoryProjectPath>,
) -> Result<Json<FetchRepositoryResponse>, WebApiError> {
    let backend = app_state.backend().clone();
    tokio::task::spawn_blocking(move || {
        backend.fetch_repository(FetchRepositoryRequest {
            project_id: path.project_id,
        })
    })
    .await
    .map_err(|source| WebApiError::internal("repository fetch worker failed", source))?
    .map(Json)
    .map_err(WebApiError::from)
}

/// Pulls the main repository branch with the selected integration strategy on a blocking worker.
pub async fn pull(
    State(app_state): State<AppState>,
    Path(path): Path<RepositoryProjectPath>,
    Json(body): Json<RepositoryPullBody>,
) -> Result<Json<PullRepositoryResponse>, WebApiError> {
    let backend = app_state.backend().clone();
    tokio::task::spawn_blocking(move || {
        backend.pull_repository(PullRepositoryRequest {
            project_id: path.project_id,
            strategy: body.strategy,
        })
    })
    .await
    .map_err(|source| WebApiError::internal("repository pull worker failed", source))?
    .map(Json)
    .map_err(WebApiError::from)
}

/// Continues or aborts the active merge/rebase on a blocking worker.
pub async fn resolve_sync(
    State(app_state): State<AppState>,
    Path(path): Path<RepositoryProjectPath>,
    Json(body): Json<RepositorySyncBody>,
) -> Result<Json<ResolveRepositorySyncResponse>, WebApiError> {
    let backend = app_state.backend().clone();
    tokio::task::spawn_blocking(move || {
        backend.resolve_repository_sync(ResolveRepositorySyncRequest {
            project_id: path.project_id,
            action: body.action,
        })
    })
    .await
    .map_err(|source| WebApiError::internal("repository sync resolution worker failed", source))?
    .map(Json)
    .map_err(WebApiError::from)
}

/// Selects and stages one side of a conflicted path on a blocking worker.
pub async fn resolve_conflict(
    State(app_state): State<AppState>,
    Path(path): Path<RepositoryProjectPath>,
    Json(body): Json<RepositoryConflictBody>,
) -> Result<Json<ResolveRepositoryConflictResponse>, WebApiError> {
    let backend = app_state.backend().clone();
    tokio::task::spawn_blocking(move || {
        backend.resolve_repository_conflict(ResolveRepositoryConflictRequest {
            project_id: path.project_id,
            path: body.path,
            side: body.side,
        })
    })
    .await
    .map_err(|source| WebApiError::internal("repository conflict worker failed", source))?
    .map(Json)
    .map_err(WebApiError::from)
}

/// Pushes the checked-out main branch on a blocking worker.
pub async fn push_branch(
    State(app_state): State<AppState>,
    Path(path): Path<RepositoryProjectPath>,
) -> Result<Json<PushRepositoryBranchResponse>, WebApiError> {
    let backend = app_state.backend().clone();
    tokio::task::spawn_blocking(move || {
        backend.push_repository_branch(PushRepositoryBranchRequest {
            project_id: path.project_id,
        })
    })
    .await
    .map_err(|source| WebApiError::internal("repository push worker failed", source))?
    .map(Json)
    .map_err(WebApiError::from)
}

/// Stages selected main-checkout changes on a blocking worker.
pub async fn stage_changes(
    State(app_state): State<AppState>,
    Path(path): Path<RepositoryProjectPath>,
    Json(body): Json<RepositoryChangesBody>,
) -> Result<Json<StageRepositoryChangesResponse>, WebApiError> {
    let backend = app_state.backend().clone();
    tokio::task::spawn_blocking(move || {
        backend.stage_repository_changes(StageRepositoryChangesRequest {
            project_id: path.project_id,
            selection: body.selection,
        })
    })
    .await
    .map_err(|source| WebApiError::internal("repository staging worker failed", source))?
    .map(Json)
    .map_err(WebApiError::from)
}

/// Unstages selected main-checkout changes on a blocking worker.
pub async fn unstage_changes(
    State(app_state): State<AppState>,
    Path(path): Path<RepositoryProjectPath>,
    Json(body): Json<RepositoryChangesBody>,
) -> Result<Json<UnstageRepositoryChangesResponse>, WebApiError> {
    let backend = app_state.backend().clone();
    tokio::task::spawn_blocking(move || {
        backend.unstage_repository_changes(UnstageRepositoryChangesRequest {
            project_id: path.project_id,
            selection: body.selection,
        })
    })
    .await
    .map_err(|source| WebApiError::internal("repository unstaging worker failed", source))?
    .map(Json)
    .map_err(WebApiError::from)
}

/// Commits staged main-checkout changes on a blocking worker.
pub async fn commit_changes(
    State(app_state): State<AppState>,
    Path(path): Path<RepositoryProjectPath>,
    Json(body): Json<RepositoryCommitBody>,
) -> Result<Json<CommitRepositoryChangesResponse>, WebApiError> {
    let backend = app_state.backend().clone();
    tokio::task::spawn_blocking(move || {
        backend.commit_repository_changes(CommitRepositoryChangesRequest {
            project_id: path.project_id,
            message: body.message,
        })
    })
    .await
    .map_err(|source| WebApiError::internal("repository commit worker failed", source))?
    .map(Json)
    .map_err(WebApiError::from)
}
