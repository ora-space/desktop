use crate::{
    PluginLaunchRequest, PluginRuntime, PluginRuntimeExit, PluginRuntimeFailure,
    PluginRuntimeLauncher,
};
use ora_plugin_runtime::{
    PluginProcessExit, PluginRuntime as ProcessPluginRuntime, PluginRuntimeConfig,
};
use ora_process::TokioProcessSpawner;
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

/// Adapts the JSON-RPC plugin runtime to lifecycle stop and exit observation.
#[derive(Clone)]
pub struct DenoPluginRuntime {
    runtime: ProcessPluginRuntime,
}

impl PluginRuntimeLauncher for DenoPluginRuntimeLauncher {
    type Runtime = DenoPluginRuntime;

    /// Starts Deno and waits for the plugin registration handshake before reporting readiness.
    fn launch(
        &self,
        request: PluginLaunchRequest,
    ) -> impl Future<Output = Result<Self::Runtime, PluginRuntimeFailure>> + Send {
        let timeouts = self.timeouts;
        async move {
            ProcessPluginRuntime::launch(
                &TokioProcessSpawner::new(),
                PluginRuntimeConfig {
                    plugin_id: request.plugin_id.to_string(),
                    deno_path: request.deno_path,
                    entrypoint: request.entrypoint,
                    permissions: Vec::new(),
                    ready_timeout: timeouts.ready,
                    call_timeout: timeouts.call,
                    shutdown_timeout: timeouts.shutdown,
                },
            )
            .await
            .map(|(runtime, _notifications)| DenoPluginRuntime { runtime })
            .map_err(|error| PluginRuntimeFailure::new(error.to_string()))
        }
    }
}

impl PluginRuntime for DenoPluginRuntime {
    /// Requests shutdown and waits until the complete supervised process tree exits.
    fn stop(&self) -> impl Future<Output = Result<(), PluginRuntimeFailure>> + Send {
        let runtime = self.runtime.clone();
        async move {
            runtime.shutdown();
            runtime.wait_for_exit().await;
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
}
