//! One blocking database owner, a clone executor that waits on Git for it, and an independently
//! responsive, bounded control session.
mod agents;
mod clones;
mod delivery;
mod executor;
mod plugins;
mod revisions;
mod session;
mod worker;
use crate::{CloneConfig, NodeConfig, ProcessConfig, Shutdown};
use ora_node_protocol::*;
use serde::{Deserialize, Serialize};
use std::{
    io,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};
use tokio::sync::oneshot;

/// Production network sessions authenticate a pinned Controller and a platform-assigned scope.
/// Local IPC and loopback transport fixtures do not establish cloud user authority.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlConfig {
    /// Platform-assigned target; a valid Controller certificate cannot choose another tenant.
    #[serde(default)]
    pub target: Option<RuntimeScope>,
    pub controller_id: ControllerId,
    pub listen: ControlListen,
    pub heartbeat_ms: u64,
    pub frame_timeout_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeScope {
    pub tenant_id: String,
    pub workspace_id: String,
    pub sandbox_id: String,
    pub runtime_generation: i64,
}
impl RuntimeScope {
    fn permits(&self, binding: &ora_node_protocol::RuntimeBinding) -> bool {
        self.tenant_id == binding.tenant_id
            && self.workspace_id == binding.workspace_id
            && self.sandbox_id == binding.sandbox_id
            && self.runtime_generation == binding.runtime_generation
    }
}

/// Where the Node accepts its Controller. Local deployments use a private Unix socket under the
/// Node home; sandboxes use mutually authenticated TLS. The plaintext variant is retained only
/// for loopback transport fixtures and is rejected by the production executable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ControlListen {
    #[serde(rename = "ipc")]
    Ipc { path: PathBuf },
    #[serde(rename = "websocket")]
    WebSocket { bind: SocketAddr, path: String },
    #[serde(rename = "mutual_tls_websocket")]
    MutualTlsWebSocket {
        bind: SocketAddr,
        path: String,
        tls: ora_node_transport::mtls::MutualTlsFiles,
    },
}

/// Composition keeps deployment paths separate from business requests and supports recovery-only startup.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceConfig {
    /// Plugin transfer timing is selected by deployment and defaults to the original policy.
    #[serde(default)]
    pub plugins: crate::PluginConfig,
    /// Enables Agent sessions using the deployment-provided Deno executable.
    #[serde(default)]
    pub agent: Option<AgentConfig>,
    pub node: NodeConfig,
    pub process: ProcessConfig,
    #[serde(default)]
    pub clone: Option<CloneConfig>,
    #[serde(default)]
    pub control: Option<ControlConfig>,
    pub recovery_interval_ms: u64,
    pub timezone: String,
}

/// Agent process configuration belongs to deployment, never to a remote start request.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    pub deno_path: PathBuf,
    pub ready_timeout_ms: u64,
    /// Dedicated client-authentication material, distinct from Node WSS server TLS.
    #[serde(default)]
    pub model_proxy: Option<crate::ModelProxyConfig>,
    /// Root-owned directory holding one directory per running session when agents run as the
    /// workload user; required exactly when `process.workload_uid` is set. The deployment creates
    /// it; the Node only validates it and manages its children.
    #[serde(default)]
    pub workload_directory: Option<PathBuf>,
}

/// Requests waiting for the blocking worker, across message handling and the replay pass.
const ADMISSION_QUEUE_BOUND: usize = 16;

// The queue holds at most ADMISSION_QUEUE_BOUND entries; inline envelopes avoid another allocation per control message.
#[allow(clippy::large_enum_variant)]
enum Request {
    Message(ControllerToNodeMessage),
    Replay(Vec<ora_node_db::EventCursor>),
}
struct Work {
    active: Arc<Mutex<bool>>,
    request: Request,
    reply: oneshot::Sender<Result<Vec<NodeToControllerMessage>, Rejection>>,
}
/// Why the worker refused a request, and the close code the session ends with because of it.
struct Rejection {
    close: ora_node_transport::CloseReason,
    message: String,
}
#[derive(Clone)]
struct SessionInfo {
    agents: Option<agents::SessionHost>,
    /// Upload grants bypass admission and land here; grant requests leave through the session.
    grants: crate::revision::GrantStore,
    identity: NodeRuntimeIdentity,
    controller: ControllerId,
    capabilities: Vec<NodeCapability>,
}

struct StopOnDrop(Shutdown);
impl Drop for StopOnDrop {
    /// Canceling the service future must also release its blocking owner and managed process scopes.
    fn drop(&mut self) {
        self.0.request();
    }
}

/// Runs independent control-session and execution lifecycles while retaining the Node lease until cleanup finishes.
pub async fn serve(config: ServiceConfig, shutdown: Shutdown) -> io::Result<()> {
    let _stop = StopOnDrop(shutdown.clone());
    if config.recovery_interval_ms == 0 {
        return Err(io::Error::other("recovery interval must be positive"));
    }
    if let Some(control) = &config.control {
        let listen_valid = match &control.listen {
            ControlListen::Ipc { path } => {
                path.is_absolute() && path.parent() == Some(config.node.home_directory.as_path())
            }
            ControlListen::WebSocket { path, .. }
            | ControlListen::MutualTlsWebSocket { path, .. } => path.starts_with('/'),
        };
        if control.heartbeat_ms == 0
            || control.frame_timeout_ms <= control.heartbeat_ms
            || !listen_valid
            || config.clone.is_none()
        {
            return Err(io::Error::other(
                "control needs clone configuration, positive bounded timing, and an IPC path directly under Node home or a WebSocket path starting with /",
            ));
        }
    }
    if let Some(agent) = &config.agent {
        agents::validate(agent, &config)?;
    }
    if let Some(proxy) = config
        .agent
        .as_ref()
        .and_then(|agent| agent.model_proxy.as_ref())
    {
        proxy.validate().map_err(io::Error::other)?;
    }
    let control = config.control.clone();
    let (sender, receiver) = mpsc::sync_channel(ADMISSION_QUEUE_BOUND);
    let (ready, started) = oneshot::channel();
    let worker_shutdown = shutdown.clone();
    let mut worker =
        tokio::task::spawn_blocking(move || worker::run(config, receiver, ready, worker_shutdown));
    let startup = started.await.map_err(io::Error::other)?;
    let result = match startup {
        Ok(info) => match control {
            Some(control) => {
                let session = session::serve(control, info, sender, shutdown.clone());
                tokio::pin!(session);
                tokio::select! {
                    result = &mut session => result,
                    result = &mut worker => {
                        // A normal stop can finish the worker first; the session then only tells
                        // the Controller it is stopping, bounded by its frame deadline.
                        if shutdown.requested() {
                            let _ = session.await;
                        }
                        return result.map_err(io::Error::other)?.map_err(io::Error::other);
                    }
                }
            }
            None => {
                while !shutdown.requested() && !worker.is_finished() {
                    tokio::time::sleep(Duration::from_millis(/*millis*/ 25)).await;
                }
                Ok(())
            }
        },
        Err(error) => Err(io::Error::other(error)),
    };
    shutdown.request();
    worker
        .await
        .map_err(io::Error::other)?
        .map_err(io::Error::other)?;
    result
}
