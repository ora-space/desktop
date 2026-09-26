use crate::*;
use std::{future::Future, io};

/// The durable coordination boundary between clone coordination logic and whichever authority
/// persists it: the local SQLite adapter or the Cloud RPC adapter of a cloud deployment.
///
/// Every method is one atomic business operation that the implementation commits as a whole; the
/// trait deliberately exposes no transaction, connection or table so a remote adapter can honor the
/// same promises with a single request. Implementations are cheap to clone and shared across the
/// runtime, the Node sessions and the API. The ordering promises are the contract, not a hint:
///
/// - `take_over_node_event` returns only after the execution fact and the exact event receipt are
///   durable; callers acknowledge that sequence to the Node only after it succeeds.
/// - `record_queried_result` stores a result learned by querying the Node and never produces a
///   receipt, so it can never justify an acknowledgement.
/// - Reads distinguish an unknown identity (conflict or `None`) from an accepted execution whose
///   result is not known yet.
pub trait CoordinationStore: Clone + Send + Sync + 'static {
    /// The persistent coordinator identity presented to Nodes; never a process or connection identity.
    fn id(&self) -> &ControllerId;

    /// Commits the execution fact carried by a Node event together with its exact receipt.
    fn take_over_node_event(
        &self,
        session: &NodeRuntimeIdentity,
        event: &CloneResultMessage,
    ) -> impl Future<Output = Result<(), Error>> + Send;

    /// Commits a Completed result learned by query; identical facts are idempotent, differing ones conflict.
    fn record_queried_result(
        &self,
        session: &NodeRuntimeIdentity,
        operation: &OperationId,
        execution: &ExecutionId,
        result: &CloneExecutionResult,
    ) -> impl Future<Output = Result<(), Error>> + Send;

    /// Resolves the exact original command before any remote fact is trusted or retransmitted.
    fn original_dispatch(
        &self,
        session: &NodeRuntimeIdentity,
        operation: &OperationId,
        execution: &ExecutionId,
    ) -> impl Future<Output = Result<CloneRepositoryMessage, Error>> + Send;

    /// Lists the commands dispatched to one Node that have no durable result yet: the executions a
    /// session keeps querying after reconnecting. Completed executions leave this list; their replayed
    /// events are still verified through `original_dispatch`, so nothing is lost by not polling them.
    fn pending_dispatches(
        &self,
        node: &NodeId,
    ) -> impl Future<Output = Result<Vec<CloneRepositoryMessage>, Error>> + Send;

    /// Reads the durable terminal outcome without acknowledging anything; `None` is an execution
    /// without a result or an unknown identity, never a failure.
    fn result(
        &self,
        execution: &ExecutionId,
    ) -> impl Future<Output = Result<Option<ExecutionOutcome>, Error>> + Send;

    /// Learns that a session with a statically configured Node completed its handshake, which
    /// proves that the configured `NodeId` names the Node actually behind the endpoint. The Cloud
    /// adapter registers tenant work to that Node only after this, so a misconfigured identity
    /// leaves the work queued with Cloud instead of pending forever on a Node that does not exist.
    /// The local adapter records dispatches at intake and ignores it.
    fn static_node_established(&self, node: &NodeRuntimeIdentity);

    /// Runs the adapter's own coordination with its authority until `shutdown` resolves, then
    /// releases what it held. A remote authority needs its lease kept and accepted work claimed
    /// and registered; the local adapter, which accepts work itself, has nothing to do. The
    /// runtime runs it once, beside the Node sessions, and stops it after they stopped.
    fn serve(
        &self,
        shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> impl Future<Output = io::Result<()>> + Send;
}

/// The intake side of an authority that accepts caller requests itself and can catalogue every
/// accepted operation: the local SQLite adapter behind the transitional JSON surface. A cloud
/// deployment accepts through Cloud's public API and has no catalogue here, so it does not implement
/// this trait and the JSON surface is never composed for it instead of answering with errors.
pub trait CloneIntake: CoordinationStore {
    /// Freezes caller intent and the original dispatch identities before any network operation.
    fn accept_request(
        &self,
        request: RequestId,
        spec: CloneExecutionSpec,
    ) -> impl Future<Output = Result<CloneRepositoryMessage, Error>> + Send;

    /// Lists accepted operations for presentation, newest first.
    fn operations(&self) -> impl Future<Output = Result<Vec<CloneOperation>, Error>> + Send;

    /// Reads one operation; `None` is an absent identity, not a pending result.
    fn operation(
        &self,
        execution: &ExecutionId,
    ) -> impl Future<Output = Result<Option<CloneOperation>, Error>> + Send;
}

/// The terminal fact of one execution in the shape every authority persists and reports: the Node
/// incarnation that produced it and its outcome. The echoed spec and the Node-local repository
/// identity of the wire result are not part of it because the Cloud contract deliberately does not
/// carry them; the local catalogue keeps the full wire result for its own presentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecutionOutcome {
    Ready {
        node: NodeRuntimeIdentity,
        path: NodePath,
        commit: CommitId,
    },
    Failed {
        node: NodeRuntimeIdentity,
        failure: CloneFailureCode,
        retained_path: Option<NodePath>,
    },
}

impl From<&CloneExecutionResult> for ExecutionOutcome {
    /// Projects a wire result onto the authority-neutral outcome.
    fn from(result: &CloneExecutionResult) -> Self {
        match result {
            CloneExecutionResult::CloneReady(ready) => Self::Ready {
                node: ready.node.clone(),
                path: ready.path.clone(),
                commit: ready.commit.clone(),
            },
            CloneExecutionResult::CloneFailed(failed) => Self::Failed {
                node: failed.node.clone(),
                failure: failed.failure,
                retained_path: match &failed.residual {
                    CloneResidual::NoDirectory {} => None,
                    CloneResidual::Retained { path, .. } => Some(path.clone()),
                },
            },
        }
    }
}
