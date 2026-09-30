//! Turns work Cloud accepted into registered dispatches the Node sessions then deliver. A claim is
//! a pure read; ownership is decided when `RecordDispatch` commits, so claiming more often than
//! needed costs queries but never duplicates work.
use super::{
    CloudStore, fault,
    fleet::{Fleet, Gate},
    mapping,
};
use crate::*;
use ora_controller_proto::v1 as proto;

/// Items registered per batch before the coordinator gets a turn: a long queue must not hold up
/// shutdown or a renewal until the lease expires.
const BATCH: usize = 16;

/// What one batch left behind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Backlog {
    /// The queue is empty, a step failed, or the lease is gone: wait for the next trigger.
    Settled,
    /// A full batch registered; more work is probably queued and the coordinator continues soon.
    Pending,
}

/// The queue head this Controller already warned about. It stays the head until Cloud resolves
/// it, so one warning per item replaces one per trigger. The same holds for the wait on the static
/// Node's handshake, which every trigger would otherwise report again.
#[derive(Default)]
pub(super) struct Refusals {
    last: Option<String>,
    unverified: bool,
}

/// Claims and registers accepted work until Cloud has none left, a step fails, or one batch is
/// full. One trigger drains the backlog, because the renewal backstop comes only every ten
/// seconds and a backlog left by dropped signals would otherwise shrink by one item per backstop.
pub(super) async fn batch(
    store: &CloudStore,
    refusals: &mut Refusals,
    fleet: Option<&Fleet>,
) -> Backlog {
    if fleet.is_none() && (store.inner.node.is_none() || !*store.inner.node_verified.borrow()) {
        return Backlog::Settled;
    }
    for _ in 0..BATCH {
        // The lease is re-read per item: a stale verdict inside the batch drops it.
        let Ok(epoch) = store.epoch() else {
            return Backlog::Settled;
        };
        let claimed = fault::write(|submission_id| async move {
            let request = store.request(proto::ClaimWorkRequest {
                submission_id,
                epoch,
            });
            store.executions().claim_work(request).await
        })
        .await;
        let item = match claimed {
            Ok(proto::ClaimWorkResponse { item: Some(item) }) => item,
            Ok(proto::ClaimWorkResponse { item: None }) => return Backlog::Settled,
            Err(verdict) => {
                let error = store.settle(verdict);
                ora_logging::ora_warn!(error = %error, "Cloud work claim failed");
                return Backlog::Settled;
            }
        };
        if let Some(target) = &item.target {
            let Some(sandbox) = fleet.and_then(|fleet| fleet.get(&target.sandbox_instance_id))
            else {
                return Backlog::Settled;
            };
            if sandbox.binding.node_id.as_str() != target.node_id || !sandbox.agent_capable() {
                return Backlog::Settled;
            }
            // Quiesce cannot race a new registration into the supposedly idle sandbox.
            let Ok(gate) = sandbox.gate.try_lock() else {
                return Backlog::Settled;
            };
            if *gate != Gate::Open {
                return Backlog::Settled;
            }
            if let Err(error) = store
                .record_agent(epoch, &item, &sandbox.binding.node_id)
                .await
            {
                ora_logging::ora_warn!(error = %error, "session work remains queued in Cloud");
                return Backlog::Settled;
            }
            continue;
        }
        let Some(node) = &store.inner.node else {
            return Backlog::Settled;
        };
        if !*store.inner.node_verified.borrow() {
            if !refusals.unverified {
                ora_logging::ora_info!("tenant work waits for its configured Node handshake");
                refusals.unverified = true;
            }
            return Backlog::Settled;
        }
        let registered = match mapping::spec(item.input.clone(), node) {
            Ok(spec) => {
                let operation = OperationId::new(item.operation_id.clone());
                record_dispatch(store, epoch, operation, spec).await
            }
            Err(error) => Err(error),
        };
        match registered {
            Ok(command) => {
                refusals.last = None;
                ora_logging::ora_info!(
                    operation_id = %command.operation_id.as_str(),
                    execution_id = %command.execution_id.as_str(),
                    "dispatch recorded with Cloud; the Node session delivers it"
                );
            }
            // A head that cannot be registered is returned by every claim until Cloud resolves it,
            // so the batch stops rather than spin on it.
            Err(error) => {
                if refusals.last.as_deref() != Some(item.operation_id.as_str()) {
                    ora_logging::ora_warn!(
                        operation_id = %item.operation_id,
                        error = %error,
                        "claimed work could not be registered for dispatch"
                    );
                    refusals.last = Some(item.operation_id);
                }
                return Backlog::Settled;
            }
        }
    }
    Backlog::Pending
}

/// Freezes a new execution identity and the full input with Cloud, for a tenant work item or a
/// Workspace operation's clone step alike; the Node protocol validates the command first so
/// nothing undispatchable is ever registered. The Node session delivers it afterwards.
pub(super) async fn record_dispatch(
    store: &CloudStore,
    epoch: i64,
    operation: OperationId,
    spec: CloneExecutionSpec,
) -> Result<CloneRepositoryMessage, Error> {
    let command = CloneRepositoryMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        request_id: None,
        operation_id: operation,
        execution_id: ExecutionId::new(uuid::Uuid::new_v4().to_string()),
        payload: CloneRepository { spec },
    };
    command.validate()?;
    let write = fault::write(|submission_id| {
        let command = &command;
        async move {
            let request = store.request(proto::RecordDispatchRequest {
                submission_id,
                epoch,
                operation_id: command.operation_id.as_str().into(),
                execution_id: command.execution_id.as_str().into(),
                node_id: command.payload.spec.node_id.as_str().into(),
                input: Some(mapping::input(&command.payload.spec)),
            });
            store.executions().record_dispatch(request).await
        }
    });
    let response = write.await.map_err(|verdict| store.settle(verdict))?;
    mapping::command(
        &response.record.ok_or(Error::Conflict)?,
        &command.payload.spec.node_id,
    )
}
