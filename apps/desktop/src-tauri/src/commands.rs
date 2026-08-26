use crate::config::validate_worktree_root;
use crate::error::{CommandError, desktop_config_backend_error};
use crate::state::DesktopState;
use crate::stream_forwarding::{forward_contract_stream, forward_workspace_watch};
use crate::workspace_files::{WorkspaceFileApi, workspace_file_backend_error};
use ora_backend::{Backend, BackendError, RequestLifecycle, UuidRequestIdGenerator};
use ora_contracts::*;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::State;
use tauri::ipc::Channel;
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

/// Executes one synchronous backend operation on the runtime's blocking executor.
async fn run_backend<Request, Response>(
    operation_name: &'static str,
    backend: Backend,
    request: Request,
    operation: fn(&Backend, Request) -> Result<Response, BackendError>,
) -> Result<Response, CommandError>
where
    Request: Send + 'static,
    Response: Send + 'static,
{
    let lifecycle = RequestLifecycle::start(operation_name, &UuidRequestIdGenerator);
    let request_span =
        ora_logging::span_with_request_id("tauri_command", &lifecycle.request_id().to_string());
    let blocking_span = request_span.clone();
    async move {
        let result = match tauri::async_runtime::spawn_blocking(move || {
            blocking_span.in_scope(|| operation(&backend, request))
        })
        .await
        {
            Ok(result) => result,
            Err(source) => Err(BackendError::internal(
                "Desktop command execution failed",
                source,
            )),
        };

        match result {
            Ok(response) => {
                lifecycle.complete_success();
                Ok(response)
            }
            Err(error) => Err(CommandError::from_backend_with_lifecycle(error, &lifecycle)),
        }
    }
    .instrument(request_span)
    .await
}

/// Executes one filesystem operation on the blocking executor while preserving the task root.
async fn run_workspace_backend<Request, Response>(
    operation_name: &'static str,
    backend: Backend,
    workspace_files: Arc<WorkspaceFileApi>,
    request: Request,
    operation: fn(&Backend, &WorkspaceFileApi, Request) -> Result<Response, BackendError>,
) -> Result<Response, CommandError>
where
    Request: Send + 'static,
    Response: Send + 'static,
{
    let lifecycle = RequestLifecycle::start(operation_name, &UuidRequestIdGenerator);
    let request_span =
        ora_logging::span_with_request_id("tauri_command", &lifecycle.request_id().to_string());
    let blocking_span = request_span.clone();
    async move {
        let result = match tauri::async_runtime::spawn_blocking(move || {
            blocking_span.in_scope(|| operation(&backend, &workspace_files, request))
        })
        .await
        {
            Ok(result) => result,
            Err(source) => Err(BackendError::internal(
                "Desktop workspace command execution failed",
                source,
            )),
        };

        match result {
            Ok(response) => {
                lifecycle.complete_success();
                Ok(response)
            }
            Err(error) => Err(CommandError::from_backend_with_lifecycle(error, &lifecycle)),
        }
    }
    .instrument(request_span)
    .await
}

async fn run_async_backend<Response, Call>(
    operation_name: &'static str,
    call: Call,
) -> Result<Response, CommandError>
where
    Call: Future<Output = Result<Response, BackendError>>,
{
    let lifecycle = RequestLifecycle::start(operation_name, &UuidRequestIdGenerator);
    let request_span =
        ora_logging::span_with_request_id("tauri_command", &lifecycle.request_id().to_string());
    async move {
        match call.await {
            Ok(response) => {
                lifecycle.complete_success();
                Ok(response)
            }
            Err(error) => Err(CommandError::from_backend_with_lifecycle(error, &lifecycle)),
        }
    }
    .instrument(request_span)
    .await
}

macro_rules! backend_command {
    ($name:ident, $request:ty, $response:ty, $operation:ident, $doc:literal) => {
        #[doc = $doc]
        #[tauri::command]
        pub async fn $name(
            state: State<'_, DesktopState>,
            request: $request,
        ) -> Result<$response, CommandError> {
            run_backend(
                stringify!($name),
                state.backend.clone(),
                request,
                Backend::$operation,
            )
            .await
        }
    };
}

macro_rules! async_backend_command {
    ($name:ident, $request:ty, $response:ty, $operation:ident, $doc:literal) => {
        #[doc = $doc]
        #[tauri::command]
        pub async fn $name(
            state: State<'_, DesktopState>,
            request: $request,
        ) -> Result<$response, CommandError> {
            run_async_backend(stringify!($name), state.backend.$operation(request)).await
        }
    };
}

/// Carries a user-selected destination and serialized workflow definition for export.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteWorkflowExportRequest {
    path: PathBuf,
    content: String,
}

/// Writes a workflow export after the desktop save dialog has selected its exact destination.
#[tauri::command]
pub async fn write_workflow_export(request: WriteWorkflowExportRequest) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || std::fs::write(request.path, request.content))
        .await
        .map_err(|error| format!("workflow export task failed: {error}"))?
        .map_err(|error| format!("workflow export write failed: {error}"))
}

// =============================================================================
// project
// =============================================================================

backend_command!(
    create_project,
    CreateProjectRequest,
    CreateProjectResponse,
    create_project,
    "Creates one project through the shared Backend."
);
backend_command!(
    get_project,
    GetProjectRequest,
    GetProjectResponse,
    get_project,
    "Gets one project through the shared Backend."
);
backend_command!(
    list_projects,
    ListProjectsRequest,
    ListProjectsResponse,
    list_projects,
    "Lists projects through the shared Backend."
);
backend_command!(
    list_workspaces,
    ListWorkspacesRequest,
    ListWorkspacesResponse,
    list_workspaces,
    "Lists workspaces through the shared Backend."
);
backend_command!(
    list_project_branches,
    ListProjectBranchesRequest,
    ListProjectBranchesResponse,
    list_project_branches,
    "Lists local branches for one project through the shared Backend."
);
backend_command!(
    update_project,
    UpdateProjectRequest,
    UpdateProjectResponse,
    update_project,
    "Updates one project through the shared Backend."
);
/// Deletes one project through the shared Backend.
///
/// Not a `backend_command!` because deleting also returns the warm provider
/// sessions the project owned, which is asynchronous.
#[tauri::command]
pub async fn delete_project(
    state: State<'_, DesktopState>,
    request: DeleteProjectRequest,
) -> Result<DeleteProjectResponse, CommandError> {
    state
        .backend
        .delete_project(request)
        .await
        .map_err(CommandError::from)
}

// =============================================================================
// task
// =============================================================================

backend_command!(
    create_task,
    CreateTaskRequest,
    CreateTaskResponse,
    create_task,
    "Creates one task through the shared Backend."
);
backend_command!(
    get_task,
    GetTaskRequest,
    GetTaskResponse,
    get_task,
    "Gets one task through the shared Backend."
);
backend_command!(
    list_tasks,
    ListTasksRequest,
    ListTasksResponse,
    list_tasks,
    "Lists tasks through the shared Backend."
);
backend_command!(
    update_task,
    UpdateTaskRequest,
    UpdateTaskResponse,
    update_task,
    "Updates one task through the shared Backend."
);
/// Deletes one task through the shared Backend.
///
/// Not a `backend_command!` because deleting also returns the warm provider
/// session the Task owned, which is asynchronous.
#[tauri::command]
pub async fn delete_task(
    state: State<'_, DesktopState>,
    request: DeleteTaskRequest,
) -> Result<DeleteTaskResponse, CommandError> {
    state
        .backend
        .delete_task(request)
        .await
        .map_err(CommandError::from)
}
backend_command!(
    get_task_diff,
    GetTaskDiffRequest,
    GetTaskDiffResponse,
    get_task_diff,
    "Reads one task diff through the shared Backend."
);
backend_command!(
    commit_task_changes,
    CommitTaskChangesRequest,
    CommitTaskChangesResponse,
    commit_task_changes,
    "Commits one task worktree through the shared Backend."
);
backend_command!(
    push_task_branch,
    PushTaskBranchRequest,
    PushTaskBranchResponse,
    push_task_branch,
    "Pushes one task worktree branch through the shared Backend."
);

// =============================================================================
// fileSystem
// =============================================================================

/// Lists one immediate directory in the selected task workspace.
#[tauri::command]
pub async fn list_workspace_directory(
    state: State<'_, DesktopState>,
    request: ListWorkspaceDirectoryRequest,
) -> Result<ListWorkspaceDirectoryResponse, CommandError> {
    run_workspace_backend(
        "list_workspace_directory",
        state.backend.clone(),
        state.workspace_files.clone(),
        request,
        list_workspace_directory_backend,
    )
    .await
}

/// Reads one bounded UTF-8 file in the selected task workspace.
#[tauri::command]
pub async fn read_workspace_file(
    state: State<'_, DesktopState>,
    request: ReadWorkspaceFileRequest,
) -> Result<ReadWorkspaceFileResponse, CommandError> {
    run_workspace_backend(
        "read_workspace_file",
        state.backend.clone(),
        state.workspace_files.clone(),
        request,
        read_workspace_file_backend,
    )
    .await
}

/// Searches the selected task workspace with bounded ripgrep output.
#[tauri::command]
pub async fn search_workspace(
    state: State<'_, DesktopState>,
    request: SearchWorkspaceRequest,
) -> Result<SearchWorkspaceResponse, CommandError> {
    let backend = state.backend.clone();
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
    backend: &Backend,
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
    backend: &Backend,
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
    run_workspace_backend(
        "list_project_directory",
        state.backend.clone(),
        state.workspace_files.clone(),
        request,
        list_project_directory_backend,
    )
    .await
}

/// Reads one bounded UTF-8 file in the selected project checkout root.
#[tauri::command]
pub async fn read_project_file(
    state: State<'_, DesktopState>,
    request: ReadProjectFileRequest,
) -> Result<ReadWorkspaceFileResponse, CommandError> {
    run_workspace_backend(
        "read_project_file",
        state.backend.clone(),
        state.workspace_files.clone(),
        request,
        read_project_file_backend,
    )
    .await
}

/// Searches the selected project checkout with bounded ripgrep output.
#[tauri::command]
pub async fn search_project(
    state: State<'_, DesktopState>,
    request: SearchProjectRequest,
) -> Result<SearchWorkspaceResponse, CommandError> {
    let backend = state.backend.clone();
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
    backend: &Backend,
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
    backend: &Backend,
    workspace_files: &WorkspaceFileApi,
    request: ReadProjectFileRequest,
) -> Result<ReadWorkspaceFileResponse, BackendError> {
    let root = backend.resolve_project_cwd(&request.project_id)?;
    workspace_files
        .read_file(&root, Path::new(&request.path))
        .map_err(workspace_file_backend_error)
}

// =============================================================================
// session
// =============================================================================

/// Returns the warm provider session backing one chat surface.
#[tauri::command]
pub async fn warm_session(
    state: State<'_, DesktopState>,
    request: WarmSessionRequest,
) -> Result<WarmSessionResponse, CommandError> {
    run_async_backend("warm_session", state.backend.warm_session(request)).await
}

/// Applies one configuration option to a warm or persisted session.
#[tauri::command]
pub async fn set_session_config(
    state: State<'_, DesktopState>,
    request: SetSessionConfigRequest,
) -> Result<SetSessionConfigResponse, CommandError> {
    run_async_backend(
        "set_session_config",
        state.backend.set_session_config(request),
    )
    .await
}

/// Persists one warm session against the Task that now owns it.
#[tauri::command]
pub async fn attach_session(
    state: State<'_, DesktopState>,
    request: AttachSessionRequest,
) -> Result<AttachSessionResponse, CommandError> {
    run_async_backend("attach_session", state.backend.attach_session(request)).await
}
backend_command!(
    get_session,
    GetSessionRequest,
    GetSessionResponse,
    get_session,
    "Gets one session through the shared Backend."
);
backend_command!(
    list_sessions,
    ListSessionsRequest,
    ListSessionsResponse,
    list_sessions,
    "Lists sessions through the shared Backend."
);
/// Routes one permission choice through the owning Session actor.
#[tauri::command]
pub async fn respond_to_session_permission(
    state: State<'_, DesktopState>,
    request: RespondToPermissionRequest,
) -> Result<RespondToPermissionResponse, CommandError> {
    run_async_backend(
        "respond_to_session_permission",
        state.backend.respond_to_session_permission(request),
    )
    .await
}

/// Stops one provider process while retaining the Ora session record.
#[tauri::command]
pub async fn stop_session(
    state: State<'_, DesktopState>,
    request: StopSessionRequest,
) -> Result<StopSessionResponse, CommandError> {
    run_async_backend("stop_session", state.backend.stop_session(request)).await
}

/// Cancels the active prompt without unloading the reusable session.
#[tauri::command]
pub async fn cancel_session_prompt(
    state: State<'_, DesktopState>,
    request: CancelSessionPromptRequest,
) -> Result<CancelSessionPromptResponse, CommandError> {
    run_backend(
        "cancel_session_prompt",
        state.backend.clone(),
        request,
        Backend::cancel_session_prompt,
    )
    .await
}

/// Moves one conversation onto a different agent CLI without changing its identity.
#[tauri::command]
pub async fn switch_session_agent(
    state: State<'_, DesktopState>,
    request: SwitchSessionAgentRequest,
) -> Result<SwitchSessionAgentResponse, CommandError> {
    run_async_backend(
        "switch_session_agent",
        state.backend.switch_session_agent(request),
    )
    .await
}

/// Returns a session whose history writes failed to a writable state.
#[tauri::command]
pub async fn resume_session_history(
    state: State<'_, DesktopState>,
    request: ResumeSessionHistoryRequest,
) -> Result<ResumeSessionHistoryResponse, CommandError> {
    run_async_backend(
        "resume_session_history",
        state.backend.resume_session_history(request),
    )
    .await
}

/// Stops the provider process before removing the Ora session record and its history.
#[tauri::command]
pub async fn delete_session(
    state: State<'_, DesktopState>,
    request: DeleteSessionRequest,
) -> Result<DeleteSessionResponse, CommandError> {
    run_async_backend("delete_session", state.backend.delete_session(request)).await
}

/// Renames one session through the shared Backend.
#[tauri::command]
pub async fn rename_session(
    state: State<'_, DesktopState>,
    request: RenameSessionRequest,
) -> Result<RenameSessionResponse, CommandError> {
    run_async_backend("rename_session", state.backend.rename_session(request)).await
}

/// Starts one typed Session stream and forwards private transport frames over a Tauri Channel.
#[tauri::command]
pub async fn stream_contract(
    state: State<'_, DesktopState>,
    operation_name: String,
    request: serde_json::Value,
    stream_call_id: String,
    on_event: Channel<serde_json::Value>,
) -> Result<(), CommandError> {
    let lifecycle = RequestLifecycle::start(
        format!("stream_contract:{operation_name}"),
        &UuidRequestIdGenerator,
    );
    let cancellation = CancellationToken::new();

    match operation_name.as_str() {
        "loadSession" => {
            let request =
                serde_json::from_value::<LoadSessionRequest>(request).map_err(|source| {
                    CommandError::from_backend_with_lifecycle(
                        BackendError::internal("failed to decode stream request", source),
                        &lifecycle,
                    )
                })?;
            let stream =
                state.backend.load_session(request).await.map_err(|error| {
                    CommandError::from_backend_with_lifecycle(error, &lifecycle)
                })?;
            register_contract_stream(&state, &stream_call_id, &cancellation)
                .map_err(|error| CommandError::from_backend_with_lifecycle(error, &lifecycle))?;
            let registry = state.stream_cancellations.clone();
            tauri::async_runtime::spawn(forward_contract_stream(
                stream,
                cancellation,
                stream_call_id,
                registry,
                on_event,
                lifecycle,
            ));
        }
        "promptSession" => {
            let request =
                serde_json::from_value::<PromptSessionRequest>(request).map_err(|source| {
                    CommandError::from_backend_with_lifecycle(
                        BackendError::internal("failed to decode stream request", source),
                        &lifecycle,
                    )
                })?;
            let stream = state
                .backend
                .prompt_session(request)
                .await
                .map_err(|error| CommandError::from_backend_with_lifecycle(error, &lifecycle))?;
            register_contract_stream(&state, &stream_call_id, &cancellation)
                .map_err(|error| CommandError::from_backend_with_lifecycle(error, &lifecycle))?;
            let registry = state.stream_cancellations.clone();
            tauri::async_runtime::spawn(forward_contract_stream(
                stream,
                cancellation,
                stream_call_id,
                registry,
                on_event,
                lifecycle,
            ));
        }
        "watchAppEvents" => {
            let stream = state.backend.watch_app_events();
            register_contract_stream(&state, &stream_call_id, &cancellation)
                .map_err(|error| CommandError::from_backend_with_lifecycle(error, &lifecycle))?;
            let registry = state.stream_cancellations.clone();
            tauri::async_runtime::spawn(forward_contract_stream(
                stream,
                cancellation,
                stream_call_id,
                registry,
                on_event,
                lifecycle,
            ));
        }
        "watchWorkspace" => {
            let request =
                serde_json::from_value::<WatchWorkspaceRequest>(request).map_err(|source| {
                    CommandError::from_backend_with_lifecycle(
                        BackendError::internal("failed to decode stream request", source),
                        &lifecycle,
                    )
                })?;
            let task_id = request.task_id;
            let backend = state.backend.clone();
            let root =
                tauri::async_runtime::spawn_blocking(move || backend.resolve_task_cwd(&task_id))
                    .await
                    .map_err(|source| {
                        CommandError::from_backend_with_lifecycle(
                            BackendError::internal(
                                "Desktop workspace root resolution failed",
                                source,
                            ),
                            &lifecycle,
                        )
                    })?
                    .map_err(|error| {
                        CommandError::from_backend_with_lifecycle(error, &lifecycle)
                    })?;
            start_workspace_watch(
                state,
                root,
                stream_call_id,
                on_event,
                lifecycle,
                cancellation,
            )
            .await?;
        }
        "watchProject" => {
            let request =
                serde_json::from_value::<WatchProjectRequest>(request).map_err(|source| {
                    CommandError::from_backend_with_lifecycle(
                        BackendError::internal("failed to decode stream request", source),
                        &lifecycle,
                    )
                })?;
            let project_id = request.project_id;
            let backend = state.backend.clone();
            let root = tauri::async_runtime::spawn_blocking(move || {
                backend.resolve_project_cwd(&project_id)
            })
            .await
            .map_err(|source| {
                CommandError::from_backend_with_lifecycle(
                    BackendError::internal("Desktop workspace location resolution failed", source),
                    &lifecycle,
                )
            })?
            .map_err(|error| CommandError::from_backend_with_lifecycle(error, &lifecycle))?;
            start_workspace_watch(
                state,
                root,
                stream_call_id,
                on_event,
                lifecycle,
                cancellation,
            )
            .await?;
        }
        "watchSpecs" => {
            return crate::spec_commands::start_watch(
                state,
                request,
                stream_call_id,
                on_event,
                lifecycle,
                cancellation,
            )
            .await;
        }
        _ => {
            return Err(CommandError::from_backend_with_lifecycle(
                BackendError::new(
                    ora_backend::ErrorClassification::InvalidRequest,
                    PublicError::InvalidRequest(EmptyErrorParams {}),
                    "unsupported stream operation",
                ),
                &lifecycle,
            ));
        }
    }
    Ok(())
}

/// Starts a native filesystem watcher for an already-resolved checkout root.
async fn start_workspace_watch(
    state: State<'_, DesktopState>,
    root: PathBuf,
    stream_call_id: String,
    on_event: Channel<serde_json::Value>,
    lifecycle: RequestLifecycle,
    cancellation: CancellationToken,
) -> Result<(), CommandError> {
    let workspace_files = state.workspace_files.clone();
    let watcher = tauri::async_runtime::spawn_blocking(move || workspace_files.watch(&root))
        .await
        .map_err(|source| {
            CommandError::from_backend_with_lifecycle(
                BackendError::internal("Desktop workspace watcher setup failed", source),
                &lifecycle,
            )
        })?
        .map_err(|error| {
            CommandError::from_backend_with_lifecycle(
                workspace_file_backend_error(error),
                &lifecycle,
            )
        })?;
    register_contract_stream(&state, &stream_call_id, &cancellation)
        .map_err(|error| CommandError::from_backend_with_lifecycle(error, &lifecycle))?;
    let registry = state.stream_cancellations.clone();
    tauri::async_runtime::spawn(forward_workspace_watch(
        watcher,
        cancellation,
        stream_call_id,
        registry,
        on_event,
        lifecycle,
    ));
    Ok(())
}

/// Registers a successfully-created stream and rejects duplicate private call identifiers.
pub(crate) fn register_contract_stream(
    state: &DesktopState,
    stream_call_id: &str,
    cancellation: &CancellationToken,
) -> Result<(), BackendError> {
    let mut registrations = state.stream_cancellations.lock().map_err(|_poisoned| {
        BackendError::internal(
            "stream registration state is unavailable",
            std::io::Error::other("stream registration lock poisoned"),
        )
    })?;
    if registrations.contains_key(stream_call_id) {
        return Err(BackendError::new(
            ora_backend::ErrorClassification::Conflict,
            PublicError::InvalidRequest(EmptyErrorParams {}),
            "stream call id is already registered",
        ));
    }
    registrations.insert(stream_call_id.to_string(), cancellation.clone());
    Ok(())
}

/// Cancels one private stream registration without exposing its id as a business identifier.
#[tauri::command]
pub async fn cancel_contract_stream(
    state: State<'_, DesktopState>,
    stream_call_id: String,
) -> Result<(), CommandError> {
    let lifecycle = RequestLifecycle::start("cancel_contract_stream", &UuidRequestIdGenerator);
    let request_span =
        ora_logging::span_with_request_id("tauri_command", &lifecycle.request_id().to_string());

    // The body holds a registry lock and never awaits, so the span is entered with `in_scope`
    // instead of `Instrument`, which would keep a guard alive across the async fn boundary.
    request_span.in_scope(|| {
        let registration = state
            .stream_cancellations
            .lock()
            .map(|mut registrations| registrations.remove(&stream_call_id))
            .map_err(|_poisoned| {
                CommandError::from_backend_with_lifecycle(
                    BackendError::internal(
                        "stream registration state is unavailable",
                        std::io::Error::other("stream registration lock poisoned"),
                    ),
                    &lifecycle,
                )
            })?;

        // Cancelling an already-finished stream is not an error: the forwarding task removes its
        // own registration on completion, so a missing entry only means the race resolved first.
        if let Some(cancellation) = registration {
            cancellation.cancel();
        }

        lifecycle.complete_success();
        Ok(())
    })
}

// =============================================================================
// agentRuntime
// =============================================================================

backend_command!(
    get_agent_runtime_status,
    GetAgentRuntimeStatusRequest,
    GetAgentRuntimeStatusResponse,
    get_agent_runtime_status,
    "Reports the live detection status of every application-scoped CLI runtime through the shared Backend."
);

backend_command!(
    list_agent_models,
    ListAgentModelsRequest,
    ListAgentModelsResponse,
    list_agent_models,
    "Lists the models one agent advertises outside any session through the shared Backend."
);

// =============================================================================
// skill
// =============================================================================

backend_command!(
    create_skill,
    CreateSkillRequest,
    CreateSkillResponse,
    create_skill,
    "Creates one skill through the shared Backend."
);
backend_command!(
    get_skill,
    GetSkillRequest,
    GetSkillResponse,
    get_skill,
    "Gets one skill through the shared Backend."
);
backend_command!(
    list_skills,
    ListSkillsRequest,
    ListSkillsResponse,
    list_skills,
    "Lists skills through the shared Backend."
);
backend_command!(
    update_skill,
    UpdateSkillRequest,
    UpdateSkillResponse,
    update_skill,
    "Updates one skill through the shared Backend."
);
backend_command!(
    delete_skill,
    DeleteSkillRequest,
    DeleteSkillResponse,
    delete_skill,
    "Deletes one skill through the shared Backend."
);
backend_command!(
    prepare_skill_import,
    PrepareSkillImportRequest,
    PrepareSkillImportResponse,
    prepare_skill_import,
    "Prepares one skill import source into a previewed session."
);
backend_command!(
    get_skill_import,
    GetSkillImportSessionRequest,
    GetSkillImportSessionResponse,
    get_skill_import,
    "Gets one skill import session with its current progress."
);
backend_command!(
    commit_skill_import,
    CommitSkillImportRequest,
    CommitSkillImportResponse,
    commit_skill_import,
    "Accepts and freezes one skill import commit."
);
backend_command!(
    cancel_skill_import,
    CancelSkillImportRequest,
    CancelSkillImportResponse,
    cancel_skill_import,
    "Cancels one prepared skill import session."
);

// =============================================================================
// agent
// =============================================================================

backend_command!(
    create_agent,
    CreateAgentRequest,
    CreateAgentResponse,
    create_agent,
    "Creates one configurable agent through the shared Backend."
);
backend_command!(
    get_agent,
    GetAgentRequest,
    GetAgentResponse,
    get_agent,
    "Gets one configurable agent through the shared Backend."
);
backend_command!(
    list_agents,
    ListAgentsRequest,
    ListAgentsResponse,
    list_agents,
    "Lists configurable agents through the shared Backend."
);
backend_command!(
    update_agent,
    UpdateAgentRequest,
    UpdateAgentResponse,
    update_agent,
    "Updates one configurable agent through the shared Backend."
);
backend_command!(
    delete_agent,
    DeleteAgentRequest,
    DeleteAgentResponse,
    delete_agent,
    "Deletes one configurable agent through the shared Backend."
);

backend_command!(
    prepare_agent_import,
    PrepareAgentImportRequest,
    PrepareAgentImportResponse,
    prepare_agent_import,
    "Prepares one agent Markdown import source."
);
backend_command!(
    commit_agent_import,
    CommitAgentImportRequest,
    CommitAgentImportResponse,
    commit_agent_import,
    "Commits one prepared agent Markdown import."
);
// =============================================================================
// plugin
// =============================================================================

backend_command!(
    list_installed_plugins,
    ListInstalledPluginsRequest,
    ListInstalledPluginsResponse,
    list_installed_plugins,
    "Lists the cached installed-plugin lifecycle snapshot."
);
backend_command!(
    get_plugin_configuration,
    GetPluginConfigurationRequest,
    GetPluginConfigurationResponse,
    get_plugin_configuration,
    "Loads one typed Plugin Configuration editor snapshot."
);
backend_command!(
    save_plugin_configuration,
    SavePluginConfigurationRequest,
    SavePluginConfigurationResponse,
    save_plugin_configuration,
    "Persists one revision-checked Plugin Configuration replacement."
);
backend_command!(
    reset_plugin_configuration,
    ResetPluginConfigurationRequest,
    ResetPluginConfigurationResponse,
    reset_plugin_configuration,
    "Resets explicit overrides or recovers a damaged Plugin Configuration."
);
backend_command!(
    list_available_plugins,
    ListAvailablePluginsRequest,
    ListAvailablePluginsResponse,
    list_available_plugins,
    "Lists the cached marketplace registry index."
);
backend_command!(
    sync_available_plugins,
    SyncAvailablePluginsRequest,
    SyncAvailablePluginsResponse,
    sync_available_plugins,
    "Pulls the marketplace source and rebuilds the cached registry index."
);

backend_command!(
    list_marketplace_sources,
    ListMarketplaceSourcesRequest,
    ListMarketplaceSourcesResponse,
    list_marketplace_sources,
    "Lists the configured marketplace source repositories."
);
backend_command!(
    add_marketplace_source,
    AddMarketplaceSourceRequest,
    AddMarketplaceSourceResponse,
    add_marketplace_source,
    "Adds one marketplace source repository."
);
backend_command!(
    delete_marketplace_source,
    DeleteMarketplaceSourceRequest,
    DeleteMarketplaceSourceResponse,
    delete_marketplace_source,
    "Removes one marketplace source repository."
);
async_backend_command!(
    scan_plugins,
    ScanPluginsRequest,
    ScanPluginsResponse,
    scan_plugins,
    "Explicitly scans and reconciles installed plugins."
);
async_backend_command!(
    enable_plugin,
    EnablePluginRequest,
    EnablePluginResponse,
    enable_plugin,
    "Persists plugin eligibility without activating it."
);
async_backend_command!(
    disable_plugin,
    DisablePluginRequest,
    DisablePluginResponse,
    disable_plugin,
    "Stops and disables one installed plugin."
);
async_backend_command!(
    activate_plugin,
    ActivatePluginRequest,
    ActivatePluginResponse,
    activate_plugin,
    "Activates one enabled plugin."
);
async_backend_command!(
    stop_plugin,
    StopPluginRequest,
    StopPluginResponse,
    stop_plugin,
    "Stops one plugin without changing eligibility."
);
async_backend_command!(
    uninstall_plugin,
    UninstallPluginRequest,
    UninstallPluginResponse,
    uninstall_plugin,
    "Stops and removes one installed plugin."
);
async_backend_command!(
    install_plugin,
    InstallPluginRequest,
    InstallPluginResponse,
    install_plugin,
    "Installs one marketplace plugin by downloading, verifying, and extracting it."
);
async_backend_command!(
    import_plugin,
    ImportPluginRequest,
    ImportPluginResponse,
    import_plugin,
    "Imports one local .orax release archive, then enables the imported plugin."
);

// =============================================================================
// gitIdentity
// =============================================================================

backend_command!(
    get_git_identity,
    GetGitIdentityRequest,
    GitIdentityResponse,
    read_git_identity,
    "Reads the host's global Git identity through the shared Backend."
);

// =============================================================================
// workflow
// =============================================================================

backend_command!(
    create_workflow,
    CreateWorkflowRequest,
    CreateWorkflowResponse,
    create_workflow,
    "Creates one workflow through the shared Backend."
);
backend_command!(
    get_workflow,
    GetWorkflowRequest,
    GetWorkflowResponse,
    get_workflow,
    "Gets one workflow through the shared Backend."
);
backend_command!(
    list_workflows,
    ListWorkflowsRequest,
    ListWorkflowsResponse,
    list_workflows,
    "Lists workflows through the shared Backend."
);
backend_command!(
    update_workflow,
    UpdateWorkflowRequest,
    UpdateWorkflowResponse,
    update_workflow,
    "Updates one workflow through the shared Backend."
);
backend_command!(
    delete_workflow,
    DeleteWorkflowRequest,
    DeleteWorkflowResponse,
    delete_workflow,
    "Deletes one workflow through the shared Backend."
);
backend_command!(
    get_workflow_draft,
    GetDraftRequest,
    GetDraftResponse,
    get_workflow_draft,
    "Gets one workflow's draft snapshot through the shared Backend."
);
backend_command!(
    update_workflow_draft,
    UpdateDraftRequest,
    UpdateDraftResponse,
    update_workflow_draft,
    "Updates one workflow's draft graph through the shared Backend."
);
backend_command!(
    publish_workflow,
    PublishWorkflowRequest,
    PublishWorkflowResponse,
    publish_workflow,
    "Publishes one workflow draft through the shared Backend."
);
backend_command!(
    rollback_workflow,
    RollbackWorkflowRequest,
    RollbackWorkflowResponse,
    rollback_workflow,
    "Rolls back one workflow draft through the shared Backend."
);
backend_command!(
    activate_workflow,
    ActivateWorkflowRequest,
    ActivateWorkflowResponse,
    activate_workflow,
    "Activates one workflow version through the shared Backend."
);
backend_command!(
    list_workflow_versions,
    ListVersionsRequest,
    ListVersionsResponse,
    list_workflow_versions,
    "Lists one workflow's published versions through the shared Backend."
);
backend_command!(
    get_workflow_version,
    GetVersionRequest,
    GetVersionResponse,
    get_workflow_version,
    "Gets one workflow version snapshot through the shared Backend."
);
backend_command!(
    delete_workflow_snapshot,
    DeleteSnapshotRequest,
    DeleteSnapshotResponse,
    delete_workflow_snapshot,
    "Deletes one workflow snapshot through the shared Backend."
);
backend_command!(
    get_workflow_snapshot,
    GetWorkflowSnapshotRequest,
    GetWorkflowSnapshotResponse,
    get_workflow_snapshot,
    "Gets one snapshot by id through the shared Backend."
);

// =============================================================================
// workflowRun
// =============================================================================

backend_command!(
    create_workflow_run,
    CreateWorkflowRunRequest,
    CreateWorkflowRunResponse,
    create_workflow_run,
    "Creates one workflow run through the shared Backend."
);
backend_command!(
    get_workflow_run,
    GetWorkflowRunRequest,
    GetWorkflowRunResponse,
    get_workflow_run,
    "Gets one workflow run through the shared Backend."
);
backend_command!(
    list_workflow_runs,
    ListWorkflowRunsRequest,
    ListWorkflowRunsResponse,
    list_workflow_runs,
    "Lists workflow runs for one project through the shared Backend."
);
backend_command!(
    list_workflow_runs_by_workflow,
    ListWorkflowRunsByWorkflowRequest,
    ListWorkflowRunsByWorkflowResponse,
    list_workflow_runs_by_workflow,
    "Lists workflow runs for one workflow through the shared Backend."
);
backend_command!(
    list_workflow_node_runs,
    ListWorkflowNodeRunsRequest,
    ListWorkflowNodeRunsResponse,
    list_workflow_node_runs,
    "Lists the node-run history of one workflow run through the shared Backend."
);
backend_command!(
    delete_workflow_run,
    DeleteWorkflowRunRequest,
    DeleteWorkflowRunResponse,
    delete_workflow_run,
    "Deletes one workflow run through the shared Backend."
);
backend_command!(
    rename_workflow_run,
    RenameWorkflowRunRequest,
    RenameWorkflowRunResponse,
    rename_workflow_run,
    "Renames one workflow run through the shared Backend."
);
backend_command!(
    start_workflow_run,
    StartWorkflowRunRequest,
    StartWorkflowRunResponse,
    start_workflow_run,
    "Starts one workflow run through the shared Backend."
);
/// Cancels one workflow run through the shared Backend.
///
/// Not a `backend_command!` because cancelling also stops the run's live agent
/// sessions, which is asynchronous.
#[tauri::command]
pub async fn cancel_workflow_run(
    state: State<'_, DesktopState>,
    request: CancelWorkflowRunRequest,
) -> Result<CancelWorkflowRunResponse, CommandError> {
    state
        .backend
        .cancel_workflow_run(request)
        .await
        .map_err(CommandError::from)
}
backend_command!(
    restart_workflow_run,
    RestartWorkflowRunRequest,
    RestartWorkflowRunResponse,
    restart_workflow_run,
    "Restarts one workflow run through the shared Backend."
);
backend_command!(
    update_workflow_run_input,
    UpdateWorkflowRunInputRequest,
    UpdateWorkflowRunInputResponse,
    update_workflow_run_input,
    "Updates the kickoff input of one workflow run through the shared Backend."
);
/// Completes one awaiting interactive workflow node through the shared Backend.
///
/// Not a `backend_command!` because completion also stops the node's session, which is
/// asynchronous.
#[tauri::command]
pub async fn complete_workflow_node(
    state: State<'_, DesktopState>,
    request: CompleteWorkflowNodeRequest,
) -> Result<CompleteWorkflowNodeResponse, CommandError> {
    state
        .backend
        .complete_workflow_node(request)
        .await
        .map_err(CommandError::from)
}

// =============================================================================
// desktop
// =============================================================================

/// Carries the empty request used to read Desktop runtime configuration consistently.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetDesktopConfigRequest {}

/// Returns the current non-sensitive Desktop runtime configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetDesktopConfigResponse {
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
        state.backend.clone(),
        request,
        resolve_task_cwd_backend,
    )
    .await
}

fn resolve_task_cwd_backend(
    backend: &Backend,
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
        state.backend.clone(),
        request,
        resolve_workspace_cwd_backend,
    )
    .await
}

/// Resolves a Workspace's local directory through the composed backend.
fn resolve_workspace_cwd_backend(
    backend: &Backend,
    request: ResolveWorkspaceCwdRequest,
) -> Result<ResolveWorkspaceCwdResponse, BackendError> {
    backend
        .resolve_workspace_cwd(&request.workspace_id)
        .map(|path| ResolveWorkspaceCwdResponse {
            path: to_native_path_string(&path),
        })
}

/// Reads the current Desktop worktree configuration without touching the Web API surface.
#[tauri::command]
pub async fn get_desktop_config(
    state: State<'_, DesktopState>,
    request: GetDesktopConfigRequest,
) -> Result<GetDesktopConfigResponse, CommandError> {
    let _ = request;
    let lifecycle = RequestLifecycle::start("get_desktop_config", &UuidRequestIdGenerator);
    let result = state
        .config
        .snapshot()
        .map_err(desktop_config_backend_error)
        .map(|config| GetDesktopConfigResponse {
            worktree_root: config.worktree_root().to_string_lossy().into_owned(),
        });
    match result {
        Ok(response) => {
            lifecycle.complete_success();
            Ok(response)
        }
        Err(error) => Err(CommandError::from_backend_with_lifecycle(error, &lifecycle)),
    }
}

/// Persists a new creation root and updates Backend configuration without interrupting in-flight work.
#[tauri::command]
pub async fn set_worktree_root(
    state: State<'_, DesktopState>,
    request: SetWorktreeRootRequest,
) -> Result<SetWorktreeRootResponse, CommandError> {
    let backend = state.backend.clone();
    let config_store = state.config.clone();
    let lifecycle = RequestLifecycle::start("set_worktree_root", &UuidRequestIdGenerator);
    let request_span =
        ora_logging::span_with_request_id("tauri_command", &lifecycle.request_id().to_string());
    let blocking_span = request_span.clone();
    let secondary_lifecycle = lifecycle.clone();
    let result = match tauri::async_runtime::spawn_blocking(move || {
        blocking_span.in_scope(|| {
            let previous = config_store
                .snapshot()
                .map_err(desktop_config_backend_error)?;
            let worktree_root = PathBuf::from(request.worktree_root);

            validate_worktree_root(&worktree_root).map_err(desktop_config_backend_error)?;
            backend.set_worktree_root(worktree_root.clone())?;
            if let Err(error) = config_store.set_worktree_root(worktree_root.clone()) {
                if let Err(rollback_error) =
                    backend.set_worktree_root(previous.worktree_root().to_path_buf())
                {
                    let report = ora_logging::ErrorReport::from_error(&rollback_error);
                    ora_logging::ora_error!(
                        operation = "set_worktree_root.rollback",
                        request_id = %secondary_lifecycle.request_id(),
                        outcome = "secondary_failure",
                        error.code = rollback_error.public_error().code(),
                        error.message = report.message(),
                        error.chain = report.chain(),
                        error.chain_depth = report.chain_depth(),
                        "secondary cleanup failed"
                    );
                }
                return Err(desktop_config_backend_error(error));
            }

            Ok(SetWorktreeRootResponse {
                worktree_root: worktree_root.to_string_lossy().into_owned(),
            })
        })
    })
    .await
    {
        Ok(result) => result,
        Err(source) => Err(BackendError::internal(
            "Desktop command execution failed",
            source,
        )),
    };
    async move {
        match result {
            Ok(response) => {
                lifecycle.complete_success();
                Ok(response)
            }
            Err(error) => Err(CommandError::from_backend_with_lifecycle(error, &lifecycle)),
        }
    }
    .instrument(request_span)
    .await
}
