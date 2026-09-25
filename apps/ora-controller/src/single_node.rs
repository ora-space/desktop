use super::*;
use ora_utils::process::{LinuxPidFd, ProcessSignal};
use std::path::Path;
use std::{
    fs, io,
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};
use tokio::net::UnixStream;

/// A Node started by this Controller. It shares the process group so an operator's group-level stop
/// reaches both, but nothing ties its lifetime to this process: Controller death alone signals nothing.
pub struct ManagedNode {
    child: Child,
    handle: LinuxPidFd,
    stop_timeout: Duration,
}

/// A validated hosting request; nothing has been started and no Controller state has been opened yet.
pub struct NodeLaunch<'a> {
    config: &'a SingleNodeConfig,
    node_id: &'a NodeId,
    endpoint: &'a Path,
}

impl ManagedNode {
    /// Cross-checks the Node deployment read-only and refuses a live endpoint before any state opens.
    pub async fn prepare<'a>(
        config: &'a SingleNodeConfig,
        runtime: &'a RuntimeConfig,
    ) -> Result<NodeLaunch<'a>, Error> {
        let target = runtime.nodes.first().ok_or_else(|| {
            Error::Configuration("single_node requires one configured Node".into())
        })?;
        // Hosting starts a local process, so only a local socket can be the Node it reaches.
        let NodeEndpoint::Ipc { path: endpoint } = &target.endpoint else {
            return Err(Error::Configuration(
                "single_node requires the configured Node to use an ipc endpoint".into(),
            ));
        };
        // The Node owns its configuration; only confirm it binds this Controller at this endpoint.
        let node: serde_json::Value = serde_json::from_slice(&fs::read(&config.node_config)?)?;
        let control = &node["control"];
        if control["controller_id"].as_str() != Some(runtime.controller_id.as_str())
            || control["listen"]["kind"].as_str() != Some("ipc")
            || control["listen"]["path"].as_str().map(Path::new) != Some(endpoint.as_path())
        {
            return Err(Error::Configuration(
                "Node configuration does not bind this Controller at the configured endpoint"
                    .into(),
            ));
        }
        if UnixStream::connect(endpoint).await.is_ok() {
            return Err(Error::Configuration(
                "a Node already listens on the configured endpoint; stop it or run without --single-node".into(),
            ));
        }
        Ok(NodeLaunch {
            config,
            node_id: &target.node_id,
            endpoint,
        })
    }

    /// Resolves only when the Node exits on its own; the composition then loses its execution environment.
    pub async fn exited(&mut self) -> io::Result<ExitStatus> {
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Ok(status);
            }
            tokio::time::sleep(Duration::from_millis(/*millis*/ 100)).await;
        }
    }

    /// Requests normal Node shutdown and waits within the configured bound; never escalates to SIGKILL,
    /// because forcing the Node past its managed-Git cleanup would hand containment to host/guardian.
    pub async fn stop(mut self) -> io::Result<()> {
        if self.child.try_wait()?.is_some() {
            return Ok(());
        }
        // The Node may exit between the check and the signal; only a still-running Node is an error.
        if let Err(error) = self.handle.signal(ProcessSignal::Terminate)
            && self.child.try_wait()?.is_none()
        {
            return Err(error);
        }
        let deadline = Instant::now() + self.stop_timeout;
        loop {
            if let Some(status) = self.child.try_wait()? {
                return if status.success() {
                    Ok(())
                } else {
                    Err(io::Error::other(format!(
                        "managed Node did not stop cleanly: {status}"
                    )))
                };
            }
            if Instant::now() >= deadline {
                ora_logging::ora_warn!(
                    "managed Node did not stop within stop_timeout_ms; leaving it to host/guardian containment"
                );
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(/*millis*/ 25)).await;
        }
    }
}

impl NodeLaunch<'_> {
    /// Starts the Node once the Controller owns its state, then waits for the endpoint to accept.
    pub async fn start(self) -> Result<ManagedNode, Error> {
        let NodeLaunch {
            config,
            node_id,
            endpoint,
        } = self;
        let mut child = Command::new(&config.node_executable)
            .arg(&config.node_config)
            .stdin(Stdio::null())
            .spawn()
            .map_err(|error| {
                Error::Configuration(format!(
                    "cannot start {}: {error}",
                    config.node_executable.display()
                ))
            })?;
        let handle = LinuxPidFd::for_child(&child)?;
        let deadline = Instant::now() + Duration::from_millis(config.ready_timeout_ms);
        loop {
            if let Some(status) = child.try_wait()? {
                return Err(Error::Configuration(format!(
                    "Node exited before becoming ready: {status}"
                )));
            }
            if UnixStream::connect(endpoint).await.is_ok() {
                break;
            }
            if Instant::now() >= deadline {
                // Do not enter admission with an unverified Node; a stop request is the only cleanup.
                let _ = handle.signal(ProcessSignal::Terminate);
                let _ = child.wait();
                return Err(Error::Configuration(
                    "Node did not become ready within ready_timeout_ms".into(),
                ));
            }
            tokio::time::sleep(Duration::from_millis(/*millis*/ 25)).await;
        }
        ora_logging::ora_info!(node_id = %node_id.as_str(), "managed Node ready");
        Ok(ManagedNode {
            child,
            handle,
            stop_timeout: Duration::from_millis(config.stop_timeout_ms),
        })
    }
}
