use crate::config::validate_worktree_root;
use crate::error::{CommandError, desktop_config_backend_error};
use crate::state::DesktopState;
use ora_backend::{Backend, BackendError, RequestLifecycle, UuidRequestIdGenerator};
use ora_contracts::*;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::path::PathBuf;
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
    update_project,
    UpdateProjectRequest,
    UpdateProjectResponse,
    update_project,
    "Updates one project through the shared Backend."
);
backend_command!(
    delete_project,
    DeleteProjectRequest,
    DeleteProjectResponse,
    delete_project,
    "Deletes one project through the shared Backend."
);

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
backend_command!(
    delete_task,
    DeleteTaskRequest,
    DeleteTaskResponse,
    delete_task,
    "Deletes one task through the shared Backend."
);

// =============================================================================
// session
// =============================================================================

/// Creates one provider-backed session through the asynchronous runtime manager.
#[tauri::command]
pub async fn create_session(
    state: State<'_, DesktopState>,
    request: CreateSessionRequest,
) -> Result<CreateSessionResponse, CommandError> {
    run_async_backend("create_session", state.backend.create_session(request)).await
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

/// Stops the provider process before removing only the Ora-owned session record.
#[tauri::command]
pub async fn delete_session(
    state: State<'_, DesktopState>,
    request: DeleteSessionRequest,
) -> Result<DeleteSessionResponse, CommandError> {
    run_async_backend("delete_session", state.backend.delete_session(request)).await
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

/// Registers a successfully-created stream and rejects duplicate private call identifiers.
fn register_contract_stream(
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
    if let Some(cancellation) = state
        .stream_cancellations
        .lock()
        .map_err(|_poisoned| CommandError::execution())?
        .remove(&stream_call_id)
    {
        cancellation.cancel();
    }
    Ok(())
}

/// Forwards ordered data/error/end frames and drops the backend stream on channel failure.
async fn forward_contract_stream<Event>(
    mut stream: ora_backend::SessionEventStream<Event>,
    cancellation: CancellationToken,
    stream_call_id: String,
    registry: std::sync::Arc<
        std::sync::Mutex<std::collections::HashMap<String, CancellationToken>>,
    >,
    on_event: Channel<serde_json::Value>,
    lifecycle: RequestLifecycle,
) where
    Event: Serialize + Send + 'static,
{
    loop {
        tokio::select! {
            () = cancellation.cancelled() => {
                lifecycle.complete_cancellation();
                break;
            },
            event = stream.recv() => {
                let is_terminal = matches!(&event, Some(Err(_)) | None);
                let frame = match event {
                    Some(Ok(data)) => serde_json::json!({ "type": "data", "data": data }),
                    Some(Err(error)) => {
                        lifecycle.complete_failure(&error);
                        serde_json::json!({
                            "type": "error",
                            "error": error.contract_error(lifecycle.request_id()),
                        })
                    },
                    None => {
                        lifecycle.complete_success();
                        serde_json::json!({ "type": "end" })
                    },
                };
                if on_event.send(frame).is_err() || is_terminal {
                    break;
                }
            }
        }
    }
    if let Ok(mut registrations) = registry.lock() {
        registrations.remove(&stream_call_id);
    }
}

// =============================================================================
// agentRuntime
// =============================================================================

/// Lists models grouped by every CLI whose discovery command succeeds.
#[tauri::command]
pub async fn list_agent_models(
    state: State<'_, DesktopState>,
    request: ListAgentModelsRequest,
) -> Result<ListAgentModelsResponse, CommandError> {
    run_async_backend(
        "list_agent_models",
        state.backend.list_agent_models(request),
    )
    .await
}

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

/// Names the host application a location can be handed off to.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LocationTarget {
    Explorer,
    Terminal,
    VsCode,
}

/// Carries the target application and the absolute path it should open.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenLocationRequest {
    pub target: LocationTarget,
    pub path: String,
}

/// Opens one absolute path in the file manager, a terminal, or VS Code on the host OS.
#[tauri::command]
pub async fn open_location(request: OpenLocationRequest) -> Result<(), CommandError> {
    let lifecycle = RequestLifecycle::start("open_location", &UuidRequestIdGenerator);
    let request_span =
        ora_logging::span_with_request_id("tauri_command", &lifecycle.request_id().to_string());
    let blocking_span = request_span.clone();
    let result = match tauri::async_runtime::spawn_blocking(move || {
        blocking_span.in_scope(|| open_location_blocking(request.target, &request.path))
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
            Ok(()) => {
                lifecycle.complete_success();
                Ok(())
            }
            Err(error) => Err(CommandError::from_backend_with_lifecycle(error, &lifecycle)),
        }
    }
    .instrument(request_span)
    .await
}

/// Reports a location handoff that the host OS refused or could not launch.
fn open_location_error(
    target: LocationTarget,
    source: impl std::error::Error + Send + Sync + 'static,
) -> BackendError {
    BackendError::with_source(
        ora_backend::ErrorClassification::Internal,
        PublicError::OpenLocationFailed(OpenLocationFailedParams {
            target: match target {
                LocationTarget::Explorer => OpenLocationTarget::Explorer,
                LocationTarget::Terminal => OpenLocationTarget::Terminal,
                LocationTarget::VsCode => OpenLocationTarget::Vscode,
            },
        }),
        "failed to open the requested location",
        source,
    )
}

/// Launches the host handler for one location, branching per OS since only desktop hosts call this.
#[cfg(target_os = "windows")]
fn open_location_blocking(target: LocationTarget, path: &str) -> Result<(), BackendError> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    // CREATE_NO_WINDOW: keep the `cmd` shim that resolves `code.cmd` from flashing a console.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    // Git reports worktree paths with forward slashes; explorer.exe only navigates
    // backslash paths and silently falls back to a parent otherwise. Normalize once -
    // `wt`, PowerShell, and `code` all accept backslashes too.
    let normalized = path.replace('/', "\\");
    let path = normalized.as_str();

    match target {
        // explorer.exe returns a non-zero exit code even on success, so a clean spawn is the only
        // signal worth trusting here.
        LocationTarget::Explorer => Command::new("explorer")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|source| open_location_error(target, source)),
        // `code` ships as `code.cmd`, which CreateProcess will not resolve directly; route it
        // through `cmd` and wait so a missing install surfaces as a failure the UI can report.
        LocationTarget::VsCode => {
            let status = Command::new("cmd")
                .args(["/C", "code", path])
                .creation_flags(CREATE_NO_WINDOW)
                .status()
                .map_err(|source| open_location_error(target, source))?;
            if status.success() {
                Ok(())
            } else {
                Err(open_location_error(
                    target,
                    std::io::Error::other(format!("VS Code exited with {status}")),
                ))
            }
        }
        // Prefer Windows Terminal; fall back to a PowerShell window opened in the target directory.
        LocationTarget::Terminal => {
            if Command::new("wt").args(["-d", path]).spawn().is_ok() {
                return Ok(());
            }
            Command::new("cmd")
                .args(["/C", "start", "", "/D", path, "powershell", "-NoExit"])
                .spawn()
                .map(|_| ())
                .map_err(|source| open_location_error(target, source))
        }
    }
}

/// Launches the host handler for one location through macOS `open`, which fails loudly when absent.
#[cfg(target_os = "macos")]
fn open_location_blocking(target: LocationTarget, path: &str) -> Result<(), BackendError> {
    use std::process::Command;

    let mut command = Command::new("open");
    match target {
        LocationTarget::Explorer => {
            command.arg(path);
        }
        LocationTarget::Terminal => {
            command.args(["-a", "Terminal", path]);
        }
        LocationTarget::VsCode => {
            command.args(["-a", "Visual Studio Code", path]);
        }
    }
    let status = command
        .status()
        .map_err(|source| open_location_error(target, source))?;
    if status.success() {
        Ok(())
    } else {
        Err(open_location_error(
            target,
            std::io::Error::other(format!("open command exited with {status}")),
        ))
    }
}

/// Rejects location handoffs on hosts that never run the desktop shell (only Web runs on Linux).
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn open_location_blocking(target: LocationTarget, _path: &str) -> Result<(), BackendError> {
    Err(open_location_error(
        target,
        std::io::Error::other("opening locations is unsupported on this platform"),
    ))
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
