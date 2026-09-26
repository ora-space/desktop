//! An in-memory Cloud that serves the real internal control contract through the test-only
//! server stubs of `ora-controller-proto`. It keeps just enough authority to exercise the
//! Controller's Cloud adapter: one lease with an epoch, a queue of accepted work, registered
//! dispatches, and at most one `Watch` subscriber. Every call is recorded so tests can compare the
//! whole conversation of the coordination loop; the reads a Node session repeats on every query
//! tick are recorded apart so they do not make that order depend on session timing. Hooks let a
//! test publish signals, drain, break the stream, or refuse later subscriptions.
use futures::{Stream, stream};
use ora_controller_proto::v1::{
    self as proto,
    control_signal_service_server::{ControlSignalService, ControlSignalServiceServer},
    controller_lease_service_server::{ControllerLeaseService, ControllerLeaseServiceServer},
    execution_service_server::{ExecutionService, ExecutionServiceServer},
    watch_response::Signal,
};
use std::{
    collections::VecDeque,
    net::SocketAddr,
    pin::Pin,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};
use tokio::sync::{Notify, mpsc, oneshot};
use tonic::{Request, Response, Status, transport::server::TcpIncoming};

/// One recorded call, reduced to what the Controller decided: which RPC and under which epoch.
/// Submission identities are random per write and left out so sequences compare deterministically.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    AcquireLease,
    RenewLease { epoch: i64 },
    ReleaseLease { epoch: i64 },
    Watch { epoch: i64 },
    ClaimWork { epoch: i64 },
    RecordDispatch { epoch: i64, operation_id: String },
    TakeOverNodeEvent { epoch: i64 },
    RecordQueriedResult { epoch: i64 },
    GetDispatch,
    ListPendingDispatches,
}

/// How the next `Watch` calls are answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchPolicy {
    /// Register the subscription and send headers.
    Accept,
    /// `UNAVAILABLE`, as a draining or stopping Cloud answers.
    Unavailable,
    /// `UNIMPLEMENTED`, as a serving Cloud without the signal service answers.
    Unimplemented,
    /// `FAILED_PRECONDITION`: the presented epoch is treated as stale.
    Stale,
}

/// The authority's state; tests read it through [`FakeCloud::until`] and [`FakeCloud::calls`].
pub struct State {
    pub calls: Vec<Call>,
    /// `GetDispatch` and `ListPendingDispatches`, which Node sessions make on their own schedule.
    pub reads: Vec<Call>,
    /// The current lease epoch, or `None` while nobody holds it.
    pub epoch: Option<i64>,
    next_epoch: i64,
    queue: VecDeque<proto::WorkItem>,
    pub dispatches: Vec<proto::ExecutionRecord>,
    watch: WatchPolicy,
    subscriber: Option<mpsc::Sender<Result<proto::WatchResponse, Status>>>,
}

impl State {
    /// Whether a `Watch` stream is registered and its Controller end still open.
    pub fn watching(&self) -> bool {
        self.subscriber
            .as_ref()
            .is_some_and(|subscriber| !subscriber.is_closed())
    }

    /// Operation identities registered for dispatch, in registration order.
    pub fn dispatched(&self) -> Vec<String> {
        self.dispatches
            .iter()
            .map(|record| record.operation_id.clone())
            .collect()
    }
}

/// Shared handle to the fake; clones serve the same authority.
#[derive(Clone)]
pub struct FakeCloud {
    state: Arc<Mutex<State>>,
    changed: Arc<Notify>,
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

/// How long [`FakeCloud::until`] waits before failing the test.
const WAIT: Duration = Duration::from_secs(/*secs*/ 5);

impl FakeCloud {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                calls: Vec::new(),
                reads: Vec::new(),
                epoch: None,
                next_epoch: 1,
                queue: VecDeque::new(),
                dispatches: Vec::new(),
                watch: WatchPolicy::Accept,
                subscriber: None,
            })),
            changed: Arc::new(Notify::new()),
        }
    }

    /// Serves all three contract services on an ephemeral loopback port.
    pub async fn serve(&self) -> Served {
        let incoming = TcpIncoming::bind("127.0.0.1:0".parse::<SocketAddr>().unwrap()).unwrap();
        let endpoint = format!("http://{}", incoming.local_addr().unwrap());
        let (stop, stopped) = oneshot::channel();
        let router = tonic::transport::Server::builder()
            .add_service(ControllerLeaseServiceServer::new(self.clone()))
            .add_service(ExecutionServiceServer::new(self.clone()))
            .add_service(ControlSignalServiceServer::new(self.clone()));
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

    /// The state lock holds no invariant a panicking test could break.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Mutates the state and wakes every waiter.
    fn update<R>(&self, change: impl FnOnce(&mut State) -> R) -> R {
        let result = change(&mut self.lock());
        self.changed.notify_waiters();
        result
    }

    /// Records a call and, for fenced calls, checks the epoch the way Cloud does.
    fn fenced(&self, call: Call, epoch: i64) -> Result<(), Status> {
        self.update(|state| {
            state.calls.push(call);
            if state.epoch == Some(epoch) {
                Ok(())
            } else {
                Err(Status::failed_precondition("stale_controller"))
            }
        })
    }

    /// Waits until `condition` holds, failing the test after five seconds.
    pub async fn until(&self, condition: impl Fn(&State) -> bool) {
        let wait = async {
            loop {
                let notified = self.changed.notified();
                if condition(&self.lock()) {
                    return;
                }
                notified.await;
            }
        };
        if tokio::time::timeout(WAIT, wait).await.is_err() {
            panic!("condition not reached; calls: {:?}", self.calls());
        }
    }

    /// Waits until the Controller cancels the registered stream, failing the test after five
    /// seconds. Cancellation drops the server's stream and with it the receiving end.
    pub async fn watch_cancelled(&self) {
        let subscriber = self.lock().subscriber.clone();
        let Some(subscriber) = subscriber else {
            panic!("no stream is registered; calls: {:?}", self.calls());
        };
        if tokio::time::timeout(WAIT, subscriber.closed())
            .await
            .is_err()
        {
            panic!("the stream was not cancelled; calls: {:?}", self.calls());
        }
    }

    pub fn calls(&self) -> Vec<Call> {
        self.lock().calls.clone()
    }

    pub fn dispatched(&self) -> Vec<String> {
        self.lock().dispatched()
    }

    /// Accepts clone work, as the public clone intake does before any signal.
    pub fn enqueue(&self, operation_id: &str) {
        self.update(|state| {
            state.queue.push_back(proto::WorkItem {
                operation_id: operation_id.into(),
                input: Some(proto::ExecutionInput {
                    spec: Some(proto::execution_input::Spec::Clone(proto::CloneSpec {
                        repository: "https://example.invalid/repo.git".into(),
                        branch: "main".into(),
                    })),
                }),
            });
        });
    }

    /// Publishes a signal to the current subscriber, if any, without blocking.
    fn publish(&self, signal: Signal) {
        let state = self.lock();
        if let Some(subscriber) = &state.subscriber {
            let _ = subscriber.try_send(Ok(proto::WatchResponse {
                signal: Some(signal),
            }));
        }
    }

    /// Signals that `operation_id` became claimable.
    pub fn signal_work(&self, operation_id: &str) {
        self.publish(Signal::WorkAvailable(proto::WorkAvailable {
            operation_id: Some(operation_id.into()),
        }));
    }

    /// Drains like a stopping Cloud: sends `Drain`, ends the stream cleanly, and refuses later
    /// subscriptions with `UNAVAILABLE`.
    pub fn drain(&self) {
        self.publish(Signal::Drain(proto::Drain {}));
        self.update(|state| {
            state.subscriber = None;
            state.watch = WatchPolicy::Unavailable;
        });
    }

    /// Ends the stream with an error status, as a lost connection does.
    pub fn break_stream(&self) {
        self.update(|state| {
            if let Some(subscriber) = state.subscriber.take() {
                let _ = subscriber.try_send(Err(Status::internal("stream reset")));
            }
        });
    }

    /// Chooses how later `Watch` calls are answered.
    pub fn answer_watch(&self, policy: WatchPolicy) {
        self.update(|state| state.watch = policy);
    }
}

/// The lease reply every lease call shares.
fn lease(holder: &str, epoch: i64) -> Option<proto::Lease> {
    Some(proto::Lease {
        holder_id: holder.into(),
        epoch,
        expires_at: None,
    })
}

/// The Controller identity the adapter declares on every call.
fn holder<T>(request: &Request<T>) -> String {
    request
        .metadata()
        .get("x-ora-controller-id")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .into()
}

#[tonic::async_trait]
impl ControllerLeaseService for FakeCloud {
    async fn acquire_lease(
        &self,
        request: Request<proto::AcquireLeaseRequest>,
    ) -> Result<Response<proto::AcquireLeaseResponse>, Status> {
        let holder = holder(&request);
        let epoch = self.update(|state| {
            state.calls.push(Call::AcquireLease);
            let epoch = state.next_epoch;
            state.next_epoch += 1;
            state.epoch = Some(epoch);
            epoch
        });
        Ok(Response::new(proto::AcquireLeaseResponse {
            lease: lease(&holder, epoch),
        }))
    }

    async fn renew_lease(
        &self,
        request: Request<proto::RenewLeaseRequest>,
    ) -> Result<Response<proto::RenewLeaseResponse>, Status> {
        let epoch = request.get_ref().epoch;
        self.fenced(Call::RenewLease { epoch }, epoch)?;
        Ok(Response::new(proto::RenewLeaseResponse {
            lease: lease(&holder(&request), epoch),
        }))
    }

    async fn release_lease(
        &self,
        request: Request<proto::ReleaseLeaseRequest>,
    ) -> Result<Response<proto::ReleaseLeaseResponse>, Status> {
        let epoch = request.get_ref().epoch;
        self.fenced(Call::ReleaseLease { epoch }, epoch)?;
        self.update(|state| state.epoch = None);
        Ok(Response::new(proto::ReleaseLeaseResponse {
            lease: lease(&holder(&request), epoch),
        }))
    }
}

#[tonic::async_trait]
impl ExecutionService for FakeCloud {
    async fn claim_work(
        &self,
        request: Request<proto::ClaimWorkRequest>,
    ) -> Result<Response<proto::ClaimWorkResponse>, Status> {
        let epoch = request.get_ref().epoch;
        self.fenced(Call::ClaimWork { epoch }, epoch)?;
        // Claiming is a pure read: the head stays queued until a dispatch is registered.
        let item = self.lock().queue.front().cloned();
        Ok(Response::new(proto::ClaimWorkResponse { item }))
    }

    async fn record_dispatch(
        &self,
        request: Request<proto::RecordDispatchRequest>,
    ) -> Result<Response<proto::RecordDispatchResponse>, Status> {
        let message = request.into_inner();
        self.fenced(
            Call::RecordDispatch {
                epoch: message.epoch,
                operation_id: message.operation_id.clone(),
            },
            message.epoch,
        )?;
        let record = proto::ExecutionRecord {
            operation_id: message.operation_id,
            execution_id: message.execution_id,
            node_id: message.node_id,
            input: message.input,
            result: None,
        };
        self.update(|state| {
            state
                .queue
                .retain(|item| item.operation_id != record.operation_id);
            state.dispatches.push(record.clone());
        });
        Ok(Response::new(proto::RecordDispatchResponse {
            record: Some(record),
        }))
    }

    async fn take_over_node_event(
        &self,
        request: Request<proto::TakeOverNodeEventRequest>,
    ) -> Result<Response<proto::TakeOverNodeEventResponse>, Status> {
        let epoch = request.get_ref().epoch;
        self.fenced(Call::TakeOverNodeEvent { epoch }, epoch)?;
        Err(Status::unimplemented("the fake Cloud records no results"))
    }

    async fn record_queried_result(
        &self,
        request: Request<proto::RecordQueriedResultRequest>,
    ) -> Result<Response<proto::RecordQueriedResultResponse>, Status> {
        let epoch = request.get_ref().epoch;
        self.fenced(Call::RecordQueriedResult { epoch }, epoch)?;
        Err(Status::unimplemented("the fake Cloud records no results"))
    }

    async fn get_dispatch(
        &self,
        request: Request<proto::GetDispatchRequest>,
    ) -> Result<Response<proto::GetDispatchResponse>, Status> {
        let execution_id = request.into_inner().execution_id;
        let record = self.update(|state| {
            state.reads.push(Call::GetDispatch);
            state
                .dispatches
                .iter()
                .find(|record| record.execution_id == execution_id)
                .cloned()
        });
        record
            .map(|record| {
                Response::new(proto::GetDispatchResponse {
                    record: Some(record),
                })
            })
            .ok_or_else(|| Status::not_found("not_found"))
    }

    async fn list_pending_dispatches(
        &self,
        _request: Request<proto::ListPendingDispatchesRequest>,
    ) -> Result<Response<proto::ListPendingDispatchesResponse>, Status> {
        let records = self.update(|state| {
            state.reads.push(Call::ListPendingDispatches);
            state.dispatches.clone()
        });
        Ok(Response::new(proto::ListPendingDispatchesResponse {
            records,
        }))
    }
}

#[tonic::async_trait]
impl ControlSignalService for FakeCloud {
    type WatchStream =
        Pin<Box<dyn Stream<Item = Result<proto::WatchResponse, Status>> + Send + 'static>>;

    async fn watch(
        &self,
        request: Request<proto::WatchRequest>,
    ) -> Result<Response<Self::WatchStream>, Status> {
        let epoch = request.get_ref().epoch;
        let (sender, receiver) = mpsc::channel(/*buffer*/ 16);
        self.update(|state| {
            state.calls.push(Call::Watch { epoch });
            match state.watch {
                WatchPolicy::Unavailable => return Err(Status::unavailable("draining")),
                WatchPolicy::Unimplemented => {
                    return Err(Status::unimplemented("no signal service"));
                }
                WatchPolicy::Stale => return Err(Status::failed_precondition("stale_controller")),
                WatchPolicy::Accept => {}
            }
            if state.epoch != Some(epoch) {
                return Err(Status::failed_precondition("stale_controller"));
            }
            // Registered before the headers leave, as Cloud does, so no later signal is missed.
            state.subscriber = Some(sender);
            Ok(())
        })?;
        let signals = stream::unfold(receiver, |mut receiver| async move {
            receiver.recv().await.map(|signal| (signal, receiver))
        });
        Ok(Response::new(Box::pin(signals)))
    }
}
