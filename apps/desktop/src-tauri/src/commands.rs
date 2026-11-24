//! Desktop request execution and explicit domain command modules.

use crate::error::CommandError;
use ora_backend::{BackendError, RequestLifecycle, UuidRequestIdGenerator};
use ora_contracts::RequestId;
use std::future::Future;
use std::pin::Pin;
use tracing::Instrument;

/// Upper bound for the future an async command may hold across its await.
///
/// Tauri constructs every async command's future on the **main thread**, inside the WebView2
/// custom-protocol callback, and only then moves it onto the async runtime. That callback stack
/// is roughly 1 MB and the webview/tauri frames below the command handler already occupy
/// several hundred KB of it, so a command future of a few hundred KB overflows the stack before
/// the runtime ever polls it. Release 0.2.0 died exactly this way (WER `0xc00000fd`, main
/// thread, in `tauri::async_runtime::spawn` called from this crate's invoke handler): the
/// marketplace install chain monomorphizes into a single ~650 KB state machine, and the mere
/// act of constructing the `install_plugin` future killed the process.
///
/// The budget below is therefore an IPC-stack safety envelope, not a style preference: any
/// command future within it leaves two orders of magnitude of headroom under the remaining
/// main-thread stack, while a regression that embeds a deep domain future again fails this
/// bound by orders of magnitude.
#[cfg(test)]
pub(super) const IPC_COMMAND_FUTURE_BUDGET: usize = 4096;

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
///
/// `call` must already be boxed (`Box::pin(..)`): Tauri constructs every async command's future
/// on the main thread inside the WebView2 IPC callback, and a deep domain future held across
/// this await overflows that ~1 MB stack before the runtime ever polls it (release 0.2.0 died
/// with a main-thread stack overflow the moment a marketplace install was clicked — the
/// install chain alone is a ~650 KB state machine). Boxing at the call site keeps the future
/// this wrapper — and therefore the command future the IPC thread materializes — pointer-sized,
/// while the deep domain future itself is only ever constructed on the async-runtime thread
/// that first polls this wrapper. The boxed parameter type makes the invariant
/// compiler-enforced: a caller cannot pass a raw domain future here without noticing.
pub(super) async fn run_async_backend<Response, Call>(
    operation_name: &'static str,
    call: Pin<Box<Call>>,
) -> Result<Response, CommandError>
where
    Call: Future<Output = Result<Response, BackendError>>,
{
    run_async_backend_with_request_id(operation_name, |_| call).await
}

/// Supplies diagnostic correlation without granting the operation lifecycle completion authority.
///
/// The operation returns an already-boxed future for the same IPC-stack reason
/// [`run_async_backend`] requires one.
pub(super) async fn run_async_backend_with_request_id<Response, Call>(
    operation_name: &'static str,
    operation: impl FnOnce(RequestId) -> Pin<Box<Call>>,
) -> Result<Response, CommandError>
where
    Call: Future<Output = Result<Response, BackendError>>,
{
    let lifecycle = RequestLifecycle::start(operation_name, &UuidRequestIdGenerator);
    let request_span =
        ora_logging::span_with_request_id("tauri_command", &lifecycle.request_id().to_string());
    async move {
        match operation(lifecycle.request_id()).await {
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
            // The domain future is boxed so the IPC thread never materializes it; see
            // `run_async_backend` and `IPC_COMMAND_FUTURE_BUDGET` for the stack constraint.
            $crate::commands::run_async_backend(
                stringify!($name),
                Box::pin(module.$operation(request)),
            )
            .await
        }
    };
}

pub(crate) mod agent;
pub(crate) mod agent_runtime;
pub(crate) mod app_events;
pub(crate) mod cloud_connection;
pub(crate) mod effect;
pub(crate) mod files;
pub(crate) mod git_identity;
pub(crate) mod plugin;
pub(crate) mod project;
pub(crate) mod scheduled_task;
pub(crate) mod session;
pub(crate) mod settings;
pub(crate) mod skill;
pub(crate) mod stream;
mod stream_routes;
pub(crate) mod task;
pub(crate) mod workflow;
pub(crate) mod workflow_run;
pub(crate) mod workspace;

#[cfg(test)]
mod execution_tests;
