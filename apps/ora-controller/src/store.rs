use crate::*;
use std::{collections::BTreeMap, future::Future, io};

/// The durable coordination boundary between clone coordination logic and whichever authority
/// persists it: the local SQLite adapter or the Cloud RPC adapter of a cloud deployment.
///
/// Every method is one atomic business operation that the implementation commits as a whole; the
/// trait deliberately exposes no transaction, connection or table so a remote adapter can honor the
/// same promises with a single request. Implementations are cheap to clone and shared across the
/// runtime, the Node sessions and local intake. The ordering promises are the contract, not a hint:
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

    /// Local clone-only stores have no plugin responsibility; Cloud overrides these operations.
    fn pending_plugins(
        &self,
        _node: &NodeId,
    ) -> impl Future<Output = Result<Vec<PluginCommand>, Error>> + Send {
        async { Ok(Vec::new()) }
    }

    /// Returns a plugin command only when the exact dispatch belongs to this result family.
    fn original_plugin_dispatch(
        &self,
        _session: &NodeRuntimeIdentity,
        _operation: &OperationId,
        _execution: &ExecutionId,
    ) -> impl Future<Output = Result<Option<PluginCommand>, Error>> + Send {
        async { Ok(None) }
    }

    /// Rechecks runtime permission immediately before sending the original plugin input.
    fn dispatch_plugins(
        &self,
        _command: PluginCommand,
    ) -> impl Future<Output = Result<Option<ControllerToNodeMessage>, Error>> + Send {
        async { Err(Error::Conflict) }
    }

    /// Durably takes over an actual plugin event and its receipt before an acknowledgement.
    fn take_over_plugins(
        &self,
        _session: &NodeRuntimeIdentity,
        _event: &PluginsResultMessage,
    ) -> impl Future<Output = Result<(), Error>> + Send {
        async { Err(Error::Conflict) }
    }

    /// Persists a queried plugin result without inventing an event receipt.
    fn record_queried_plugins(
        &self,
        _session: &NodeRuntimeIdentity,
        _operation: &OperationId,
        _execution: &ExecutionId,
        _result: &PluginExecutionResult,
    ) -> impl Future<Output = Result<(), Error>> + Send {
        async { Err(Error::Conflict) }
    }

    /// Cloud session dispatches still awaiting terminal event takeover.
    fn pending_agents(
        &self,
        _node: &NodeId,
    ) -> impl Future<Output = Result<Vec<StartAgentSessionMessage>, Error>> + Send {
        async { Ok(Vec::new()) }
    }
    /// Resolves session ownership without treating another known execution family as a session.
    fn original_agent_dispatch(
        &self,
        _session: &NodeRuntimeIdentity,
        _operation: &OperationId,
        _execution: &ExecutionId,
    ) -> impl Future<Output = Result<Option<StartAgentSessionMessage>, Error>> + Send {
        async { Ok(None) }
    }
    /// Obtains fresh runtime authority for a registered session start.
    fn dispatch_agent(
        &self,
        _command: StartAgentSessionMessage,
    ) -> impl Future<Output = Result<Option<ControllerToNodeMessage>, Error>> + Send {
        async { Err(Error::Conflict) }
    }
    /// Commits an ordered batch before the transport can acknowledge any member.
    fn take_over_thread(
        &self,
        _session: &NodeRuntimeIdentity,
        _events: &[ThreadEventMessage],
    ) -> impl Future<Output = Result<(), Error>> + Send {
        async { Err(Error::Conflict) }
    }
    /// Commits the terminal event only after all preceding Thread batches completed.
    fn take_over_agent_end(
        &self,
        _session: &NodeRuntimeIdentity,
        _event: &AgentSessionEndedMessage,
    ) -> impl Future<Output = Result<(), Error>> + Send {
        async { Err(Error::Conflict) }
    }
    /// Cloud Revision deliveries dispatched to this Node that have no terminal result yet.
    fn pending_deliveries(
        &self,
        _node: &NodeId,
    ) -> impl Future<Output = Result<Vec<DeliverRevisionMessage>, Error>> + Send {
        async { Ok(Vec::new()) }
    }
    /// Resolves delivery ownership without treating another known execution family as a delivery.
    fn original_delivery_dispatch(
        &self,
        _session: &NodeRuntimeIdentity,
        _operation: &OperationId,
        _execution: &ExecutionId,
    ) -> impl Future<Output = Result<Option<DeliverRevisionMessage>, Error>> + Send {
        async { Ok(None) }
    }
    /// Obtains fresh runtime authority for a registered delivery; `None` means not permitted now.
    fn dispatch_delivery(
        &self,
        _command: DeliverRevisionMessage,
    ) -> impl Future<Output = Result<Option<ControllerToNodeMessage>, Error>> + Send {
        async { Err(Error::Conflict) }
    }
    /// Durably takes over an actual delivery terminal event and its receipt before an
    /// acknowledgement. A queried Completed delivery has no such hook: Cloud accepts a delivery
    /// result only together with the Node's sequenced receipt, so the session waits for the event.
    fn take_over_revision(
        &self,
        _session: &NodeRuntimeIdentity,
        _event: &RevisionResultMessage,
    ) -> impl Future<Output = Result<(), Error>> + Send {
        async { Err(Error::Conflict) }
    }
    /// Asks the authority for fresh upload grants of a running delivery. Grants are bearer
    /// credentials: implementations keep them in memory, never persist or log them, and a refusal
    /// is never turned into a delivery failure.
    fn grant_upload(
        &self,
        _session: &NodeRuntimeIdentity,
        _operation: &OperationId,
        _execution: &ExecutionId,
        _request: GrantRequest,
    ) -> impl Future<Output = Result<GrantOutcome, Error>> + Send {
        async { Ok(GrantOutcome::NotGrantable) }
    }

    /// Asks the authority for a fresh read grant of the prior bundle a restoring session needs.
    /// The answer is either granted or refused; a refusal ends the Node's restore, so it is given
    /// only when the authority definitively declines, never for an outage (an `Err` the caller
    /// retries). Grants are bearer credentials: implementations keep them in memory and never
    /// persist or log them. Authorities without sessions refuse.
    fn grant_download(
        &self,
        _session: &NodeRuntimeIdentity,
        operation: &OperationId,
        execution: &ExecutionId,
    ) -> impl Future<Output = Result<DownloadGrantMessage, Error>> + Send {
        let refused = DownloadGrantMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: operation.clone(),
            execution_id: execution.clone(),
            payload: DownloadGrant::Refused {},
        };
        async move { Ok(refused) }
    }

    /// An advisory wake-up; periodic polling remains authoritative if a hint is missed.
    fn wait_agent_command_hint(&self) -> impl Future<Output = ()> + Send {
        std::future::pending()
    }

    /// Returns commands in Cloud creation order; the transport sends only one per execution at once.
    fn pending_agent_commands(
        &self,
        _node: &NodeId,
    ) -> impl Future<Output = Result<Vec<AgentCommand>, Error>> + Send {
        async { Ok(Vec::new()) }
    }
    /// Records delivery after a matching Node accepted/rejected reply, never after a send alone.
    fn agent_command_delivered(
        &self,
        _command: &AgentCommand,
    ) -> impl Future<Output = Result<(), Error>> + Send {
        async { Err(Error::Conflict) }
    }

    /// Cloud requires a negotiated runtime binding; local private IPC keeps its existing intake.
    fn requires_runtime_control(&self) -> bool {
        false
    }

    fn runtime_bindings(
        &self,
        _node: &NodeId,
    ) -> impl Future<Output = Result<Vec<RuntimeBinding>, Error>> + Send {
        async { Ok(Vec::new()) }
    }

    fn acknowledge_runtime_binding(
        &self,
        _state: &RuntimeControlState,
    ) -> impl Future<Output = Result<(), Error>> + Send {
        async { Err(Error::Conflict) }
    }

    /// Registration is historical evidence. This method rechecks current permission before send.
    fn dispatch_message(
        &self,
        command: CloneRepositoryMessage,
    ) -> impl Future<Output = Result<Option<ControllerToNodeMessage>, Error>> + Send {
        async move { Ok(Some(ControllerToNodeMessage::CloneRepository(command))) }
    }

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
/// accepted operation: the local SQLite adapter, reached through [`ControllerHandle`] by whatever
/// embeds a local Controller. A cloud deployment accepts through Cloud's public API and has no
/// catalogue here, so it does not implement this trait and offers no local intake at all.
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

/// Why a Node session asks for upload grants of one delivery execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GrantRequest {
    /// The Node reported that it holds no valid grant for these objects, each with the SHA-256 it
    /// froze before its first PUT; the grants are bound to exactly these digests.
    Needed(BTreeMap<ObjectKey, Sha256Digest>),
    /// A new connection found the delivery still running. Only digests the Node already reported
    /// to this process can be reused; without them the Node asks again itself.
    Resumed,
}

/// What a grant request produced. Deliberately not `Debug`: an issued grant carries signed headers
/// that must not reach diagnostics.
pub enum GrantOutcome {
    /// Fresh grants for the Node, to be sent and then forgotten.
    Issued(UploadGrantMessage),
    /// The delivery is no longer grantable (it has a result, its run stopped delivering, or the
    /// request named foreign objects). This is never a delivery failure; Cloud settles the run.
    NotGrantable,
    /// No checksums are known for a resumed delivery; the Node's `UploadGrantNeeded` will carry them.
    AwaitNode,
}
