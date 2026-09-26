//! An in-memory Cloud that serves the real Workspace operation, Node report and execution
//! contracts through the test-only server stubs, with just enough authority to exercise the
//! Controller's operation driver: operations advance only on the evidence the real Cloud checks
//! (a succeeded effect, a registered and connected Node, a ready clone, accepted idle evidence),
//! writes are fenced by epoch and version, and every decision the Controller made is appended to a
//! timeline the fake Substrate and fake Node share, so tests can assert ordering across all three.
use futures::{Stream, stream};
use ora_controller_proto::v1::{
    self as proto,
    control_signal_service_server::{ControlSignalService, ControlSignalServiceServer},
    controller_lease_service_server::{ControllerLeaseService, ControllerLeaseServiceServer},
    effect_evidence::Evidence,
    effect_request::Request as EffectRequest,
    execution_service_server::{ExecutionService, ExecutionServiceServer},
    node_report_service_server::{NodeReportService, NodeReportServiceServer},
    watch_response::Signal,
    workspace_operation_service_server::{
        WorkspaceOperationService, WorkspaceOperationServiceServer,
    },
};
use std::{
    net::SocketAddr,
    pin::Pin,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};
use tokio::sync::{Notify, mpsc, oneshot};
use tonic::{Request, Response, Status, transport::server::TcpIncoming};

pub const PROJECT: &str = "00000000-0000-4000-8000-000000000001";
pub const WORKSPACE: &str = "00000000-0000-4000-8000-000000000002";
pub const REPOSITORY: &str = "https://example.invalid/repo.git";
/// The only lease the fake grants.
pub const EPOCH: i64 = 1;

/// One observable decision, from whichever party saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Claimed {
        step: &'static str,
    },
    Planned {
        kind: &'static str,
    },
    Recorded {
        kind: &'static str,
        state: &'static str,
    },
    Advanced {
        to: &'static str,
    },
    Deferred {
        reason: &'static str,
    },
    Registered {
        incarnation: String,
    },
    Status {
        connection: &'static str,
    },
    Ended,
    Idle {
        idle: bool,
    },
    Dispatched,
    /// The fake Substrate received a request (`GET` or `PUT`) for an effect of `kind`.
    Substrate {
        method: &'static str,
        kind: &'static str,
    },
    /// The fake Node's session with the Controller ended.
    SessionEnded,
    /// The Controller listed the live sandboxes; `sandboxes` is how many Cloud returned.
    Listed {
        sandboxes: usize,
    },
}

/// A shared, append-only timeline.
#[derive(Clone, Default)]
pub struct Timeline {
    events: Arc<Mutex<Vec<Event>>>,
    changed: Arc<Notify>,
}

impl Timeline {
    pub fn push(&self, event: Event) {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event);
        self.changed.notify_waiters();
    }

    pub fn events(&self) -> Vec<Event> {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Waits until `condition` holds over the events, failing the test after ten seconds.
    pub async fn until(&self, condition: impl Fn(&[Event]) -> bool) {
        let wait = async {
            loop {
                let notified = self.changed.notified();
                if condition(&self.events()) {
                    return;
                }
                notified.await;
            }
        };
        if tokio::time::timeout(Duration::from_secs(/*secs*/ 10), wait)
            .await
            .is_err()
        {
            panic!("condition not reached; events: {:#?}", self.events());
        }
    }
}

struct Op {
    operation: proto::Operation,
    steps: &'static [proto::OperationStep],
}

struct State {
    ops: Vec<Op>,
    workspace: proto::OperationWorkspace,
    sandboxes: Vec<proto::SandboxRecord>,
    nodes: Vec<proto::NodeRecord>,
    effects: Vec<(String, proto::Effect)>,
    clones: Vec<proto::ExecutionRecord>,
    subscriber: Option<mpsc::Sender<Result<proto::WatchResponse, Status>>>,
    next_id: u64,
}

/// Shared handle to the fake; clones serve the same authority.
#[derive(Clone)]
pub struct WorkspaceCloud {
    state: Arc<Mutex<State>>,
    pub timeline: Timeline,
}

/// The running server; dropping it stops serving.
pub struct Served {
    pub endpoint: String,
    stop: Option<oneshot::Sender<()>>,
}

impl Drop for Served {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

fn step_name(step: proto::OperationStep) -> &'static str {
    match step {
        proto::OperationStep::Sandbox => "sandbox",
        proto::OperationStep::Node => "node",
        proto::OperationStep::Clone => "clone",
        proto::OperationStep::Quiesce => "quiesce",
        proto::OperationStep::Terminate => "terminate",
        proto::OperationStep::Cleanup => "cleanup",
        proto::OperationStep::Plugin => "plugin",
        proto::OperationStep::Done => "done",
        proto::OperationStep::Unspecified => "unspecified",
    }
}

pub fn kind_name(kind: proto::EffectKind) -> &'static str {
    match kind {
        proto::EffectKind::SandboxEnsure => "sandbox_ensure",
        proto::EffectKind::SandboxTerminate => "sandbox_terminate",
        proto::EffectKind::WorkspaceDataDelete => "workspace_data_delete",
        proto::EffectKind::PluginEnsure => "plugin_ensure",
        proto::EffectKind::PluginDelete => "plugin_delete",
        proto::EffectKind::Unspecified => "unspecified",
    }
}

fn conflict(code: &str) -> Status {
    Status::aborted(code.to_owned())
}

impl WorkspaceCloud {
    pub fn new(timeline: Timeline, requested_ref: &str) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                ops: Vec::new(),
                workspace: proto::OperationWorkspace {
                    id: WORKSPACE.into(),
                    kind: proto::WorkspaceKind::Main as i32,
                    desired_state: "running".into(),
                    observed_state: "provisioning".into(),
                    runtime_generation: 0,
                    admission_open: false,
                    admission_epoch: 1,
                    requested_ref: requested_ref.into(),
                    base_commit_id: None,
                    version: 1,
                },
                sandboxes: Vec::new(),
                nodes: Vec::new(),
                effects: Vec::new(),
                clones: Vec::new(),
                subscriber: None,
                next_id: 100,
            })),
            timeline,
        }
    }

    pub async fn serve(&self) -> Served {
        let incoming = TcpIncoming::bind("127.0.0.1:0".parse::<SocketAddr>().unwrap()).unwrap();
        let endpoint = format!("http://{}", incoming.local_addr().unwrap());
        let (stop, stopped) = oneshot::channel();
        let router = tonic::transport::Server::builder()
            .add_service(ControllerLeaseServiceServer::new(self.clone()))
            .add_service(ExecutionServiceServer::new(self.clone()))
            .add_service(ControlSignalServiceServer::new(self.clone()))
            .add_service(WorkspaceOperationServiceServer::new(self.clone()))
            .add_service(NodeReportServiceServer::new(self.clone()));
        tokio::spawn(async move {
            router
                .serve_with_incoming_shutdown(incoming, async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        Served {
            endpoint,
            stop: Some(stop),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Queues an operation of `kind` on the Workspace and signals it, as the public API does.
    pub fn queue(&self, kind: proto::OperationKind) -> String {
        use proto::OperationStep as S;
        let steps: &'static [S] = match kind {
            proto::OperationKind::CreateWorkspace => &[S::Sandbox, S::Node, S::Clone, S::Done],
            proto::OperationKind::Start => &[S::Sandbox, S::Node, S::Done],
            proto::OperationKind::Stop => &[S::Quiesce, S::Terminate, S::Done],
            proto::OperationKind::DeleteWorkspace => {
                &[S::Quiesce, S::Terminate, S::Cleanup, S::Done]
            }
            _ => panic!("unsupported operation kind in the fake"),
        };
        let id = {
            let mut state = self.lock();
            state.next_id += 1;
            let id = format!("00000000-0000-4000-9000-{:012}", state.next_id);
            state.ops.push(Op {
                operation: proto::Operation {
                    id: id.clone(),
                    kind: kind as i32,
                    state: proto::OperationState::Queued as i32,
                    step: steps[0] as i32,
                    project_id: PROJECT.into(),
                    workspace_id: Some(WORKSPACE.into()),
                    version: 1,
                    controller_epoch: None,
                    error_code: None,
                },
                steps,
            });
            if let Some(subscriber) = &state.subscriber {
                let _ = subscriber.try_send(Ok(proto::WatchResponse {
                    signal: Some(Signal::OperationAvailable(proto::OperationAvailable {
                        operation_id: id.clone(),
                    })),
                }));
            }
            id
        };
        id
    }

    /// The operation's current state and step.
    pub fn operation(&self, id: &str) -> proto::Operation {
        self.lock()
            .ops
            .iter()
            .find(|op| op.operation.id == id)
            .map(|op| op.operation.clone())
            .unwrap()
    }

    /// Registers an execution to `node` that has no result, as a dispatch still in flight.
    pub fn inject_pending(&self, operation: &str, node: &str) {
        let mut state = self.lock();
        let execution = Self::next_id(&mut state);
        let branch = state.workspace.requested_ref.clone();
        state.clones.push(proto::ExecutionRecord {
            operation_id: operation.into(),
            execution_id: execution,
            node_id: node.into(),
            input: Some(proto::ExecutionInput {
                spec: Some(proto::execution_input::Spec::Clone(proto::CloneSpec {
                    repository: REPOSITORY.into(),
                    branch,
                })),
            }),
            result: None,
        });
    }

    pub fn workspace(&self) -> proto::OperationWorkspace {
        self.lock().workspace.clone()
    }

    fn next_id(state: &mut State) -> String {
        state.next_id += 1;
        format!("00000000-0000-4000-a000-{:012}", state.next_id)
    }

    /// Checks the fencing a write carries and returns the operation's index.
    fn fenced(state: &State, id: &str, epoch: i64, version: i64) -> Result<usize, Status> {
        let index = state
            .ops
            .iter()
            .position(|op| op.operation.id == id)
            .ok_or_else(|| Status::not_found("not_found"))?;
        let operation = &state.ops[index].operation;
        if operation.state != proto::OperationState::Running as i32
            || operation.controller_epoch != Some(epoch)
            || operation.version != version
        {
            return Err(Status::failed_precondition("stale_operation"));
        }
        Ok(index)
    }

    fn snapshot(state: &State, index: usize) -> proto::OperationSnapshot {
        let id = &state.ops[index].operation.id;
        proto::OperationSnapshot {
            operation: Some(state.ops[index].operation.clone()),
            project: Some(proto::OperationProject {
                id: PROJECT.into(),
                repository_url: REPOSITORY.into(),
                default_branch: state.workspace.requested_ref.clone(),
                credential_ref: None,
            }),
            workspaces: vec![state.workspace.clone()],
            sandboxes: state.sandboxes.clone(),
            nodes: state.nodes.clone(),
            effects: state
                .effects
                .iter()
                .filter(|(operation, _)| operation == id)
                .map(|(_, effect)| effect.clone())
                .collect(),
            clones: state
                .clones
                .iter()
                .filter(|record| &record.operation_id == id)
                .cloned()
                .collect(),
        }
    }

    fn live_sandbox(state: &mut State) -> Option<&mut proto::SandboxRecord> {
        state
            .sandboxes
            .iter_mut()
            .find(|sandbox| sandbox.observed_state != "terminated")
    }

    fn live_node(state: &State) -> Option<&proto::NodeRecord> {
        let sandbox = state
            .sandboxes
            .iter()
            .find(|sandbox| sandbox.observed_state != "terminated")?;
        state.nodes.iter().find(|node| {
            node.sandbox_instance_id == sandbox.id
                && node.connection == proto::NodeConnection::Connected as i32
        })
    }

    fn effect_of(state: &State, operation: &str, kind: proto::EffectKind) -> Option<proto::Effect> {
        state
            .effects
            .iter()
            .find(|(owner, effect)| owner == operation && effect.kind == kind as i32)
            .map(|(_, effect)| effect.clone())
    }

    /// Moves the operation past its current step once the step's evidence is there.
    fn advance(state: &mut State, index: usize) -> Result<(), Status> {
        let id = state.ops[index].operation.id.clone();
        let step = state.ops[index].operation.step();
        let succeeded = |state: &State, kind| {
            Self::effect_of(state, &id, kind)
                .is_some_and(|effect| effect.state == proto::EffectState::Succeeded as i32)
        };
        match step {
            proto::OperationStep::Sandbox => {
                let effect = Self::effect_of(state, &id, proto::EffectKind::SandboxEnsure)
                    .filter(|effect| effect.state == proto::EffectState::Succeeded as i32)
                    .ok_or_else(|| conflict("effect_incomplete"))?;
                let sandbox = Self::live_sandbox(state).ok_or_else(|| conflict("no_sandbox"))?;
                sandbox.substrate_sandbox_id = effect.external_id;
                sandbox.observed_state = "starting".into();
            }
            proto::OperationStep::Node => {
                Self::live_node(state).ok_or_else(|| conflict("node_not_ready"))?;
                if state.ops[index].operation.kind() == proto::OperationKind::Start {
                    state.workspace.observed_state = "ready".into();
                    state.workspace.admission_open = true;
                }
            }
            proto::OperationStep::Clone => {
                let commit = state
                    .clones
                    .iter()
                    .rev()
                    .find(|record| record.operation_id == id)
                    .and_then(|record| record.result.clone())
                    .and_then(|result| match result.outcome {
                        Some(proto::execution_result::Outcome::CloneReady(ready)) => {
                            Some(ready.commit)
                        }
                        _ => None,
                    })
                    .ok_or_else(|| conflict("clone_incomplete"))?;
                state.workspace.base_commit_id = Some(commit);
                state.workspace.observed_state = "ready".into();
                state.workspace.admission_open = true;
            }
            proto::OperationStep::Quiesce => {
                let epoch = state.workspace.admission_epoch;
                if Self::live_sandbox(state).is_some()
                    && !Self::live_node(state)
                        .is_some_and(|node| node.idle_admission_epoch == Some(epoch))
                {
                    return Err(conflict("idle_unconfirmed"));
                }
            }
            proto::OperationStep::Terminate => {
                if Self::live_sandbox(state).is_some() {
                    if !succeeded(state, proto::EffectKind::SandboxTerminate) {
                        return Err(conflict("effect_incomplete"));
                    }
                    let sandbox = Self::live_sandbox(state).unwrap();
                    sandbox.observed_state = "terminated".into();
                    let sandbox = sandbox.id.clone();
                    for node in &mut state.nodes {
                        if node.sandbox_instance_id == sandbox {
                            node.connection = proto::NodeConnection::Ended as i32;
                            node.version += 1;
                        }
                    }
                }
                state.workspace.observed_state = "stopped".into();
            }
            proto::OperationStep::Cleanup => {
                if !succeeded(state, proto::EffectKind::WorkspaceDataDelete) {
                    return Err(conflict("effect_incomplete"));
                }
                state.workspace.observed_state = "deleted".into();
            }
            _ => return Err(conflict("invalid_step")),
        }
        let op = &mut state.ops[index];
        let position = op
            .steps
            .iter()
            .position(|candidate| *candidate == step)
            .unwrap();
        let next = op.steps[position + 1];
        op.operation.step = next as i32;
        op.operation.version += 1;
        if next == proto::OperationStep::Done {
            op.operation.state = proto::OperationState::Succeeded as i32;
        }
        Ok(())
    }
}

#[tonic::async_trait]
impl ControllerLeaseService for WorkspaceCloud {
    async fn acquire_lease(
        &self,
        _request: Request<proto::AcquireLeaseRequest>,
    ) -> Result<Response<proto::AcquireLeaseResponse>, Status> {
        Ok(Response::new(proto::AcquireLeaseResponse {
            lease: Some(proto::Lease {
                holder_id: "owner".into(),
                epoch: EPOCH,
                expires_at: None,
            }),
        }))
    }

    async fn renew_lease(
        &self,
        request: Request<proto::RenewLeaseRequest>,
    ) -> Result<Response<proto::RenewLeaseResponse>, Status> {
        Ok(Response::new(proto::RenewLeaseResponse {
            lease: Some(proto::Lease {
                holder_id: "owner".into(),
                epoch: request.get_ref().epoch,
                expires_at: None,
            }),
        }))
    }

    async fn release_lease(
        &self,
        request: Request<proto::ReleaseLeaseRequest>,
    ) -> Result<Response<proto::ReleaseLeaseResponse>, Status> {
        Ok(Response::new(proto::ReleaseLeaseResponse {
            lease: Some(proto::Lease {
                holder_id: "owner".into(),
                epoch: request.get_ref().epoch,
                expires_at: None,
            }),
        }))
    }
}

#[tonic::async_trait]
impl ControlSignalService for WorkspaceCloud {
    type WatchStream =
        Pin<Box<dyn Stream<Item = Result<proto::WatchResponse, Status>> + Send + 'static>>;

    async fn watch(
        &self,
        _request: Request<proto::WatchRequest>,
    ) -> Result<Response<Self::WatchStream>, Status> {
        let (sender, receiver) = mpsc::channel(/*buffer*/ 16);
        self.lock().subscriber = Some(sender);
        let signals = stream::unfold(receiver, |mut receiver| async move {
            receiver.recv().await.map(|signal| (signal, receiver))
        });
        Ok(Response::new(Box::pin(signals)))
    }
}

#[tonic::async_trait]
impl WorkspaceOperationService for WorkspaceCloud {
    async fn claim_operation(
        &self,
        request: Request<proto::ClaimOperationRequest>,
    ) -> Result<Response<proto::ClaimOperationResponse>, Status> {
        let epoch = request.get_ref().epoch;
        let (snapshot, step) = {
            let mut state = self.lock();
            let Some(index) = state.ops.iter().position(|op| {
                matches!(
                    op.operation.state(),
                    proto::OperationState::Queued | proto::OperationState::Running
                )
            }) else {
                return Ok(Response::new(proto::ClaimOperationResponse {
                    snapshot: None,
                }));
            };
            let operation = &mut state.ops[index].operation;
            operation.state = proto::OperationState::Running as i32;
            operation.controller_epoch = Some(epoch);
            operation.version += 1;
            let step = step_name(operation.step());
            (Self::snapshot(&state, index), step)
        };
        self.timeline.push(Event::Claimed { step });
        Ok(Response::new(proto::ClaimOperationResponse {
            snapshot: Some(snapshot),
        }))
    }

    async fn plan_effect(
        &self,
        request: Request<proto::PlanEffectRequest>,
    ) -> Result<Response<proto::PlanEffectResponse>, Status> {
        let message = request.into_inner();
        let kind = message.kind();
        let (effect, operation) = {
            let mut state = self.lock();
            let index = Self::fenced(
                &state,
                &message.operation_id,
                message.epoch,
                message.version,
            )?;
            let effect = match Self::effect_of(&state, &message.operation_id, kind) {
                Some(effect) => effect,
                None => {
                    let id = Self::next_id(&mut state);
                    let request = match kind {
                        proto::EffectKind::SandboxEnsure => {
                            state.workspace.runtime_generation += 1;
                            let generation = state.workspace.runtime_generation;
                            state.sandboxes.push(proto::SandboxRecord {
                                id: id.clone(),
                                workspace_id: WORKSPACE.into(),
                                generation,
                                substrate_sandbox_id: None,
                                observed_state: "allocating".into(),
                            });
                            EffectRequest::SandboxEnsure(proto::SandboxEnsureRequest {
                                project_id: PROJECT.into(),
                                workspace_id: WORKSPACE.into(),
                            })
                        }
                        proto::EffectKind::SandboxTerminate => {
                            let sandbox = Self::live_sandbox(&mut state)
                                .ok_or_else(|| conflict("no_current_sandbox"))?;
                            sandbox.observed_state = "terminating".into();
                            EffectRequest::SandboxTerminate(proto::SandboxTerminateRequest {
                                project_id: PROJECT.into(),
                                workspace_id: WORKSPACE.into(),
                                sandbox_instance_id: sandbox.id.clone(),
                            })
                        }
                        proto::EffectKind::WorkspaceDataDelete => {
                            EffectRequest::WorkspaceDataDelete(proto::WorkspaceDataDeleteRequest {
                                project_id: PROJECT.into(),
                                workspace_id: WORKSPACE.into(),
                            })
                        }
                        _ => return Err(conflict("invalid_step")),
                    };
                    let effect = proto::Effect {
                        id,
                        kind: kind as i32,
                        state: proto::EffectState::Planned as i32,
                        workspace_id: WORKSPACE.into(),
                        request: Some(proto::EffectRequest {
                            request: Some(request),
                        }),
                        external_id: None,
                        evidence: None,
                        failure: None,
                        reconciled_epoch: message.epoch,
                    };
                    state
                        .effects
                        .push((message.operation_id.clone(), effect.clone()));
                    state.ops[index].operation.version += 1;
                    effect
                }
            };
            (effect, state.ops[index].operation.clone())
        };
        self.timeline.push(Event::Planned {
            kind: kind_name(kind),
        });
        Ok(Response::new(proto::PlanEffectResponse {
            effect: Some(effect),
            operation: Some(operation),
        }))
    }

    async fn record_effect_result(
        &self,
        request: Request<proto::RecordEffectResultRequest>,
    ) -> Result<Response<proto::RecordEffectResultResponse>, Status> {
        let message = request.into_inner();
        let (effect, operation) = {
            let mut state = self.lock();
            let index = Self::fenced(
                &state,
                &message.operation_id,
                message.epoch,
                message.version,
            )?;
            let (_, effect) = state
                .effects
                .iter_mut()
                .find(|(_, effect)| effect.id == message.effect_id)
                .ok_or_else(|| Status::not_found("not_found"))?;
            if message.state() != proto::EffectState::Absent {
                effect.state = message.state;
                effect.external_id = Some(message.external_id.clone());
                effect.evidence = message.evidence.clone();
                effect.failure = message.failure.clone();
            }
            effect.reconciled_epoch = message.epoch;
            let effect = effect.clone();
            state.ops[index].operation.version += 1;
            (effect, state.ops[index].operation.clone())
        };
        self.timeline.push(Event::Recorded {
            kind: kind_name(effect.kind()),
            state: match message.state() {
                proto::EffectState::Succeeded => "succeeded",
                proto::EffectState::Failed => "failed",
                proto::EffectState::Running => "running",
                _ => "absent",
            },
        });
        Ok(Response::new(proto::RecordEffectResultResponse {
            effect: Some(effect),
            operation: Some(operation),
        }))
    }

    async fn advance_operation(
        &self,
        request: Request<proto::AdvanceOperationRequest>,
    ) -> Result<Response<proto::AdvanceOperationResponse>, Status> {
        let message = request.into_inner();
        let operation = {
            let mut state = self.lock();
            let index = Self::fenced(
                &state,
                &message.operation_id,
                message.epoch,
                message.version,
            )?;
            Self::advance(&mut state, index)?;
            state.ops[index].operation.clone()
        };
        self.timeline.push(Event::Advanced {
            to: step_name(operation.step()),
        });
        Ok(Response::new(proto::AdvanceOperationResponse {
            operation: Some(operation),
        }))
    }

    async fn defer_operation(
        &self,
        request: Request<proto::DeferOperationRequest>,
    ) -> Result<Response<proto::DeferOperationResponse>, Status> {
        let message = request.into_inner();
        let operation = {
            let mut state = self.lock();
            let index = Self::fenced(
                &state,
                &message.operation_id,
                message.epoch,
                message.version,
            )?;
            let operation = &mut state.ops[index].operation;
            // The fake never offers a parked operation again, so a test sees exactly one round.
            operation.state = match message.state() {
                proto::DeferState::Blocked => proto::OperationState::Blocked,
                _ => proto::OperationState::RetryWait,
            } as i32;
            operation.version += 1;
            operation.clone()
        };
        self.timeline.push(Event::Deferred {
            reason: match message.reason() {
                proto::DeferReason::SubstrateTimeout => "substrate_timeout",
                proto::DeferReason::TerminationUnconfirmed => "termination_unconfirmed",
                proto::DeferReason::NodeUnavailable => "node_unavailable",
                proto::DeferReason::ExternalFailure => "external_failure",
                proto::DeferReason::CloneFailed => "clone_failed",
                proto::DeferReason::CloneResultUnknown => "clone_result_unknown",
                proto::DeferReason::Unspecified => "unspecified",
            },
        });
        Ok(Response::new(proto::DeferOperationResponse {
            operation: Some(operation),
        }))
    }

    async fn list_live_sandboxes(
        &self,
        request: Request<proto::ListLiveSandboxesRequest>,
    ) -> Result<Response<proto::ListLiveSandboxesResponse>, Status> {
        if request.get_ref().epoch != EPOCH {
            return Err(Status::failed_precondition("stale_controller"));
        }
        let sandboxes = Self::live_sandboxes(&self.lock());
        self.timeline.push(Event::Listed {
            sandboxes: sandboxes.len(),
        });
        Ok(Response::new(proto::ListLiveSandboxesResponse {
            sandboxes,
        }))
    }
}

impl WorkspaceCloud {
    /// The sandboxes the real Cloud lists as live: not terminating or terminated, of the
    /// Workspace's current generation, with a succeeded ensure effect giving the NodeId.
    fn live_sandboxes(state: &State) -> Vec<proto::LiveSandbox> {
        state
            .sandboxes
            .iter()
            .filter(|sandbox| {
                !matches!(
                    sandbox.observed_state.as_str(),
                    "terminating" | "terminated"
                ) && sandbox.generation == state.workspace.runtime_generation
            })
            .filter_map(|sandbox| {
                let ensure = state
                    .effects
                    .iter()
                    .map(|(_, effect)| effect)
                    .find(|effect| effect.id == sandbox.id)?;
                let Some(proto::EffectEvidence {
                    evidence: Some(Evidence::SandboxEnsured(ensured)),
                }) = &ensure.evidence
                else {
                    return None;
                };
                let mut record = sandbox.clone();
                record.substrate_sandbox_id = record
                    .substrate_sandbox_id
                    .or_else(|| ensure.external_id.clone());
                Some(proto::LiveSandbox {
                    sandbox: Some(record),
                    node_id: ensured.node_id.clone(),
                    nodes: state
                        .nodes
                        .iter()
                        .filter(|node| {
                            node.sandbox_instance_id == sandbox.id
                                && node.connection != proto::NodeConnection::Ended as i32
                        })
                        .cloned()
                        .collect(),
                })
            })
            .collect()
    }
}

#[tonic::async_trait]
impl NodeReportService for WorkspaceCloud {
    async fn register_node(
        &self,
        request: Request<proto::RegisterNodeRequest>,
    ) -> Result<Response<proto::RegisterNodeResponse>, Status> {
        let message = request.into_inner();
        let identity = message.node.clone().unwrap_or_default();
        let record = {
            let mut state = self.lock();
            let expected = state
                .effects
                .iter()
                .find(|(_, effect)| effect.id == message.sandbox_instance_id)
                .and_then(|(_, effect)| match &effect.evidence {
                    Some(proto::EffectEvidence {
                        evidence: Some(Evidence::SandboxEnsured(ensured)),
                    }) => Some(ensured.node_id.clone()),
                    _ => None,
                });
            if expected.as_deref() != Some(identity.node_id.as_str()) {
                return Err(conflict("node_mismatch"));
            }
            if let Some(existing) = state.nodes.iter().find(|node| {
                node.sandbox_instance_id == message.sandbox_instance_id
                    && node.identity.as_ref() == Some(&identity)
            }) {
                if existing.connection == proto::NodeConnection::Ended as i32 {
                    return Err(conflict("stale_node"));
                }
                existing.clone()
            } else {
                if state.nodes.iter().any(|node| {
                    node.sandbox_instance_id == message.sandbox_instance_id
                        && node.connection != proto::NodeConnection::Ended as i32
                }) {
                    return Err(conflict("node_already_registered"));
                }
                let id = Self::next_id(&mut state);
                let record = proto::NodeRecord {
                    id,
                    sandbox_instance_id: message.sandbox_instance_id.clone(),
                    workspace_id: WORKSPACE.into(),
                    identity: Some(identity.clone()),
                    connection: proto::NodeConnection::Connected as i32,
                    initialized: true,
                    version: 1,
                    idle_admission_epoch: None,
                };
                state.nodes.push(record.clone());
                record
            }
        };
        self.timeline.push(Event::Registered {
            incarnation: identity.node_incarnation_id,
        });
        Ok(Response::new(proto::RegisterNodeResponse {
            node: Some(record),
        }))
    }

    async fn report_node_status(
        &self,
        request: Request<proto::ReportNodeStatusRequest>,
    ) -> Result<Response<proto::ReportNodeStatusResponse>, Status> {
        let message = request.into_inner();
        let record = {
            let mut state = self.lock();
            let node = state
                .nodes
                .iter_mut()
                .find(|node| node.id == message.node_instance_id)
                .ok_or_else(|| Status::not_found("not_found"))?;
            if node.version != message.version
                || node.connection == proto::NodeConnection::Ended as i32
            {
                return Err(conflict("stale_node"));
            }
            node.connection = message.connection;
            node.version += 1;
            node.clone()
        };
        self.timeline.push(Event::Status {
            connection: match message.connection() {
                proto::NodeConnection::Connected => "connected",
                _ => "disconnected",
            },
        });
        Ok(Response::new(proto::ReportNodeStatusResponse {
            node: Some(record),
        }))
    }

    async fn end_node(
        &self,
        request: Request<proto::EndNodeRequest>,
    ) -> Result<Response<proto::EndNodeResponse>, Status> {
        let message = request.into_inner();
        let record = {
            let mut state = self.lock();
            let node = state
                .nodes
                .iter_mut()
                .find(|node| node.id == message.node_instance_id)
                .ok_or_else(|| Status::not_found("not_found"))?;
            if node.connection != proto::NodeConnection::Ended as i32 {
                node.connection = proto::NodeConnection::Ended as i32;
                node.version += 1;
            }
            node.clone()
        };
        self.timeline.push(Event::Ended);
        Ok(Response::new(proto::EndNodeResponse { node: Some(record) }))
    }

    async fn report_node_idle(
        &self,
        request: Request<proto::ReportNodeIdleRequest>,
    ) -> Result<Response<proto::ReportNodeIdleResponse>, Status> {
        let message = request.into_inner();
        let response = {
            let mut state = self.lock();
            let admission = state.workspace.admission_epoch;
            let index = state
                .nodes
                .iter()
                .position(|node| node.id == message.node_instance_id)
                .ok_or_else(|| Status::not_found("not_found"))?;
            if state.nodes[index].version != message.version || message.admission_epoch != admission
            {
                return Err(conflict("stale_node"));
            }
            if message.idle {
                let node = &mut state.nodes[index];
                node.idle_admission_epoch = Some(admission);
                node.version += 1;
                proto::ReportNodeIdleResponse {
                    accepted: true,
                    node: Some(node.clone()),
                }
            } else {
                if let Some(op) = state
                    .ops
                    .iter_mut()
                    .find(|op| op.operation.id == message.operation_id)
                {
                    op.operation.state = proto::OperationState::Failed as i32;
                }
                proto::ReportNodeIdleResponse {
                    accepted: false,
                    node: None,
                }
            }
        };
        self.timeline.push(Event::Idle { idle: message.idle });
        Ok(Response::new(response))
    }
}

#[tonic::async_trait]
impl ExecutionService for WorkspaceCloud {
    async fn claim_work(
        &self,
        _request: Request<proto::ClaimWorkRequest>,
    ) -> Result<Response<proto::ClaimWorkResponse>, Status> {
        Ok(Response::new(proto::ClaimWorkResponse { item: None }))
    }

    async fn record_dispatch(
        &self,
        request: Request<proto::RecordDispatchRequest>,
    ) -> Result<Response<proto::RecordDispatchResponse>, Status> {
        let message = request.into_inner();
        let record = {
            let mut state = self.lock();
            let op = state
                .ops
                .iter()
                .find(|op| op.operation.id == message.operation_id)
                .ok_or_else(|| Status::not_found("not_found"))?;
            if op.operation.step() != proto::OperationStep::Clone {
                return Err(conflict("dispatch_conflict"));
            }
            let expected = proto::ExecutionInput {
                spec: Some(proto::execution_input::Spec::Clone(proto::CloneSpec {
                    repository: REPOSITORY.into(),
                    branch: state.workspace.requested_ref.clone(),
                })),
            };
            if message.input.as_ref() != Some(&expected) {
                return Err(conflict("dispatch_conflict"));
            }
            let record = proto::ExecutionRecord {
                operation_id: message.operation_id,
                execution_id: message.execution_id,
                node_id: message.node_id,
                input: message.input,
                result: None,
            };
            state.clones.push(record.clone());
            record
        };
        self.timeline.push(Event::Dispatched);
        Ok(Response::new(proto::RecordDispatchResponse {
            record: Some(record),
        }))
    }

    async fn take_over_node_event(
        &self,
        request: Request<proto::TakeOverNodeEventRequest>,
    ) -> Result<Response<proto::TakeOverNodeEventResponse>, Status> {
        let message = request.into_inner();
        self.store_result(&message.execution_id, message.result)
            .map(|record| {
                Response::new(proto::TakeOverNodeEventResponse {
                    record: Some(record),
                })
            })
    }

    async fn record_queried_result(
        &self,
        request: Request<proto::RecordQueriedResultRequest>,
    ) -> Result<Response<proto::RecordQueriedResultResponse>, Status> {
        let message = request.into_inner();
        self.store_result(&message.execution_id, message.result)
            .map(|record| {
                Response::new(proto::RecordQueriedResultResponse {
                    record: Some(record),
                })
            })
    }

    async fn get_dispatch(
        &self,
        request: Request<proto::GetDispatchRequest>,
    ) -> Result<Response<proto::GetDispatchResponse>, Status> {
        let execution_id = request.into_inner().execution_id;
        self.lock()
            .clones
            .iter()
            .find(|record| record.execution_id == execution_id)
            .cloned()
            .map(|record| {
                Response::new(proto::GetDispatchResponse {
                    record: Some(record),
                })
            })
            .ok_or_else(|| Status::not_found("not_found"))
    }

    async fn list_pending_dispatches(
        &self,
        request: Request<proto::ListPendingDispatchesRequest>,
    ) -> Result<Response<proto::ListPendingDispatchesResponse>, Status> {
        let node = request.into_inner().node_id;
        let records = self
            .lock()
            .clones
            .iter()
            .filter(|record| record.result.is_none() && (node.is_empty() || record.node_id == node))
            .cloned()
            .collect();
        Ok(Response::new(proto::ListPendingDispatchesResponse {
            records,
        }))
    }
}

impl WorkspaceCloud {
    fn store_result(
        &self,
        execution: &str,
        result: Option<proto::ExecutionResult>,
    ) -> Result<proto::ExecutionRecord, Status> {
        let mut state = self.lock();
        let record = state
            .clones
            .iter_mut()
            .find(|record| record.execution_id == execution)
            .ok_or_else(|| Status::not_found("not_found"))?;
        match &record.result {
            Some(existing) if Some(existing) != result.as_ref() => Err(conflict("result_conflict")),
            _ => {
                record.result = result;
                Ok(record.clone())
            }
        }
    }
}
