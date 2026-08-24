use crate::ports::{
    LaunchedRuntime, PluginCallError, PluginLaunchRequest, PluginRuntime, PluginRuntimeExit,
    PluginRuntimeFailure, PluginRuntimeLauncher,
};
use crate::storage::PluginStorage;
use ora_plugin_runtime::{
    PluginProcessExit, PluginRegistration, PluginRuntime as ProcessPluginRuntime,
    PluginRuntimeConfig, PluginRuntimeError,
};
use ora_process::TokioProcessSpawner;
use serde_json::Value;
use std::future::Future;
use std::time::Duration;

/// Configures bounded startup, invocation, and shutdown waits for real plugin processes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PluginRuntimeTimeouts {
    pub ready: Duration,
    pub call: Duration,
    pub shutdown: Duration,
}

impl Default for PluginRuntimeTimeouts {
    fn default() -> Self {
        Self {
            ready: Duration::from_secs(10),
            call: Duration::from_secs(30),
            shutdown: Duration::from_secs(5),
        }
    }
}

/// Launches production Deno plugin processes through Ora's process-tree supervisor.
#[derive(Debug, Clone, Copy, Default)]
pub struct DenoPluginRuntimeLauncher {
    timeouts: PluginRuntimeTimeouts,
}

impl DenoPluginRuntimeLauncher {
    /// Creates a launcher with explicit process lifecycle timeouts.
    pub fn new(timeouts: PluginRuntimeTimeouts) -> Self {
        Self { timeouts }
    }
}

/// Adapts the JSON-RPC plugin runtime to lifecycle stop, exit observation, and calls.
#[derive(Clone)]
pub struct DenoPluginRuntime {
    runtime: ProcessPluginRuntime,
    /// Captured once at launch: the process runtime fixes it before reporting readiness, and
    /// keeping a copy lets contract validation stay synchronous.
    registration: PluginRegistration,
}

impl DenoPluginRuntime {
    /// Returns the JSON-RPC handle callers invoke plugin methods and notifications through.
    ///
    /// Consumers that already speak the process protocol directly, such as the agent connection
    /// supervisor, use this instead of re-wrapping every call through the lifecycle trait.
    pub fn process(&self) -> &ProcessPluginRuntime {
        &self.runtime
    }
}

impl PluginRuntimeLauncher for DenoPluginRuntimeLauncher {
    type Runtime = DenoPluginRuntime;

    /// Renders permissions, starts Deno in the package root with the storage handler bound to
    /// the plugin's data directory, and waits for the handshake.
    fn launch(
        &self,
        request: PluginLaunchRequest,
    ) -> impl Future<Output = Result<LaunchedRuntime<Self::Runtime>, PluginRuntimeFailure>> + Send
    {
        let timeouts = self.timeouts;
        async move {
            let permissions = request
                .permissions
                .iter()
                .map(|permission| {
                    permission
                        .to_flag()
                        .map(|flag| flag.to_string_lossy().into_owned())
                        .map_err(|error| PluginRuntimeFailure::new(error.to_string()))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let (runtime, notifications) = ProcessPluginRuntime::launch(
                &TokioProcessSpawner::new(),
                PluginRuntimeConfig {
                    plugin_id: request.plugin_id.to_string(),
                    deno_path: request.deno_path,
                    entrypoint: request.entrypoint,
                    permissions,
                    cwd: Some(request.package_root),
                    ready_timeout: timeouts.ready,
                    call_timeout: timeouts.call,
                    shutdown_timeout: timeouts.shutdown,
                },
                PluginStorage::new(request.data_dir),
            )
            .await
            .map_err(|error| PluginRuntimeFailure::new(error.to_string()))?;
            let registration = runtime.registration().await;
            Ok(LaunchedRuntime {
                runtime: DenoPluginRuntime {
                    runtime,
                    registration,
                },
                notifications,
            })
        }
    }
}

/// Maps process-runtime call failures onto the transport-neutral lifecycle error.
fn map_call_error(error: PluginRuntimeError) -> PluginCallError {
    match error {
        PluginRuntimeError::Unavailable(_) => PluginCallError::Unavailable,
        PluginRuntimeError::MethodNotRegistered(_) => PluginCallError::MethodNotRegistered,
        PluginRuntimeError::CallTimeout => PluginCallError::Timeout,
        PluginRuntimeError::Remote { code, message } => PluginCallError::Remote { code, message },
        PluginRuntimeError::RequestChannelClosed
        | PluginRuntimeError::MissingEntrypoint(_)
        | PluginRuntimeError::Spawn(_)
        | PluginRuntimeError::MissingStdio
        | PluginRuntimeError::ReadyTimeout => PluginCallError::Transport(error.to_string()),
    }
}

impl PluginRuntime for DenoPluginRuntime {
    /// Requests shutdown and waits until the complete supervised process tree exits.
    fn stop(&self) -> impl Future<Output = Result<(), PluginRuntimeFailure>> + Send {
        let runtime = self.runtime.clone();
        async move {
            runtime.shutdown_and_wait().await;
            Ok(())
        }
    }

    /// Observes process exit without conflating intentional shutdown with failure.
    fn wait_for_exit(&self) -> impl Future<Output = PluginRuntimeExit> + Send + 'static {
        let runtime = self.runtime.clone();
        async move {
            match runtime.wait_for_exit().await {
                PluginProcessExit::Stopped => PluginRuntimeExit::Stopped,
                PluginProcessExit::Failed(reason) => {
                    PluginRuntimeExit::Failed(PluginRuntimeFailure::new(reason))
                }
            }
        }
    }

    /// Returns the registration captured when the handshake completed.
    fn registration(&self) -> PluginRegistration {
        self.registration.clone()
    }

    /// Delegates the request to the process runtime and maps its error.
    fn invoke(
        &self,
        method: &str,
        params: Value,
    ) -> impl Future<Output = Result<Value, PluginCallError>> + Send {
        let runtime = self.runtime.clone();
        let method = method.to_string();
        async move {
            runtime
                .invoke(&method, params)
                .await
                .map_err(map_call_error)
        }
    }
}
