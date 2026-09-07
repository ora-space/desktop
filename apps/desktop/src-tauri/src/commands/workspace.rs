//! Desktop workspace operations.

use super::run_backend;
use crate::{error::CommandError, state::DesktopState};
use ora_backend::{BackendError, WorkspaceApi};
use ora_contracts::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::State;

backend_command!(
    list_workspaces,
    ListWorkspacesRequest,
    ListWorkspacesResponse,
    workspaces.list,
    "Lists workspaces through the shared Backend."
);

backend_command!(
    get_workspace_diff,
    GetWorkspaceDiffRequest,
    GetWorkspaceDiffResponse,
    workspaces.get_diff,
    "Reads one workspace diff through the shared Backend."
);
backend_command!(
    commit_workspace_changes,
    CommitWorkspaceChangesRequest,
    CommitWorkspaceChangesResponse,
    workspaces.commit_changes,
    "Commits one workspace checkout through the shared Backend."
);
backend_command!(
    push_workspace_branch,
    PushWorkspaceBranchRequest,
    PushWorkspaceBranchResponse,
    workspaces.push_branch,
    "Pushes one workspace checkout's branch through the shared Backend."
);

/// Carries the empty request used to read the active worktree root.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetWorktreeRootRequest {}

/// Returns the active worktree creation root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetWorktreeRootResponse {
    pub worktree_root: String,
}

/// Carries a user-selected worktree creation root into the Desktop configuration command.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetWorktreeRootRequest {
    pub worktree_root: String,
}

/// Confirms the active worktree root after a successful configuration update.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetWorktreeRootResponse {
    pub worktree_root: String,
}

/// Identifies the task whose backing git worktree directory should be resolved.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveTaskCwdRequest {
    pub task_id: String,
}

/// Returns the absolute working directory that backs the requested task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveTaskCwdResponse {
    pub path: String,
}

/// Identifies the Workspace whose local directory should be resolved.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveWorkspaceCwdRequest {
    pub workspace_id: String,
}

/// Returns the absolute local directory backing a Workspace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveWorkspaceCwdResponse {
    pub path: String,
}

/// Renders an absolute path with the host OS's native separators.
///
/// Git reports worktree paths with forward slashes on every platform, so this keeps
/// both the copied text and the opened target reading naturally on Windows while
/// leaving already-native paths untouched on macOS.
fn to_native_path_string(path: &std::path::Path) -> String {
    let rendered = path.to_string_lossy().into_owned();
    #[cfg(target_os = "windows")]
    let rendered = rendered.replace('/', "\\");
    rendered
}

/// Resolves the on-disk git worktree directory for one task, live, off the API surface.
#[tauri::command]
pub async fn resolve_task_cwd(
    state: State<'_, DesktopState>,
    request: ResolveTaskCwdRequest,
) -> Result<ResolveTaskCwdResponse, CommandError> {
    run_backend(
        "resolve_task_cwd",
        state.backend.workspaces(),
        request,
        resolve_task_cwd_backend,
    )
    .await
}

/// Renders the authoritative task location in the native platform's expected path spelling.
fn resolve_task_cwd_backend(
    backend: &WorkspaceApi,
    request: ResolveTaskCwdRequest,
) -> Result<ResolveTaskCwdResponse, BackendError> {
    backend
        .resolve_task_cwd(&request.task_id)
        .map(|path| ResolveTaskCwdResponse {
            path: to_native_path_string(&path),
        })
}

/// Resolves a Workspace's local directory off the API surface.
#[tauri::command]
pub async fn resolve_workspace_cwd(
    state: State<'_, DesktopState>,
    request: ResolveWorkspaceCwdRequest,
) -> Result<ResolveWorkspaceCwdResponse, CommandError> {
    run_backend(
        "resolve_workspace_cwd",
        state.backend.workspaces(),
        request,
        resolve_workspace_cwd_backend,
    )
    .await
}

/// Resolves a Workspace's local directory through the composed backend.
fn resolve_workspace_cwd_backend(
    backend: &WorkspaceApi,
    request: ResolveWorkspaceCwdRequest,
) -> Result<ResolveWorkspaceCwdResponse, BackendError> {
    backend
        .resolve_workspace_cwd(&request.workspace_id)
        .map(|path| ResolveWorkspaceCwdResponse {
            path: to_native_path_string(&path),
        })
}

/// Reads the active worktree root through Backend's SQLite-backed configuration.
#[tauri::command]
pub async fn get_worktree_root(
    state: State<'_, DesktopState>,
    request: GetWorktreeRootRequest,
) -> Result<GetWorktreeRootResponse, CommandError> {
    run_backend(
        "get_worktree_root",
        state.backend.workspaces(),
        request,
        get_worktree_root_backend,
    )
    .await
}

/// Keeps native response formatting outside the workspace configuration interface.
fn get_worktree_root_backend(
    backend: &WorkspaceApi,
    _request: GetWorktreeRootRequest,
) -> Result<GetWorktreeRootResponse, BackendError> {
    backend.worktree_root().map(|root| GetWorktreeRootResponse {
        worktree_root: root.to_string_lossy().into_owned(),
    })
}

/// Persists a new creation root without interrupting in-flight task creation.
#[tauri::command]
pub async fn set_worktree_root(
    state: State<'_, DesktopState>,
    request: SetWorktreeRootRequest,
) -> Result<SetWorktreeRootResponse, CommandError> {
    run_backend(
        "set_worktree_root",
        state.backend.workspaces(),
        request,
        set_worktree_root_backend,
    )
    .await
}

/// Publishes the selected path only after the workspace module accepts and persists it.
fn set_worktree_root_backend(
    backend: &WorkspaceApi,
    request: SetWorktreeRootRequest,
) -> Result<SetWorktreeRootResponse, BackendError> {
    let worktree_root = PathBuf::from(request.worktree_root);
    backend.set_worktree_root(worktree_root.clone())?;
    Ok(SetWorktreeRootResponse {
        worktree_root: worktree_root.to_string_lossy().into_owned(),
    })
}
