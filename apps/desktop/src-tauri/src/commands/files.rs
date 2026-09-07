//! Desktop files operations.

use super::{run_async_backend, run_backend};
use crate::workspace_files::{WorkspaceFileApi, workspace_file_backend_error};
use crate::{error::CommandError, state::DesktopState};
use ora_backend::{BackendError, WorkspaceApi};
use ora_contracts::*;
use std::path::Path;
use tauri::State;

/// Resolves a task-owned checkout before creating the native watcher at the filesystem seam.
pub(super) async fn start_workspace_watch(
    state: State<'_, DesktopState>,
    request: WatchWorkspaceRequest,
    context: super::stream::StreamStart,
) -> Result<(), CommandError> {
    let backend = state.backend.workspaces();
    let files = state.workspace_files.clone();
    context
        .watch(async move {
            tauri::async_runtime::spawn_blocking(move || {
                let root = backend.resolve_task_cwd(&request.task_id)?;
                files.watch(&root).map_err(workspace_file_backend_error)
            })
            .await
            .map_err(|error| {
                BackendError::internal("Desktop workspace watcher setup failed", error)
            })?
        })
        .await
}

/// Resolves a project-owned checkout while the shared context handles transport teardown.
pub(super) async fn start_project_watch(
    state: State<'_, DesktopState>,
    request: WatchProjectRequest,
    context: super::stream::StreamStart,
) -> Result<(), CommandError> {
    let backend = state.backend.workspaces();
    let files = state.workspace_files.clone();
    context
        .watch(async move {
            tauri::async_runtime::spawn_blocking(move || {
                let root = backend.resolve_project_cwd(&request.project_id)?;
                files.watch(&root).map_err(workspace_file_backend_error)
            })
            .await
            .map_err(|error| {
                BackendError::internal("Desktop project watcher setup failed", error)
            })?
        })
        .await
}

/// Lists one immediate directory in the selected task workspace.
#[tauri::command]
pub async fn list_workspace_directory(
    state: State<'_, DesktopState>,
    request: ListWorkspaceDirectoryRequest,
) -> Result<ListWorkspaceDirectoryResponse, CommandError> {
    run_backend(
        "list_workspace_directory",
        (state.backend.workspaces(), state.workspace_files.clone()),
        request,
        |(backend, files), request| list_workspace_directory_backend(backend, files, request),
    )
    .await
}

/// Reads one bounded UTF-8 file in the selected task workspace.
#[tauri::command]
pub async fn read_workspace_file(
    state: State<'_, DesktopState>,
    request: ReadWorkspaceFileRequest,
) -> Result<ReadWorkspaceFileResponse, CommandError> {
    run_backend(
        "read_workspace_file",
        (state.backend.workspaces(), state.workspace_files.clone()),
        request,
        |(backend, files), request| read_workspace_file_backend(backend, files, request),
    )
    .await
}

/// Searches the selected task workspace with bounded ripgrep output.
#[tauri::command]
pub async fn search_workspace(
    state: State<'_, DesktopState>,
    request: SearchWorkspaceRequest,
) -> Result<SearchWorkspaceResponse, CommandError> {
    let backend = state.backend.workspaces();
    let workspace_files = state.workspace_files.clone();
    let task_id = request.task_id;
    let query = request.query;
    let kind = request.kind;
    run_async_backend("search_workspace", async move {
        let root = tauri::async_runtime::spawn_blocking(move || backend.resolve_task_cwd(&task_id))
            .await
            .map_err(|source| {
                BackendError::internal("Desktop workspace root resolution failed", source)
            })??;
        workspace_files
            .search(&root, &query, kind)
            .await
            .map_err(workspace_file_backend_error)
    })
    .await
}

/// Resolves a task workspace and lists the requested relative directory.
fn list_workspace_directory_backend(
    backend: &WorkspaceApi,
    workspace_files: &WorkspaceFileApi,
    request: ListWorkspaceDirectoryRequest,
) -> Result<ListWorkspaceDirectoryResponse, BackendError> {
    let root = backend.resolve_task_cwd(&request.task_id)?;
    let path = request
        .path
        .as_deref()
        .map(Path::new)
        .unwrap_or_else(|| Path::new(""));
    workspace_files
        .list_directory(&root, path)
        .map_err(workspace_file_backend_error)
}

/// Resolves a task workspace and reads the requested relative file.
fn read_workspace_file_backend(
    backend: &WorkspaceApi,
    workspace_files: &WorkspaceFileApi,
    request: ReadWorkspaceFileRequest,
) -> Result<ReadWorkspaceFileResponse, BackendError> {
    let root = backend.resolve_task_cwd(&request.task_id)?;
    workspace_files
        .read_file(&root, Path::new(&request.path))
        .map_err(workspace_file_backend_error)
}

/// Lists one immediate directory in the selected project checkout root.
#[tauri::command]
pub async fn list_project_directory(
    state: State<'_, DesktopState>,
    request: ListProjectDirectoryRequest,
) -> Result<ListWorkspaceDirectoryResponse, CommandError> {
    run_backend(
        "list_project_directory",
        (state.backend.workspaces(), state.workspace_files.clone()),
        request,
        |(backend, files), request| list_project_directory_backend(backend, files, request),
    )
    .await
}

/// Reads one bounded UTF-8 file in the selected project checkout root.
#[tauri::command]
pub async fn read_project_file(
    state: State<'_, DesktopState>,
    request: ReadProjectFileRequest,
) -> Result<ReadWorkspaceFileResponse, CommandError> {
    run_backend(
        "read_project_file",
        (state.backend.workspaces(), state.workspace_files.clone()),
        request,
        |(backend, files), request| read_project_file_backend(backend, files, request),
    )
    .await
}

/// Searches the selected project checkout with bounded ripgrep output.
#[tauri::command]
pub async fn search_project(
    state: State<'_, DesktopState>,
    request: SearchProjectRequest,
) -> Result<SearchWorkspaceResponse, CommandError> {
    let backend = state.backend.workspaces();
    let workspace_files = state.workspace_files.clone();
    let project_id = request.project_id;
    let query = request.query;
    let kind = request.kind;
    run_async_backend("search_project", async move {
        let root =
            tauri::async_runtime::spawn_blocking(move || backend.resolve_project_cwd(&project_id))
                .await
                .map_err(|source| {
                    BackendError::internal("Desktop workspace location resolution failed", source)
                })??;
        workspace_files
            .search(&root, &query, kind)
            .await
            .map_err(workspace_file_backend_error)
    })
    .await
}

/// Resolves a project checkout and lists the requested relative directory.
fn list_project_directory_backend(
    backend: &WorkspaceApi,
    workspace_files: &WorkspaceFileApi,
    request: ListProjectDirectoryRequest,
) -> Result<ListWorkspaceDirectoryResponse, BackendError> {
    let root = backend.resolve_project_cwd(&request.project_id)?;
    let path = request
        .path
        .as_deref()
        .map(Path::new)
        .unwrap_or_else(|| Path::new(""));
    workspace_files
        .list_directory(&root, path)
        .map_err(workspace_file_backend_error)
}

/// Resolves a project checkout and reads the requested relative file.
fn read_project_file_backend(
    backend: &WorkspaceApi,
    workspace_files: &WorkspaceFileApi,
    request: ReadProjectFileRequest,
) -> Result<ReadWorkspaceFileResponse, BackendError> {
    let root = backend.resolve_project_cwd(&request.project_id)?;
    workspace_files
        .read_file(&root, Path::new(&request.path))
        .map_err(workspace_file_backend_error)
}
