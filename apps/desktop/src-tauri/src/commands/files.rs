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

/// Creates one empty file or directory in the selected task workspace.
#[tauri::command]
pub async fn create_workspace_entry(
    state: State<'_, DesktopState>,
    request: CreateWorkspaceEntryRequest,
) -> Result<WorkspaceEntry, CommandError> {
    run_backend(
        "create_workspace_entry",
        (state.backend.workspaces(), state.workspace_files.clone()),
        request,
        |(backend, files), request| create_workspace_entry_backend(backend, files, request),
    )
    .await
}

/// Creates one empty file or directory in the selected project checkout root.
#[tauri::command]
pub async fn create_project_entry(
    state: State<'_, DesktopState>,
    request: CreateProjectEntryRequest,
) -> Result<WorkspaceEntry, CommandError> {
    run_backend(
        "create_project_entry",
        (state.backend.workspaces(), state.workspace_files.clone()),
        request,
        |(backend, files), request| create_project_entry_backend(backend, files, request),
    )
    .await
}

/// Resolves a task workspace and creates the requested relative file or directory.
fn create_workspace_entry_backend(
    backend: &WorkspaceApi,
    workspace_files: &WorkspaceFileApi,
    request: CreateWorkspaceEntryRequest,
) -> Result<WorkspaceEntry, BackendError> {
    let root = backend.resolve_task_cwd(&request.task_id)?;
    workspace_files
        .create_entry(&root, Path::new(&request.path), request.kind)
        .map_err(workspace_file_backend_error)
}

/// Resolves a project checkout and creates the requested relative file or directory.
fn create_project_entry_backend(
    backend: &WorkspaceApi,
    workspace_files: &WorkspaceFileApi,
    request: CreateProjectEntryRequest,
) -> Result<WorkspaceEntry, BackendError> {
    let root = backend.resolve_project_cwd(&request.project_id)?;
    workspace_files
        .create_entry(&root, Path::new(&request.path), request.kind)
        .map_err(workspace_file_backend_error)
}

/// Copies one empty-or-existing path onto a missing destination in the selected task workspace.
#[tauri::command]
pub async fn copy_workspace_entry(
    state: State<'_, DesktopState>,
    request: CopyWorkspaceEntryRequest,
) -> Result<WorkspaceEntry, CommandError> {
    run_backend(
        "copy_workspace_entry",
        (state.backend.workspaces(), state.workspace_files.clone()),
        request,
        |(backend, files), request| copy_workspace_entry_backend(backend, files, request),
    )
    .await
}

/// Copies one path onto a missing destination in the selected project checkout root.
#[tauri::command]
pub async fn copy_project_entry(
    state: State<'_, DesktopState>,
    request: CopyProjectEntryRequest,
) -> Result<WorkspaceEntry, CommandError> {
    run_backend(
        "copy_project_entry",
        (state.backend.workspaces(), state.workspace_files.clone()),
        request,
        |(backend, files), request| copy_project_entry_backend(backend, files, request),
    )
    .await
}

/// Moves or renames one path in the selected task workspace.
#[tauri::command]
pub async fn move_workspace_entry(
    state: State<'_, DesktopState>,
    request: MoveWorkspaceEntryRequest,
) -> Result<WorkspaceEntry, CommandError> {
    run_backend(
        "move_workspace_entry",
        (state.backend.workspaces(), state.workspace_files.clone()),
        request,
        |(backend, files), request| move_workspace_entry_backend(backend, files, request),
    )
    .await
}

/// Moves or renames one path in the selected project checkout root.
#[tauri::command]
pub async fn move_project_entry(
    state: State<'_, DesktopState>,
    request: MoveProjectEntryRequest,
) -> Result<WorkspaceEntry, CommandError> {
    run_backend(
        "move_project_entry",
        (state.backend.workspaces(), state.workspace_files.clone()),
        request,
        |(backend, files), request| move_project_entry_backend(backend, files, request),
    )
    .await
}

/// Resolves a task workspace and copies the requested relative path.
fn copy_workspace_entry_backend(
    backend: &WorkspaceApi,
    workspace_files: &WorkspaceFileApi,
    request: CopyWorkspaceEntryRequest,
) -> Result<WorkspaceEntry, BackendError> {
    let root = backend.resolve_task_cwd(&request.task_id)?;
    workspace_files
        .copy_entry(&root, Path::new(&request.from), Path::new(&request.path))
        .map_err(workspace_file_backend_error)
}

/// Resolves a project checkout and copies the requested relative path.
fn copy_project_entry_backend(
    backend: &WorkspaceApi,
    workspace_files: &WorkspaceFileApi,
    request: CopyProjectEntryRequest,
) -> Result<WorkspaceEntry, BackendError> {
    let root = backend.resolve_project_cwd(&request.project_id)?;
    workspace_files
        .copy_entry(&root, Path::new(&request.from), Path::new(&request.path))
        .map_err(workspace_file_backend_error)
}

/// Resolves a task workspace and moves or renames the requested relative path.
fn move_workspace_entry_backend(
    backend: &WorkspaceApi,
    workspace_files: &WorkspaceFileApi,
    request: MoveWorkspaceEntryRequest,
) -> Result<WorkspaceEntry, BackendError> {
    let root = backend.resolve_task_cwd(&request.task_id)?;
    workspace_files
        .move_entry(&root, Path::new(&request.from), Path::new(&request.path))
        .map_err(workspace_file_backend_error)
}

/// Resolves a project checkout and moves or renames the requested relative path.
fn move_project_entry_backend(
    backend: &WorkspaceApi,
    workspace_files: &WorkspaceFileApi,
    request: MoveProjectEntryRequest,
) -> Result<WorkspaceEntry, BackendError> {
    let root = backend.resolve_project_cwd(&request.project_id)?;
    workspace_files
        .move_entry(&root, Path::new(&request.from), Path::new(&request.path))
        .map_err(workspace_file_backend_error)
}

/// Deletes one path in the selected task workspace.
#[tauri::command]
pub async fn delete_workspace_entry(
    state: State<'_, DesktopState>,
    request: DeleteWorkspaceEntryRequest,
) -> Result<DeleteFileSystemEntryResponse, CommandError> {
    run_backend(
        "delete_workspace_entry",
        (state.backend.workspaces(), state.workspace_files.clone()),
        request,
        |(backend, files), request| delete_workspace_entry_backend(backend, files, request),
    )
    .await
}

/// Deletes one path in the selected project checkout root.
#[tauri::command]
pub async fn delete_project_entry(
    state: State<'_, DesktopState>,
    request: DeleteProjectEntryRequest,
) -> Result<DeleteFileSystemEntryResponse, CommandError> {
    run_backend(
        "delete_project_entry",
        (state.backend.workspaces(), state.workspace_files.clone()),
        request,
        |(backend, files), request| delete_project_entry_backend(backend, files, request),
    )
    .await
}

/// Resolves a task workspace and deletes the requested relative path.
fn delete_workspace_entry_backend(
    backend: &WorkspaceApi,
    workspace_files: &WorkspaceFileApi,
    request: DeleteWorkspaceEntryRequest,
) -> Result<DeleteFileSystemEntryResponse, BackendError> {
    let root = backend.resolve_task_cwd(&request.task_id)?;
    workspace_files
        .delete_entry(&root, Path::new(&request.path))
        .map_err(workspace_file_backend_error)?;
    Ok(DeleteFileSystemEntryResponse {})
}

/// Resolves a project checkout and deletes the requested relative path.
fn delete_project_entry_backend(
    backend: &WorkspaceApi,
    workspace_files: &WorkspaceFileApi,
    request: DeleteProjectEntryRequest,
) -> Result<DeleteFileSystemEntryResponse, BackendError> {
    let root = backend.resolve_project_cwd(&request.project_id)?;
    workspace_files
        .delete_entry(&root, Path::new(&request.path))
        .map_err(workspace_file_backend_error)?;
    Ok(DeleteFileSystemEntryResponse {})
}
