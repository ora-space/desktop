use crate::childprocess::{
    ChildProcessEnvironmentProvider, NoChildProcessEnvironment, PluginProcessHost,
};
use crate::ports::{
    LaunchedRuntime, PluginCallError, PluginLaunchRequest, PluginRuntime, PluginRuntimeExit,
    PluginRuntimeFailure, PluginRuntimeLauncher,
};
use crate::storage::PluginStorage;
use ora_plugin_runtime::{
    HostRequestError, HostRequestHandler, PluginProcessExit, PluginRegistration,
    PluginRuntime as ProcessPluginRuntime, PluginRuntimeConfig, PluginRuntimeError,
};
use ora_process::TokioProcessSpawner;
use serde_json::Value;
use std::future::Future;
use std::time::Duration;

/// Dispatches one plugin process's host requests to whichever handler owns the method's
/// namespace. `ora-plugin-runtime` launches with exactly one [`HostRequestHandler`] per process,
/// so this is the single place `ora/storage/*` and `ora/childprocess/*` combine.
///
/// `processes` is `None` for every non-agent plugin: those launch with zero Deno permissions
/// specifically so they cannot start a process (see [`PluginLaunchRequest::allow_childprocess`]),
/// and mounting the handler unconditionally would let a host request reopen that door.
struct PluginHostRequests<E: ChildProcessEnvironmentProvider> {
    storage: PluginStorage,
    processes: Option<PluginProcessHost<TokioProcessSpawner, E>>,
}

impl<E: ChildProcessEnvironmentProvider> HostRequestHandler for PluginHostRequests<E> {
    async fn handle(&self, method: &str, params: Value) -> Result<Value, HostRequestError> {
        if method.starts_with("ora/childprocess/") {
            match &self.processes {
                Some(processes) => processes.handle(method, params).await,
                None => Err(HostRequestError::method_not_found(method)),
            }
        } else {
            self.storage.handle(method, params).await
        }
    }
}

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
#[derive(Debug, Clone, Default)]
pub struct DenoPluginRuntimeLauncher<E: ChildProcessEnvironmentProvider = NoChildProcessEnvironment>
{
    timeouts: PluginRuntimeTimeouts,
    environment_provider: E,
}

impl DenoPluginRuntimeLauncher<NoChildProcessEnvironment> {
    /// Creates a launcher with explicit process lifecycle timeouts.
    pub fn new(timeouts: PluginRuntimeTimeouts) -> Self {
        Self {
            timeouts,
            environment_provider: NoChildProcessEnvironment,
        }
    }
}

impl<E: ChildProcessEnvironmentProvider> DenoPluginRuntimeLauncher<E> {
    /// Creates a launcher that injects host-authorized Agent subprocess environment variables.
    pub fn with_environment_provider(
        timeouts: PluginRuntimeTimeouts,
        environment_provider: E,
    ) -> Self {
        Self {
            timeouts,
            environment_provider,
        }
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

impl<E: ChildProcessEnvironmentProvider> PluginRuntimeLauncher for DenoPluginRuntimeLauncher<E> {
    type Runtime = DenoPluginRuntime;

    /// Renders permissions, starts Deno in the package root with the storage handler bound to
    /// the plugin's data directory, and waits for the handshake.
    fn launch(
        &self,
        request: PluginLaunchRequest,
    ) -> impl Future<Output = Result<LaunchedRuntime<Self::Runtime>, PluginRuntimeFailure>> + Send
    {
        let timeouts = self.timeouts;
        let environment_provider = self.environment_provider.clone();
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
            let processes = request.allow_childprocess.then(|| {
                PluginProcessHost::with_environment_provider(
                    request.plugin_id.to_string(),
                    request.package_root.clone(),
                    TokioProcessSpawner::new(),
                    environment_provider,
                )
            });
            let host_requests = PluginHostRequests {
                storage: PluginStorage::new(request.data_dir),
                processes: processes.clone(),
            };
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
                host_requests,
            )
            .await
            .map_err(|error| PluginRuntimeFailure::new(error.to_string()))?;
            // The handler must exist before `launch` is called, so it can only learn how to push
            // notifications back to the plugin once `launch` has returned this handle.
            if let Some(processes) = &processes {
                processes.attach_runtime(runtime.clone());
            }
            // Whatever ends this plugin generation — an intentional stop, uninstall, restart, or
            // failure — must also end every process it asked the host to spawn on its behalf; see
            // `PluginProcessHost::kill_all`.
            tokio::spawn({
                let processes = processes.clone();
                let runtime = runtime.clone();
                async move {
                    runtime.wait_for_exit().await;
                    if let Some(processes) = processes {
                        processes.kill_all().await;
                    }
                }
            });
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

#[cfg(test)]
mod tests {
    use super::{NoChildProcessEnvironment, PluginHostRequests};
    use crate::childprocess::CHILDPROCESS_SPAWN_METHOD;
    use crate::storage::PluginStorage;
    use ora_plugin_runtime::{HostRequestError, HostRequestHandler};
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::path::PathBuf;

    /// A non-agent plugin's dispatcher has no `processes` handler mounted, so a spawn request
    /// must fail the same way an entirely unknown method would rather than reach a spawner.
    #[tokio::test]
    async fn childprocess_methods_are_unknown_without_the_processes_handler() {
        let host_requests: PluginHostRequests<NoChildProcessEnvironment> = PluginHostRequests {
            storage: PluginStorage::new(PathBuf::from(".")),
            processes: None,
        };

        let error = host_requests
            .handle(CHILDPROCESS_SPAWN_METHOD, json!({ "command": "opencode" }))
            .await
            .expect_err("spawn is refused without a processes handler");

        assert_eq!(
            error,
            HostRequestError::method_not_found(CHILDPROCESS_SPAWN_METHOD)
        );
    }
}
