//! Desktop request execution and explicit domain command modules.

use crate::error::CommandError;
use ora_backend::{BackendError, RequestLifecycle, UuidRequestIdGenerator};
use std::future::Future;
use tracing::Instrument;

/// Executes one synchronous backend operation on the runtime's blocking executor.
pub(super) async fn run_backend<Context, Request, Response, Operation>(
    operation_name: &'static str,
    backend: Context,
    request: Request,
    operation: Operation,
) -> Result<Response, CommandError>
where
    Context: Send + 'static,
    Operation: FnOnce(&Context, Request) -> Result<Response, BackendError> + Send + 'static,
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

/// Executes asynchronous work with the same correlated request completion contract.
pub(super) async fn run_async_backend<Response, Call>(
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
    ($name:ident, $request:ty, $response:ty, $domain:ident.$operation:ident, $doc:literal) => {
        #[doc = $doc]
        #[tauri::command]
        pub async fn $name(
            state: tauri::State<'_, $crate::state::DesktopState>,
            request: $request,
        ) -> Result<$response, $crate::error::CommandError> {
            $crate::commands::run_backend(
                stringify!($name),
                state.backend.$domain(),
                request,
                |module, request| module.$operation(request),
            )
            .await
        }
    };
}

macro_rules! async_backend_command {
    ($name:ident, $request:ty, $response:ty, $domain:ident.$operation:ident, $doc:literal) => {
        #[doc = $doc]
        #[tauri::command]
        pub async fn $name(
            state: tauri::State<'_, $crate::state::DesktopState>,
            request: $request,
        ) -> Result<$response, $crate::error::CommandError> {
            let module = state.backend.$domain();
            $crate::commands::run_async_backend(stringify!($name), module.$operation(request)).await
        }
    };
}

pub(crate) mod agent;
pub(crate) mod agent_runtime;
pub(crate) mod app_events;
pub(crate) mod effect;
pub(crate) mod files;
pub(crate) mod git_identity;
pub(crate) mod plugin;
pub(crate) mod project;
pub(crate) mod session;
pub(crate) mod settings;
pub(crate) mod skill;
pub(crate) mod stream;
mod stream_routes;
pub(crate) mod task;
pub(crate) mod workflow;
pub(crate) mod workflow_run;
pub(crate) mod workspace;
