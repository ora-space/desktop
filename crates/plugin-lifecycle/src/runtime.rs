use crate::childprocess::{
    ChildProcessEnvironmentProvider, NoChildProcessEnvironment, PluginProcessHost,
};
use crate::ports::{
    LaunchedRuntime, PluginCallError, PluginLaunchRequest, PluginRuntime, PluginRuntimeExit,
    PluginRuntimeFailure, PluginRuntimeLauncher,
};
use crate::storage::PluginStorage;
use ora_plugin_runtime::{
    HostRequestError, HostRequestHandler, PluginLogSetup, PluginProcessExit, PluginRegistration,
    PluginRuntime as ProcessPluginRuntime, PluginRuntimeConfig, PluginRuntimeError,
};
use ora_process::{ProcessSpawner, TokioManagedProcess, TokioProcessSpawner};
use serde_json::Value;
use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

/// Dispatches one plugin process's host requests to whichever handler owns the method's
/// namespace. `ora-plugin-runtime` launches with exactly one [`HostRequestHandler`] per process,
/// so this is the single place `ora/storage/*` and `ora/childprocess/*` combine.
///
/// `processes` is `None` for every non-agent plugin: those launch with zero Deno permissions
/// specifically so they cannot start a process (see [`PluginLaunchRequest::allow_childprocess`]),
/// and mounting the handler unconditionally would let a host request reopen that door.
struct PluginHostRequests<
    E: ChildProcessEnvironmentProvider,
    S: ProcessSpawner = TokioProcessSpawner,
> {
    storage: PluginStorage,
    processes: Option<PluginProcessHost<S, E>>,
}

impl<E, S> HostRequestHandler for PluginHostRequests<E, S>
where
    E: ChildProcessEnvironmentProvider,
    S: ProcessSpawner + Send + Sync + 'static,
    S::Process: Send + Sync + 'static,
{
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
pub struct DenoPluginRuntimeLauncher<
    E: ChildProcessEnvironmentProvider = NoChildProcessEnvironment,
    S = TokioProcessSpawner,
> {
    timeouts: PluginRuntimeTimeouts,
    environment_provider: E,
    /// Starts both the Deno process and every process it asks the host to spawn, so the plugin
    /// and its agent CLI always share one OS identity.
    spawner: S,
}

impl DenoPluginRuntimeLauncher<NoChildProcessEnvironment> {
    /// Creates a launcher with explicit process lifecycle timeouts.
    pub fn new(timeouts: PluginRuntimeTimeouts) -> Self {
        Self {
            timeouts,
            environment_provider: NoChildProcessEnvironment,
            spawner: TokioProcessSpawner::new(),
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
            spawner: TokioProcessSpawner::new(),
        }
    }

    /// Replaces the spawner, for a host that runs plugins under an identity other than its own
    /// (see [`ora_process::ProcessIdentity`]).
    pub fn with_spawner<S>(self, spawner: S) -> DenoPluginRuntimeLauncher<E, S> {
        DenoPluginRuntimeLauncher {
            timeouts: self.timeouts,
            environment_provider: self.environment_provider,
            spawner,
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
    stop: tokio::sync::watch::Sender<bool>,
    cleanup: tokio::sync::watch::Receiver<Option<Result<(), PluginRuntimeFailure>>>,
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

impl<E, S> PluginRuntimeLauncher for DenoPluginRuntimeLauncher<E, S>
where
    E: ChildProcessEnvironmentProvider,
    S: ProcessSpawner<Process = TokioManagedProcess> + Clone + Send + Sync + 'static,
{
    type Runtime = DenoPluginRuntime;

    /// Renders permissions, starts Deno in the package root with the storage handler bound to
    /// the plugin's data directory, and waits for the handshake.
    fn launch(
        &self,
        request: PluginLaunchRequest,
        log: PluginLogSetup,
    ) -> impl Future<Output = Result<LaunchedRuntime<Self::Runtime>, PluginRuntimeFailure>> + Send
    {
        let timeouts = self.timeouts;
        let environment_provider = self.environment_provider.clone();
        let spawner = self.spawner.clone();
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
            // Only agent plugins may run processes, so only they receive a process environment.
            let environment = if request.allow_childprocess {
                environment_provider.plugin_environment(&request.plugin_id.to_string())
            } else {
                BTreeMap::new()
            };
            let processes = request.allow_childprocess.then(|| {
                PluginProcessHost::with_environment_provider(
                    request.plugin_id.to_string(),
                    request.package_root.clone(),
                    spawner.clone(),
                    environment_provider,
                )
            });
            let host_requests = PluginHostRequests {
                storage: PluginStorage::new(request.data_dir),
                processes: processes.clone(),
            };
            let launched = ProcessPluginRuntime::launch(
                &spawner,
                PluginRuntimeConfig {
                    plugin_id: request.plugin_id.to_string(),
                    deno_path: request.deno_path,
                    entrypoint: request.entrypoint,
                    permissions,
                    cwd: Some(request.package_root),
                    environment,
                    ready_timeout: timeouts.ready,
                    call_timeout: timeouts.call,
                    shutdown_timeout: timeouts.shutdown,
                },
                host_requests,
                log,
            )
            .await;
            let (runtime, notifications) = match launched {
                Ok(launched) => launched,
                Err(error) => {
                    if let Some(processes) = processes {
                        match tokio::time::timeout(timeouts.shutdown, processes.kill_all()).await {
                            Ok(Ok(())) => {}
                            _ => {
                                return Err(PluginRuntimeFailure::new(
                                    "host child cleanup unconfirmed",
                                ));
                            }
                        }
                    }
                    return Err(PluginRuntimeFailure::new(error.to_string()));
                }
            };
            // The handler must exist before `launch` is called, so it can only learn how to push
            // notifications back to the plugin once `launch` has returned this handle.
            if let Some(processes) = &processes {
                processes.attach_runtime(runtime.clone());
            }
            // Whatever ends this plugin generation — an intentional stop, uninstall, restart, or
            // failure — must also end every process it asked the host to spawn on its behalf; see
            // `PluginProcessHost::kill_all`.
            let (stop, mut stop_rx) = tokio::sync::watch::channel(false);
            let (cleanup_tx, cleanup) = tokio::sync::watch::channel(None);
            tokio::spawn({
                let processes = processes.clone();
                let runtime = runtime.clone();
                async move {
                    tokio::select! {
                        _ = runtime.wait_for_exit() => {},
                        _ = stop_rx.wait_for(|stop| *stop) => {},
                    }
                    // The process supervisor consumes the graceful interval before forced kill;
                    // leave a separate bounded interval for reaping and finishing its log.
                    let result = tokio::time::timeout(
                        timeouts.shutdown.saturating_add(Duration::from_secs(5)),
                        async {
                            let children = async {
                                match processes {
                                    Some(processes) => processes.kill_all().await,
                                    None => Ok(()),
                                }
                            };
                            let (_, result) = tokio::join!(runtime.shutdown_and_wait(), children);
                            result.map_err(|_| {
                                PluginRuntimeFailure::new("host child cleanup unconfirmed")
                            })
                        },
                    )
                    .await
                    .unwrap_or_else(|_| {
                        Err(PluginRuntimeFailure::new("plugin cleanup deadline elapsed"))
                    });
                    cleanup_tx.send_replace(Some(result));
                }
            });
            let registration = runtime.registration().await;
            Ok(LaunchedRuntime {
                runtime: DenoPluginRuntime {
                    runtime,
                    registration,
                    stop,
                    cleanup,
                },
                notifications,
            })
        }
    }
}

/// Maps plugin-runtime call failures onto the transport-neutral lifecycle error.
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
    /// Requests shutdown and waits for direct plugin and host-child cleanup acknowledgements.
    fn stop(&self) -> impl Future<Output = Result<(), PluginRuntimeFailure>> + Send {
        self.stop.send_replace(true);
        let mut cleanup = self.cleanup.clone();
        async move {
            loop {
                if let Some(result) = cleanup.borrow().clone() {
                    return result;
                }
                if cleanup.changed().await.is_err() {
                    return Err(PluginRuntimeFailure::new(
                        "plugin cleanup owner unavailable",
                    ));
                }
            }
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
