//! Cloud RPC adapter of [`CoordinationStore`]: every durable operation is one call on the Cloud
//! internal control contract, committed by Cloud in PostgreSQL. The adapter holds no authoritative
//! state. Its channel and lease epoch are runtime state a replacement instance rebuilds from Cloud,
//! and it never opens a local database or takes a file lease.
mod fault;
mod lease;
mod mapping;

use crate::*;
use fault::Verdict;
use ora_controller_proto::v1::{
    self as proto, controller_lease_service_client::ControllerLeaseServiceClient,
    execution_service_client::ExecutionServiceClient,
};
use std::{
    io,
    sync::{Arc, Mutex},
    time::Duration,
};
use tonic::{
    metadata::{AsciiMetadataValue, MetadataValue},
    transport::Channel,
};

/// The metadata key carrying `controller_id`; the contract names no other identity for Controllers.
const HOLDER_METADATA: &str = "x-ora-controller-id";

/// Coordination through Cloud's contract. Cheap to clone: every clone shares the channel and the
/// lease epoch, so the runtime, the Node session and the claim loop all
/// write under the same fencing token.
pub struct CloudStore {
    inner: Arc<Inner>,
}

impl Clone for CloudStore {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

struct Inner {
    id: ControllerId,
    /// The one Node this deployment dispatches to; the contract keeps the target beside the input.
    node: NodeId,
    channel: Channel,
    /// `controller_id` as request metadata: Cloud records it as the lease and submission holder.
    holder: AsciiMetadataValue,
    /// The epoch of the lease currently held, or `None` while ineligible to write.
    lease: Mutex<Option<i64>>,
    claim_interval: Duration,
}

impl CloudStore {
    /// Binds the adapter to the deployment's Cloud endpoint without contacting it. The channel
    /// connects lazily, so a refusal here costs nothing and an unreachable Cloud is an unavailable
    /// authority at call time rather than a start-up failure that would hide accepted work.
    pub fn open(config: &RuntimeConfig) -> Result<Self, Error> {
        let Persistence::Cloud {
            endpoint,
            claim_interval_ms,
        } = &config.persistence
        else {
            return Err(Error::Configuration(
                "cloud adapter requires persistence.kind = cloud".into(),
            ));
        };
        let [node] = config.nodes.as_slice() else {
            return Err(Error::Configuration(
                "cloud persistence dispatches to exactly one configured Node".into(),
            ));
        };
        if *claim_interval_ms == 0 || config.controller_id.as_str().trim().is_empty() {
            return Err(Error::Configuration(
                "cloud persistence needs a nonzero claim_interval_ms and a controller_id".into(),
            ));
        }
        // gRPC ASCII metadata carries only printable ASCII; the header parser alone would admit
        // UTF-8 bytes that peers reject, so the identity is checked here once.
        let printable = |id: &str| {
            id.bytes()
                .all(|byte| byte == b' ' || byte.is_ascii_graphic())
        };
        let holder = Some(config.controller_id.as_str())
            .filter(|id| printable(id))
            .and_then(|id| MetadataValue::try_from(id).ok())
            .ok_or_else(|| {
                Error::Configuration("controller_id must be printable ASCII to reach Cloud".into())
            })?;
        let channel = tonic::transport::Endpoint::from_shared(endpoint.clone())
            .map_err(|error| Error::Configuration(format!("invalid cloud endpoint: {error}")))?
            .connect_timeout(fault::RPC_TIMEOUT)
            .connect_lazy();
        Ok(Self {
            inner: Arc::new(Inner {
                id: config.controller_id.clone(),
                node: node.node_id.clone(),
                channel,
                holder,
                lease: Mutex::new(None),
                claim_interval: Duration::from_millis(*claim_interval_ms),
            }),
        })
    }

    /// Names this Controller on every call. Cloud does not authenticate Controllers at this stage;
    /// the identity only tells it who holds the lease and who presented a submission.
    fn request<T>(&self, message: T) -> tonic::Request<T> {
        let mut request = tonic::Request::new(message);
        request
            .metadata_mut()
            .insert(HOLDER_METADATA, self.inner.holder.clone());
        request
    }

    fn executions(&self) -> ExecutionServiceClient<Channel> {
        ExecutionServiceClient::new(self.inner.channel.clone())
    }

    fn leases(&self) -> ControllerLeaseServiceClient<Channel> {
        ControllerLeaseServiceClient::new(self.inner.channel.clone())
    }

    /// The fencing token every write carries; without a held lease nothing may be written.
    fn epoch(&self) -> Result<i64, Error> {
        self.lease().ok_or(Error::StaleEligibility)
    }

    /// The lease cell holds no invariant a panic could break, so a poisoned lock is reused.
    fn lease(&self) -> Option<i64> {
        *self
            .inner
            .lease
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn set_lease(&self, epoch: Option<i64>) {
        *self
            .inner
            .lease
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = epoch;
    }

    /// Applies a verdict's side effects and turns it into the interface error. A stale verdict
    /// forgets the held lease so the claim loop re-acquires. `NotFound` reaches here only where a
    /// fact was required, which the interface defines as a conflict.
    fn settle(&self, verdict: Verdict) -> Error {
        match verdict {
            Verdict::NotFound | Verdict::Conflict => Error::Conflict,
            Verdict::Stale(detail) => {
                ora_logging::ora_warn!(detail = %detail, "Cloud lease is no longer current");
                self.set_lease(None);
                Error::StaleEligibility
            }
            Verdict::Unavailable(detail) => Error::Unavailable(detail.to_string()),
            Verdict::Unknown(detail) => Error::Unknown(detail.to_string()),
        }
    }

    /// Reads one recorded execution; `None` is an unknown identity.
    async fn record(
        &self,
        execution: &ExecutionId,
    ) -> Result<Option<proto::ExecutionRecord>, Error> {
        let call = async {
            let request = self.request(proto::GetDispatchRequest {
                execution_id: execution.as_str().into(),
            });
            self.executions().get_dispatch(request).await
        };
        match fault::read(call).await {
            Ok(response) => Ok(response.record),
            Err(Verdict::NotFound) => Ok(None),
            Err(verdict) => Err(self.settle(verdict)),
        }
    }

    /// The guard both fact writes share with the local adapter: the Node's fact must describe the
    /// dispatched spec and the command's request identity, and come from the dispatched Node.
    async fn dispatched(
        &self,
        session: &NodeRuntimeIdentity,
        operation: &OperationId,
        execution: &ExecutionId,
        result: &CloneExecutionResult,
        request: Option<&Option<RequestId>>,
    ) -> Result<CloneRepositoryMessage, Error> {
        let command = self
            .original_dispatch(session, operation, execution)
            .await?;
        let (node, spec) = match result {
            CloneExecutionResult::CloneReady(ready) => (&ready.node, &ready.spec),
            CloneExecutionResult::CloneFailed(failed) => (&failed.node, &failed.spec),
        };
        if node.node_id != session.node_id
            || *spec != command.payload.spec
            || request.is_some_and(|request| *request != command.request_id)
        {
            return Err(Error::Conflict);
        }
        Ok(command)
    }
}

impl CoordinationStore for CloudStore {
    fn id(&self) -> &ControllerId {
        &self.inner.id
    }

    async fn take_over_node_event(
        &self,
        session: &NodeRuntimeIdentity,
        event: &CloneResultMessage,
    ) -> Result<(), Error> {
        let command = self
            .dispatched(
                session,
                &event.operation_id,
                &event.execution_id,
                &event.payload,
                Some(&event.request_id),
            )
            .await?;
        let epoch = self.epoch()?;
        let result = mapping::result(&event.payload);
        // The exact event travels verbatim so a replay with the same sequence but different
        // content is detected by the authority as a conflict, as the local receipt table does.
        let encoded = serde_json::to_vec(event)?;
        let write = fault::write(|submission_id| {
            let (result, encoded, command) = (result.clone(), encoded.clone(), &command);
            async move {
                let request = self.request(proto::TakeOverNodeEventRequest {
                    submission_id,
                    epoch,
                    operation_id: command.operation_id.as_str().into(),
                    execution_id: command.execution_id.as_str().into(),
                    sequence: event.sequence.value(),
                    result: Some(result),
                    event: encoded,
                });
                self.executions().take_over_node_event(request).await
            }
        });
        write
            .await
            .map(drop)
            .map_err(|verdict| self.settle(verdict))
    }

    async fn record_queried_result(
        &self,
        session: &NodeRuntimeIdentity,
        operation: &OperationId,
        execution: &ExecutionId,
        result: &CloneExecutionResult,
    ) -> Result<(), Error> {
        let command = self
            .dispatched(session, operation, execution, result, /*request*/ None)
            .await?;
        let epoch = self.epoch()?;
        let result = mapping::result(result);
        let write = fault::write(|submission_id| {
            let (result, command) = (result.clone(), &command);
            async move {
                let request = self.request(proto::RecordQueriedResultRequest {
                    submission_id,
                    epoch,
                    operation_id: command.operation_id.as_str().into(),
                    execution_id: command.execution_id.as_str().into(),
                    result: Some(result),
                });
                self.executions().record_queried_result(request).await
            }
        });
        write
            .await
            .map(drop)
            .map_err(|verdict| self.settle(verdict))
    }

    async fn original_dispatch(
        &self,
        session: &NodeRuntimeIdentity,
        operation: &OperationId,
        execution: &ExecutionId,
    ) -> Result<CloneRepositoryMessage, Error> {
        let record = self.record(execution).await?.ok_or(Error::Conflict)?;
        if record.operation_id != operation.as_str() || record.node_id != session.node_id.as_str() {
            return Err(Error::Conflict);
        }
        mapping::command(&record, &self.inner.node)
    }

    async fn pending_dispatches(
        &self,
        node: &NodeId,
    ) -> Result<Vec<CloneRepositoryMessage>, Error> {
        let call = async {
            let request = self.request(proto::ListPendingDispatchesRequest {
                node_id: node.as_str().into(),
            });
            self.executions().list_pending_dispatches(request).await
        };
        let response = fault::read(call)
            .await
            .map_err(|verdict| self.settle(verdict))?;
        response
            .records
            .iter()
            .map(|record| mapping::command(record, node))
            .collect()
    }

    async fn result(&self, execution: &ExecutionId) -> Result<Option<ExecutionOutcome>, Error> {
        self.record(execution)
            .await?
            .and_then(|record| record.result)
            .map(mapping::outcome)
            .transpose()
    }

    fn serve(
        &self,
        shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> impl Future<Output = io::Result<()>> + Send {
        lease::coordinate(self.clone(), shutdown)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn config(controller_id: &str) -> RuntimeConfig {
        RuntimeConfig {
            home_directory: "/nonexistent/controller".into(),
            persistence: Persistence::Cloud {
                endpoint: "http://127.0.0.1:1".into(),
                claim_interval_ms: 100,
            },
            protected_state_directories: Vec::new(),
            controller_id: ControllerId::new(controller_id),
            nodes: vec![NodeTarget {
                node_id: NodeId::new("node"),
                endpoint: NodeEndpoint::Ipc {
                    path: "/nonexistent/control.sock".into(),
                },
            }],
            session: SessionConfig {
                io_timeout_ms: 100,
                query_interval_ms: 10,
            },
            reconnect_ms: 10,
            timezone: "Asia/Shanghai".into(),
        }
    }

    /// Every call names this Controller, which Cloud records as the lease and submission holder.
    #[tokio::test]
    async fn every_request_names_the_controller() {
        let store = CloudStore::open(&config("owner")).unwrap();
        let request = store.request(());
        assert_eq!(request.metadata().get(HOLDER_METADATA).unwrap(), "owner");
    }

    /// An identity that cannot travel as metadata is a deployment error, found before any call.
    #[test]
    fn non_ascii_controller_ids_are_configuration_errors() {
        assert!(matches!(
            CloudStore::open(&config("控制器")),
            Err(Error::Configuration(_))
        ));
    }
}
