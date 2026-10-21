//! In-process authority exercising production adapters over the generated gRPC contract.
use super::*;
use pretty_assertions::assert_eq;
use proto::agent_run_service_server::{AgentRunService, AgentRunServiceServer};
use proto::execution_service_server::{ExecutionService, ExecutionServiceServer};
use std::sync::{Arc, Mutex};
use tonic::{Request, Response, Status};

#[derive(Clone, Default)]
pub(super) struct Server(Arc<Mutex<State>>);
#[derive(Default)]
struct State {
    previews: Vec<proto::PublishThreadPreviewRequest>,
    events: Vec<proto::TakeOverThreadEventsRequest>,
    failure: Option<tonic::Code>,
    ended: bool,
}
impl Server {
    /// Returns received requests, including rejected attempts.
    pub(super) fn previews(&self) -> Vec<proto::PublishThreadPreviewRequest> {
        self.0.lock().unwrap().previews.clone()
    }
    /// Returns durable calls separately so previews cannot masquerade as receipts.
    pub(super) fn events(&self) -> Vec<proto::TakeOverThreadEventsRequest> {
        self.0.lock().unwrap().events.clone()
    }
    /// Injects one remote verdict.
    pub(super) fn fail(&self, code: tonic::Code) {
        self.0.lock().unwrap().failure = Some(code);
    }
    /// Marks the registered execution terminal.
    pub(super) fn end(&self) {
        self.0.lock().unwrap().ended = true;
    }
}

pub(super) struct Fixture {
    pub(super) store: CloudStore,
    pub(super) server: Server,
    pub(super) node: NodeRuntimeIdentity,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    /// Cancels the test-owned listener even on assertion failure.
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    /// Binds before returning, so tests never rely on sleeps to await server readiness.
    pub(super) async fn new() -> Self {
        let server = Server::default();
        let incoming =
            tonic::transport::server::TcpIncoming::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let endpoint = format!("http://{}", incoming.local_addr().unwrap());
        let services = tonic::transport::Server::builder()
            .add_service(AgentRunServiceServer::new(server.clone()))
            .add_service(ExecutionServiceServer::new(server.clone()));
        let task = tokio::spawn(async move {
            services.serve_with_incoming(incoming).await.unwrap();
        });
        let store = CloudStore::open(&RuntimeConfig {
            management_tls: None,
            home_directory: std::path::PathBuf::new(),
            persistence: Persistence::Cloud {
                endpoint,
                claim_interval_ms: 100,
                substrate: Some(SubstrateConfig {
                    effects_url: "http://127.0.0.1:1".into(),
                    router_url: "ws://127.0.0.1:1/node".into(),
                    atespace: "test".into(),
                    request_timeout_ms: 1000,
                }),
            },
            protected_state_directories: vec![],
            controller_id: ControllerId::new("holder"),
            nodes: vec![],
            session: SessionConfig {
                io_timeout_ms: 1000,
                query_interval_ms: 100,
            },
            reconnect_ms: 100,
            timezone: "Asia/Shanghai".into(),
        })
        .unwrap();
        store.set_lease(Some(17));
        Self {
            store,
            server,
            task,
            node: NodeRuntimeIdentity {
                node_id: NodeId::new("node"),
                incarnation_id: NodeIncarnationId::new("incarnation"),
            },
        }
    }
}

#[tonic::async_trait]
impl ExecutionService for Server {
    async fn get_dispatch(
        &self,
        request: Request<proto::GetDispatchRequest>,
    ) -> Result<Response<proto::GetDispatchResponse>, Status> {
        if request.into_inner().execution_id != "execution" {
            return Err(Status::not_found("execution"));
        }
        Ok(Response::new(proto::GetDispatchResponse {
            record: Some(proto::ExecutionRecord {
                operation_id: "cloud-run".into(),
                execution_id: "execution".into(),
                node_operation_id: "node-operation".into(),
                node_id: "node".into(),
                input: Some(proto::ExecutionInput {
                    spec: Some(proto::execution_input::Spec::AgentSession(
                        proto::AgentSessionSpec {
                            agent_plugin_id: "official/ora-space.echo".into(),
                            agent_plugin_version: "1.2.3".into(),
                            checkout_execution_id: "checkout".into(),
                            model_binding_id: String::new(),
                            agent_configuration: None,
                            git_identity: Some(proto::GitIdentity {
                                name: "User".into(),
                                email: "user@example.com".into(),
                            }),
                            initial_turn: Some(proto::UserTurn {
                                turn_id: "turn".into(),
                                content: vec![proto::ContentBlock {
                                    block: Some(proto::content_block::Block::Text(
                                        proto::TextContent {
                                            text: "hello".into(),
                                        },
                                    )),
                                }],
                            }),
                            prior_revision: None,
                        },
                    )),
                }),
                result: self
                    .0
                    .lock()
                    .unwrap()
                    .ended
                    .then(proto::ExecutionResult::default),
            }),
        }))
    }
    async fn claim_work(
        &self,
        _: Request<proto::ClaimWorkRequest>,
    ) -> Result<Response<proto::ClaimWorkResponse>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn record_dispatch(
        &self,
        _: Request<proto::RecordDispatchRequest>,
    ) -> Result<Response<proto::RecordDispatchResponse>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn take_over_node_event(
        &self,
        _: Request<proto::TakeOverNodeEventRequest>,
    ) -> Result<Response<proto::TakeOverNodeEventResponse>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn record_queried_result(
        &self,
        _: Request<proto::RecordQueriedResultRequest>,
    ) -> Result<Response<proto::RecordQueriedResultResponse>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn list_pending_dispatches(
        &self,
        _: Request<proto::ListPendingDispatchesRequest>,
    ) -> Result<Response<proto::ListPendingDispatchesResponse>, Status> {
        Err(Status::unimplemented("unused"))
    }
}
#[tonic::async_trait]
impl AgentRunService for Server {
    async fn publish_thread_preview(
        &self,
        request: Request<proto::PublishThreadPreviewRequest>,
    ) -> Result<Response<proto::PublishThreadPreviewResponse>, Status> {
        assert_eq!(
            request.metadata().get("x-ora-controller-id").unwrap(),
            "holder"
        );
        let mut state = self.0.lock().unwrap();
        state.previews.push(request.into_inner());
        if let Some(code) = state.failure.take() {
            return Err(Status::new(code, "injected"));
        }
        Ok(Response::new(proto::PublishThreadPreviewResponse {}))
    }
    async fn take_over_thread_events(
        &self,
        request: Request<proto::TakeOverThreadEventsRequest>,
    ) -> Result<Response<proto::TakeOverThreadEventsResponse>, Status> {
        let request = request.into_inner();
        let through = request.events.last().unwrap().sequence;
        self.0.lock().unwrap().events.push(request);
        Ok(Response::new(proto::TakeOverThreadEventsResponse {
            taken_over_through: through,
        }))
    }
    async fn claim_thread_commands(
        &self,
        _: Request<proto::ClaimThreadCommandsRequest>,
    ) -> Result<Response<proto::ClaimThreadCommandsResponse>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn record_thread_command_delivered(
        &self,
        _: Request<proto::RecordThreadCommandDeliveredRequest>,
    ) -> Result<Response<proto::RecordThreadCommandDeliveredResponse>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn grant_revision_upload(
        &self,
        _: Request<proto::GrantRevisionUploadRequest>,
    ) -> Result<Response<proto::GrantRevisionUploadResponse>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn grant_revision_download(
        &self,
        _: Request<proto::GrantRevisionDownloadRequest>,
    ) -> Result<Response<proto::GrantRevisionDownloadResponse>, Status> {
        Err(Status::unimplemented("unused"))
    }
}
