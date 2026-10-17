//! Injected binding authority for real TLS/Node/Git regressions, not PostgreSQL authorization evidence.
use super::*;
use ora_controller::{CoordinationStore, ExecutionOutcome, SqliteStore};
use ora_node_transport::{mtls::MutualTlsFiles, websocket::WsEndpoint};
use pretty_assertions::assert_eq;
use std::{
    ops::Deref,
    path::Path,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

/// Creates ephemeral test-only credentials without sharing the private issuer with workloads.
pub(super) fn tls(root: &Path, peer: &str) -> MutualTlsFiles {
    let ca_path = root.join("management-ca.pem");
    if !ca_path.exists() {
        let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let key = rcgen::KeyPair::generate().unwrap();
        let ca = params.self_signed(&key).unwrap();
        fs::write(&ca_path, ca.pem()).unwrap();
        let issuer = rcgen::Issuer::from_params(&params, key);
        for name in ["node", "controller"] {
            let mut p = rcgen::CertificateParams::new(vec!["localhost".into()]).unwrap();
            p.extended_key_usages = vec![if name == "node" {
                rcgen::ExtendedKeyUsagePurpose::ServerAuth
            } else {
                rcgen::ExtendedKeyUsagePurpose::ClientAuth
            }];
            let k = rcgen::KeyPair::generate().unwrap();
            let cert = p.signed_by(&k, &issuer).unwrap();
            fs::write(root.join(format!("management-{name}.pem")), cert.pem()).unwrap();
            fs::write(
                root.join(format!("management-{name}.key")),
                k.serialize_pem(),
            )
            .unwrap();
        }
    }
    MutualTlsFiles {
        certificate_file: root.join(format!("management-{peer}.pem")),
        private_key_file: root.join(format!("management-{peer}.key")),
        ca_file: ca_path,
        peer_certificate_file: Some(root.join("management-controller.pem")),
    }
}
/// Uses authenticated loopback transport while retaining actual process and Git boundaries.
pub(super) fn endpoint(fixture: &Fixture, bind: std::net::SocketAddr) -> WsEndpoint {
    WsEndpoint {
        url: format!("wss://localhost:{}{}", bind.port(), websocket::PATH),
        headers: std::collections::BTreeMap::new(),
        tls: Some(tls(fixture.path(), "controller")),
    }
}
/// Supplies a fixed test target; this fixture never substitutes for Cloud authorization.
pub(super) fn scope() -> ora_node::RuntimeScope {
    ora_node::RuntimeScope {
        tenant_id: "test-tenant".into(),
        workspace_id: "test-workspace".into(),
        sandbox_id: "test-sandbox".into(),
        runtime_generation: 1,
    }
}
/// Models a fresh authority decision after restart without reviving an old qualification.
pub(super) fn advance_epoch(root: &Path) {
    let p = root.join("control-epoch");
    let epoch = fs::read_to_string(&p)
        .map(|s| s.parse::<i64>().unwrap())
        .unwrap_or(0)
        + 1;
    fs::write(p, epoch.to_string()).unwrap();
}
/// Binds the real handshake identity to a short-lived test authority decision.
pub(super) fn binding(root: &Path, node: &NodeRuntimeIdentity) -> RuntimeBinding {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let epoch = fs::read_to_string(root.join("control-epoch"))
        .unwrap()
        .parse()
        .unwrap();
    RuntimeBinding {
        tenant_id: "test-tenant".into(),
        workspace_id: "test-workspace".into(),
        sandbox_id: "test-sandbox".into(),
        runtime_generation: 1,
        node_id: node.node_id.as_str().into(),
        node_incarnation_id: node.incarnation_id.as_str().into(),
        node_instance_id: format!("instance-{}", node.incarnation_id.as_str()),
        controller_epoch: 1,
        control_epoch: epoch,
        control_version: epoch,
        session_id: "test-page".into(),
        actor_user_id: "test-actor".into(),
        operation_id: String::new(),
        execution_id: String::new(),
        node_operation_id: String::new(),
        input_closed: false,
        issued_at_ms: now,
        expires_at_ms: now + 30_000,
    }
}
/// Names the exact execution being dispatched through the production controlled entrance.
pub(super) fn controlled(
    mut permit: RuntimeBinding,
    command: CloneRepositoryMessage,
) -> ControllerToNodeMessage {
    permit.execution_id = command.execution_id.as_str().into();
    permit.node_operation_id = command.operation_id.as_str().into();
    ControllerToNodeMessage::ControlledClone(ControlledClone {
        binding: permit,
        command,
    })
}
#[derive(Clone)]
pub(super) struct RuntimeStore {
    inner: SqliteStore,
    root: PathBuf,
    current: Arc<Mutex<Option<(NodeRuntimeIdentity, bool)>>>,
}
impl RuntimeStore {
    /// Seeds pre-existing coordination evidence; no product intake is exposed.
    pub(super) async fn seed_clone(
        &self,
        request: RequestId,
        spec: CloneExecutionSpec,
    ) -> Result<CloneRepositoryMessage, ora_controller::Error> {
        let suffix = request.as_str();
        Ok(seed_dispatch(
            &self.root.join("controller"),
            CloneRepositoryMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                request_id: None,
                operation_id: OperationId::new(format!("op-{suffix}")),
                execution_id: ExecutionId::new(format!("exec-{suffix}")),
                payload: CloneRepository { spec },
            },
            request,
        ))
    }
    pub(super) fn new(inner: SqliteStore, root: &Path) -> Self {
        Self {
            inner,
            root: root.into(),
            current: Arc::default(),
        }
    }
}
impl Deref for RuntimeStore {
    type Target = SqliteStore;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl CoordinationStore for RuntimeStore {
    fn id(&self) -> &ControllerId {
        self.inner.id()
    }
    fn requires_runtime_control(&self) -> bool {
        true
    }
    async fn runtime_bindings(
        &self,
        _node: &NodeId,
    ) -> Result<Vec<RuntimeBinding>, ora_controller::Error> {
        Ok(self
            .current
            .lock()
            .unwrap()
            .as_ref()
            .map(|(node, _)| vec![binding(&self.root, node)])
            .unwrap_or_default())
    }
    async fn acknowledge_runtime_binding(
        &self,
        state: &RuntimeControlState,
    ) -> Result<(), ora_controller::Error> {
        // Renewal keeps the same accepted responsibility; activity does not invalidate the binding.
        // The real Node independently rejects a second conflicting execution.
        assert_eq!(state.binding.workspace_id, "test-workspace");
        if let Some((_, confirmed)) = self.current.lock().unwrap().as_mut() {
            *confirmed = true
        };
        Ok(())
    }
    async fn dispatch_message(
        &self,
        command: CloneRepositoryMessage,
    ) -> Result<Option<ControllerToNodeMessage>, ora_controller::Error> {
        Ok(self
            .current
            .lock()
            .unwrap()
            .as_ref()
            .filter(|(_, confirmed)| *confirmed)
            .map(|(node, _)| controlled(binding(&self.root, node), command)))
    }
    async fn take_over_node_event(
        &self,
        session: &NodeRuntimeIdentity,
        event: &CloneResultMessage,
    ) -> Result<(), ora_controller::Error> {
        self.inner.take_over_node_event(session, event).await
    }
    async fn record_queried_result(
        &self,
        session: &NodeRuntimeIdentity,
        operation: &OperationId,
        execution: &ExecutionId,
        result: &CloneExecutionResult,
    ) -> Result<(), ora_controller::Error> {
        self.inner
            .record_queried_result(session, operation, execution, result)
            .await
    }
    async fn original_dispatch(
        &self,
        session: &NodeRuntimeIdentity,
        operation: &OperationId,
        execution: &ExecutionId,
    ) -> Result<CloneRepositoryMessage, ora_controller::Error> {
        let original = self
            .inner
            .original_dispatch(session, operation, execution)
            .await?;
        let mut current = self.current.lock().unwrap();
        if current.as_ref().is_none_or(|(node, _)| node != session) {
            *current = Some((session.clone(), false))
        };
        Ok(original)
    }
    async fn pending_dispatches(
        &self,
        node: &NodeId,
    ) -> Result<Vec<CloneRepositoryMessage>, ora_controller::Error> {
        self.inner.pending_dispatches(node).await
    }
    async fn result(
        &self,
        execution: &ExecutionId,
    ) -> Result<Option<ExecutionOutcome>, ora_controller::Error> {
        self.inner.result(execution).await
    }
    fn static_node_established(&self, node: &NodeRuntimeIdentity) {
        self.inner.static_node_established(node)
    }
    async fn serve(
        &self,
        shutdown: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> std::io::Result<()> {
        self.inner.serve(shutdown).await
    }
}
