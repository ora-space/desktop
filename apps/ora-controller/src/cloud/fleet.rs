//! The Node sessions a cloud Controller holds with Workspace sandboxes. Unlike the static `nodes`,
//! these targets are never configured: each one is derived from Cloud's record of a live sandbox
//! of the current generation (its `sandbox_ensure` evidence) and the deployment's router, and it
//! lives until the terminate step stops it or Cloud reports the sandbox terminated. The Controller
//! keeps no local record of them; a new process rebuilds them from Cloud's list of live sandboxes
//! when it first holds the lease, and from the snapshots it claims.
use super::{
    CloudStore,
    reports::{self, Reported},
    substrate::Substrate,
};
use crate::{
    session::{SessionObserver, run_observed_session},
    *,
};
use ora_controller_proto::v1 as proto;
use ora_node_transport::websocket::WsEndpoint;
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};
use tokio::{
    sync::{self, watch},
    task::JoinHandle,
    time::{MissedTickBehavior, interval},
};

/// The routing header naming the sandbox, as the Sandbox Server and the platform router expect.
const TARGET_HEADER: &str = "ate-target-actor";
/// Cloud treats a Node silent for 30 seconds as not ready; reporting every 10 leaves two misses.
const REPORT_INTERVAL: Duration = Duration::from_secs(/*secs*/ 10);

/// Deployment facts every sandbox session and effect call needs.
pub(super) struct SandboxDeployment {
    pub(super) substrate: Substrate,
    router_url: String,
    atespace: String,
    session: SessionConfig,
    reconnect: Duration,
    /// The static Node of tenant clones; a sandbox claiming the same NodeId is refused.
    static_node: Option<NodeId>,
    tls: Option<ora_node_transport::mtls::MutualTlsFiles>,
}

impl SandboxDeployment {
    /// Validates the Substrate and router configuration before anything runs.
    pub(super) fn new(config: &SubstrateConfig, runtime: &RuntimeConfig) -> Result<Self, Error> {
        let atespace = config.atespace.trim();
        if atespace.is_empty() || atespace.contains('/') {
            return Err(Error::Configuration(
                "substrate.atespace must be non-empty and contain no '/'".into(),
            ));
        }
        let probe = WsEndpoint {
            tls: runtime.management_tls.clone(),
            url: config.router_url.clone(),
            headers: BTreeMap::from([(TARGET_HEADER.to_owned(), format!("{atespace}/probe"))]),
        };
        probe.validate().map_err(|error| {
            Error::Configuration(format!("invalid substrate.router_url: {error}"))
        })?;
        Ok(Self {
            substrate: Substrate::with_tls(config, runtime.management_tls.as_ref())?,
            router_url: config.router_url.clone(),
            atespace: atespace.to_owned(),
            session: runtime.session.clone(),
            reconnect: Duration::from_millis(runtime.reconnect_ms),
            static_node: runtime.nodes.first().map(|node| node.node_id.clone()),
            tls: runtime.management_tls.clone(),
        })
    }

    /// The only endpoint a sandbox Node is reached at: the router plus the header naming it.
    fn target(&self, binding: &Binding) -> NodeTarget {
        NodeTarget {
            node_id: binding.node_id.clone(),
            endpoint: NodeEndpoint::WebSocket(WsEndpoint {
                url: self.router_url.clone(),
                tls: self.tls.clone(),
                headers: BTreeMap::from([(
                    TARGET_HEADER.to_owned(),
                    format!("{}/{}", self.atespace, binding.external_id),
                )]),
            }),
        }
    }
}

/// What Cloud says about one live sandbox of the current generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Binding {
    /// Cloud's sandbox instance ID, which is also its ensure effect ID.
    pub(super) sandbox_id: String,
    pub(super) generation: i64,
    /// The Node identity the ensure effect reported; the handshake must present it.
    pub(super) node_id: NodeId,
    /// The Substrate's identity of the sandbox, which the router header names.
    pub(super) external_id: String,
}

/// Whether new work may still be dispatched to a sandbox's Node.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Gate {
    Open,
    /// Quiesce stopped dispatching before it checked for unfinished work.
    Closed,
}

/// Why an execution remains unresolved across transport reconnects.
enum Unresolved {
    Unknown(Instant),
    /// Status hints on a later connection cannot stand in for the missing takeover receipt.
    TerminalRefused(Instant),
}

impl Unresolved {
    /// Preserve the first unresolved observation while strengthening its reason.
    fn since(&self) -> Instant {
        match self {
            Self::Unknown(since) | Self::TerminalRefused(since) => *since,
        }
    }
}

/// State one sandbox's session, reporter and operation steps share.
pub(super) struct Sandbox {
    pub(super) binding: Binding,
    identity: watch::Sender<Option<NodeRuntimeIdentity>>,
    work_hint: Arc<tokio::sync::Notify>,
    pub(super) report: sync::Mutex<Reported>,
    /// Held across a clone registration and while quiesce closes it, so a registration either
    /// committed before quiesce lists unfinished work or never happens.
    pub(super) gate: sync::Mutex<Gate>,
    unresolved: Mutex<HashMap<ExecutionId, Unresolved>>,
    capabilities: Mutex<Vec<NodeCapability>>,
    /// The adapter facts of the same handshake; only an exact match runs a configured session.
    model_adapter: Mutex<Option<ModelAdapterCapability>>,
    /// Cloud's Node records of this sandbox, from the latest snapshot.
    known: Mutex<Vec<proto::NodeRecord>>,
}

impl Sandbox {
    /// The incarnation of the live session, or `None` between sessions.
    pub(super) fn identity(&self) -> Option<NodeRuntimeIdentity> {
        self.identity.borrow().clone()
    }

    /// Only capabilities the live session negotiated authorize new registrations: plugin work
    /// needs `plugin_install`, sessions `agent_session` (and `revision_restore` when they resume a
    /// prior Revision), deliveries `revision_delivery`.
    pub(super) fn advertises(&self, capability: NodeCapability) -> bool {
        self.connected()
            && self
                .capabilities
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .contains(&capability)
    }

    /// Registration freezes the handshake fact even if the socket closes during the write.
    pub(super) fn advertised_model_proxy(&self) -> bool {
        self.capabilities
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&NodeCapability::ModelProxy)
    }

    /// Registration reports the adapter facts of the same handshake as `model_proxy`.
    pub(super) fn advertised_model_adapter(&self) -> Option<ModelAdapterCapability> {
        self.model_adapter
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Whether the live session can run a session frozen with `configuration`. Cloud fences
    /// the same match; checking here keeps an incompatible item queued instead of refused.
    pub(super) fn admits(&self, configuration: &proto::AgentModelConfiguration) -> bool {
        let Some(adapter) = self.advertised_model_adapter() else {
            return false;
        };
        self.connected()
            && adapter.contract_version == configuration.adapter_version
            && adapter.target == configuration.target
            && ModelProtocol::parse(&configuration.protocol)
                .is_some_and(|protocol| adapter.protocols.contains(&protocol))
    }

    /// Whether a session is established right now.
    pub(super) fn connected(&self) -> bool {
        self.identity.borrow().is_some()
    }

    /// How long the Node has kept answering `Unknown` for an execution after its retransmission.
    pub(super) fn unresolved_for(&self, execution: &ExecutionId) -> Option<Duration> {
        self.unresolved
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(execution)
            .map(|state| state.since().elapsed())
    }

    /// Whether any execution on this Node has an outcome the Node could not tell.
    pub(super) fn any_unresolved(&self) -> bool {
        !self
            .unresolved
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    }

    /// Records Cloud's Node records for this sandbox so a restarted Controller can end an
    /// incarnation it registered before it lost that knowledge.
    fn observe(&self, records: Vec<proto::NodeRecord>) {
        *self.known.lock().unwrap_or_else(PoisonError::into_inner) = records;
    }

    /// Live incarnations Cloud lists for this sandbox other than `current`, as (record, version).
    pub(super) fn stale_incarnations(&self, current: &NodeIncarnationId) -> Vec<(String, i64)> {
        self.known
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|record| {
                record.connection != proto::NodeConnection::Ended as i32
                    && record
                        .identity
                        .as_ref()
                        .is_some_and(|identity| identity.node_incarnation_id != current.as_str())
            })
            .map(|record| (record.id.clone(), record.version))
            .collect()
    }
}

impl SessionObserver for Sandbox {
    fn capabilities(
        &self,
        capabilities: &[NodeCapability],
        model_adapter: Option<&ModelAdapterCapability>,
    ) {
        *self
            .capabilities
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = capabilities.to_vec();
        *self
            .model_adapter
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = model_adapter.cloned();
    }
    fn established(&self, node: &NodeRuntimeIdentity) {
        self.identity.send_replace(Some(node.clone()));
        self.work_hint.notify_one();
    }

    fn unresolved(&self, execution: &ExecutionId) {
        self.unresolved
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(execution.clone())
            .or_insert_with(|| Unresolved::Unknown(Instant::now()));
    }

    fn answered(&self, execution: &ExecutionId) {
        self.unresolved
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|id, state| id != execution || matches!(state, Unresolved::TerminalRefused(_)));
    }

    fn terminal_refused(&self, execution: &ExecutionId) {
        self.unresolved
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(execution.clone())
            .and_modify(|state| *state = Unresolved::TerminalRefused(state.since()))
            .or_insert_with(|| Unresolved::TerminalRefused(Instant::now()));
    }

    fn terminal_accepted(&self, execution: &ExecutionId) {
        self.unresolved
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(execution);
    }

    fn refused_executions(&self) -> Vec<ExecutionId> {
        self.unresolved
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(_, state)| matches!(state, Unresolved::TerminalRefused(_)))
            .map(|(id, _)| id.clone())
            .collect()
    }
}

/// One running sandbox target and the tasks it owns.
struct Entry {
    sandbox: Arc<Sandbox>,
    stop: watch::Sender<bool>,
    session: JoinHandle<()>,
    reporter: JoinHandle<()>,
}

/// The dynamic sandbox targets. Cheap to clone; clones share the same sessions.
#[derive(Clone)]
pub(super) struct Fleet {
    store: CloudStore,
    deployment: Arc<SandboxDeployment>,
    entries: Arc<Mutex<HashMap<String, Entry>>>,
}

impl Fleet {
    pub(super) fn new(store: CloudStore, deployment: Arc<SandboxDeployment>) -> Self {
        Self {
            store,
            deployment,
            entries: Arc::default(),
        }
    }

    pub(super) fn deployment(&self) -> &SandboxDeployment {
        &self.deployment
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, HashMap<String, Entry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The running target of one sandbox.
    pub(super) fn get(&self, sandbox: &str) -> Option<Arc<Sandbox>> {
        self.entries()
            .get(sandbox)
            .map(|entry| entry.sandbox.clone())
    }

    /// The sandboxes with a running target.
    pub(super) fn sandboxes(&self) -> Vec<String> {
        self.entries().keys().cloned().collect()
    }

    /// Starts the session and reporter of a live sandbox, unless they already run. A NodeId the
    /// static Node already uses is refused: with two sources nobody could say which one is the
    /// fact, so neither is replaced.
    pub(super) fn ensure(&self, binding: Binding, records: Vec<proto::NodeRecord>) {
        if self.deployment.static_node.as_ref() == Some(&binding.node_id) {
            ora_logging::ora_warn!(
                sandbox_instance_id = %binding.sandbox_id,
                node_id = %binding.node_id.as_str(),
                "sandbox Node identity collides with the static Node; not connecting to it"
            );
            return;
        }
        let mut entries = self.entries();
        if let Some(entry) = entries.get(&binding.sandbox_id) {
            if entry.sandbox.binding != binding {
                ora_logging::ora_warn!(
                    sandbox_instance_id = %binding.sandbox_id,
                    "Cloud reports a different binding for a connected sandbox; keeping the session"
                );
            }
            entry.sandbox.observe(records);
            return;
        }
        let sandbox = Arc::new(Sandbox {
            binding: binding.clone(),
            identity: watch::Sender::new(None),
            work_hint: self.store.inner.work_hint.clone(),
            report: sync::Mutex::default(),
            gate: sync::Mutex::new(Gate::Open),
            unresolved: Mutex::default(),
            capabilities: Mutex::default(),
            model_adapter: Mutex::default(),
            known: Mutex::new(records),
        });
        let (stop, stopping) = watch::channel(false);
        let session = tokio::spawn(connect(
            self.store.clone(),
            self.deployment.clone(),
            sandbox.clone(),
            stopping,
        ));
        let reporter = tokio::spawn(report(self.store.clone(), sandbox.clone()));
        ora_logging::ora_info!(
            sandbox_instance_id = %binding.sandbox_id,
            node_id = %binding.node_id.as_str(),
            "sandbox Node target added"
        );
        entries.insert(
            binding.sandbox_id,
            Entry {
                sandbox,
                stop,
                session,
                reporter,
            },
        );
    }

    /// Stops a sandbox's session and removes its target; returns once the session ended, so a
    /// caller may terminate the sandbox afterwards. The reporter stops first, so the deliberate end
    /// is never reported to Cloud as a lost connection.
    pub(super) async fn remove(&self, sandbox: &str) {
        let Some(entry) = self.entries().remove(sandbox) else {
            return;
        };
        entry.reporter.abort();
        let _ = entry.stop.send(true);
        let closing =
            Duration::from_millis(self.deployment.session.io_timeout_ms).saturating_mul(2);
        let mut session = entry.session;
        if tokio::time::timeout(closing, &mut session).await.is_err() {
            session.abort();
            let _ = session.await;
        }
        ora_logging::ora_info!(sandbox_instance_id = %sandbox, "sandbox Node target removed");
    }

    /// Stops every sandbox session, as on shutdown. Sandboxes keep running; a successor Controller
    /// reconnects to them.
    pub(super) async fn shutdown(&self) {
        for sandbox in self.sandboxes() {
            self.remove(&sandbox).await;
        }
    }
}

/// Keeps one sandbox's session up until asked to stop, reconnecting after every loss. A router that
/// cannot find the sandbox is only logged: whether a sandbox exists is Cloud's record, and only a
/// new operation may create another one.
async fn connect(
    store: CloudStore,
    deployment: Arc<SandboxDeployment>,
    sandbox: Arc<Sandbox>,
    mut stopping: watch::Receiver<bool>,
) {
    let target = deployment.target(&sandbox.binding);
    loop {
        let stop = {
            let mut stopping = stopping.clone();
            async move {
                let _ = stopping.wait_for(|stop| *stop).await;
            }
        };
        let result =
            run_observed_session(&store, &target, &deployment.session, stop, &*sandbox).await;
        sandbox.identity.send_replace(None);
        if let Err(error) = result {
            ora_logging::ora_warn!(
                sandbox_instance_id = %sandbox.binding.sandbox_id,
                node_id = %target.node_id.as_str(),
                error = %error,
                "sandbox Node connection unavailable; retrying"
            );
        }
        tokio::select! {
            _ = stopping.wait_for(|stop| *stop) => return,
            () = tokio::time::sleep(deployment.reconnect) => {}
        }
    }
}

/// Reports the session to Cloud on every change and every `REPORT_INTERVAL`.
async fn report(store: CloudStore, sandbox: Arc<Sandbox>) {
    let mut changes = sandbox.identity.subscribe();
    let mut tick = interval(REPORT_INTERVAL);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            changed = changes.changed() => {
                if changed.is_err() {
                    return;
                }
            }
            _ = tick.tick() => {}
        }
        reports::sync(&store, &sandbox).await;
    }
}

#[cfg(test)]
mod non_acceptance_tests;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn runtime() -> RuntimeConfig {
        RuntimeConfig {
            management_tls: None,
            home_directory: "/nonexistent/controller".into(),
            persistence: Persistence::Sqlite,
            protected_state_directories: Vec::new(),
            controller_id: ControllerId::new("owner"),
            nodes: Vec::new(),
            session: SessionConfig {
                io_timeout_ms: 100,
                query_interval_ms: 10,
            },
            reconnect_ms: 10,
            timezone: "Asia/Shanghai".into(),
        }
    }

    fn substrate(atespace: &str) -> SubstrateConfig {
        SubstrateConfig {
            effects_url: "http://sandbox-server:18001".into(),
            router_url: "ws://sandbox-server:18000/ora-node/v1".into(),
            atespace: atespace.into(),
            request_timeout_ms: 1000,
        }
    }

    /// A sandbox Node is reached only through the configured router, with the header naming the
    /// sandbox by the Substrate's identity and the NodeId the ensure effect reported.
    #[test]
    fn sandbox_targets_come_from_the_router_and_the_ensure_evidence() {
        let deployment = SandboxDeployment::new(&substrate("local"), &runtime()).unwrap();
        let binding = Binding {
            sandbox_id: "sandbox".into(),
            generation: 2,
            node_id: NodeId::new("workspace-w"),
            external_id: "external".into(),
        };
        let target = deployment.target(&binding);
        assert_eq!(target.node_id, NodeId::new("workspace-w"));
        assert_eq!(
            target.endpoint,
            NodeEndpoint::WebSocket(WsEndpoint {
                tls: None,
                url: "ws://sandbox-server:18000/ora-node/v1".into(),
                headers: BTreeMap::from([("ate-target-actor".into(), "local/external".into())]),
            })
        );
    }

    /// Management TLS authenticates the shared router; sandbox identities stay in routing headers.
    #[test]
    fn tls_targets_share_the_router_without_deriving_node_hostnames() {
        let root = tempfile::tempdir().unwrap();
        let key = rcgen::KeyPair::generate().unwrap();
        let mut parameters = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        parameters.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        parameters.key_usages = vec![rcgen::KeyUsagePurpose::KeyCertSign];
        let ca = rcgen::CertifiedIssuer::self_signed(parameters, key).unwrap();
        let key = rcgen::KeyPair::generate().unwrap();
        let mut parameters = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        parameters.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ClientAuth];
        let certificate = parameters.signed_by(&key, &ca).unwrap();
        let files = ora_node_transport::mtls::MutualTlsFiles {
            certificate_file: root.path().join("controller.pem"),
            private_key_file: root.path().join("controller.key"),
            ca_file: root.path().join("ca.pem"),
            peer_certificate_file: None,
        };
        std::fs::write(&files.certificate_file, certificate.pem()).unwrap();
        std::fs::write(&files.private_key_file, key.serialize_pem()).unwrap();
        std::fs::write(&files.ca_file, ca.pem()).unwrap();
        let mut runtime = runtime();
        runtime.management_tls = Some(files.clone());
        let config = SubstrateConfig {
            effects_url: "https://platform:18001".into(),
            router_url: "wss://platform:18000/ora-node/v1".into(),
            ..substrate("local")
        };
        let deployment = SandboxDeployment::new(&config, &runtime).unwrap();
        for sandbox in ["first", "second"] {
            let target = deployment.target(&Binding {
                sandbox_id: sandbox.into(),
                generation: 1,
                node_id: NodeId::new(format!("workspace-{sandbox}")),
                external_id: sandbox.into(),
            });
            assert_eq!(
                target.endpoint,
                NodeEndpoint::WebSocket(WsEndpoint {
                    url: config.router_url.clone(),
                    tls: Some(files.clone()),
                    headers: BTreeMap::from([(
                        "ate-target-actor".into(),
                        format!("local/{sandbox}")
                    )]),
                })
            );
        }
        let plain = SubstrateConfig {
            router_url: "ws://platform/ora-node/v1".into(),
            ..config
        };
        assert!(matches!(
            SandboxDeployment::new(&plain, &runtime),
            Err(Error::Configuration(_))
        ));
    }

    /// An atespace that would change the header's shape is a deployment error.
    #[test]
    fn an_atespace_with_a_slash_is_refused() {
        assert!(matches!(
            SandboxDeployment::new(&substrate("a/b"), &runtime()),
            Err(Error::Configuration(_))
        ));
    }
}
