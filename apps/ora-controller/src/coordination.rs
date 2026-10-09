use crate::*;

/// Takes one Node message through the store boundary. Only an actually received event can produce
/// an Ack, and only after the store confirmed the takeover; queries never acknowledge anything.
pub async fn take_over<S: CoordinationStore>(
    store: &S,
    session: &NodeRuntimeIdentity,
    message: &NodeToControllerMessage,
) -> Result<Option<EventAckMessage>, Error> {
    message.validate()?;
    match message {
        NodeToControllerMessage::RuntimeControlState(state) => {
            if state.binding.node_id != session.node_id.as_str()
                || state.binding.node_incarnation_id != session.incarnation_id.as_str()
            {
                return Err(Error::Conflict);
            }
            store.acknowledge_runtime_binding(state).await?;
            Ok(None)
        }
        NodeToControllerMessage::PluginsResult(event) => {
            store.take_over_plugins(session, event).await?;
            Ok(Some(EventAckMessage { protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: event.operation_id.clone(), execution_id: event.execution_id.clone(), sequence: event.sequence,
                payload: EventAck { node_id: session.node_id.clone() },
            }))
        }
        NodeToControllerMessage::RevisionResult(event) => {
            store.take_over_revision(session, event).await?;
            Ok(Some(EventAckMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: event.operation_id.clone(),
                execution_id: event.execution_id.clone(),
                sequence: event.sequence,
                payload: EventAck {
                    node_id: session.node_id.clone(),
                },
            }))
        }
        NodeToControllerMessage::CloneResult(event) => {
            store.take_over_node_event(session, event).await?;
            Ok(Some(EventAckMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: event.operation_id.clone(),
                execution_id: event.execution_id.clone(),
                sequence: event.sequence,
                payload: EventAck {
                    node_id: session.node_id.clone(),
                },
            }))
        }
        NodeToControllerMessage::ExecutionStatus(status) => {
            if status.payload.node != *session {
                return Err(Error::Conflict);
            }
            match &status.payload.state {
                ExecutionState::Completed(ExecutionResult::Clone(result)) => {
                    store
                        .record_queried_result(
                            session,
                            &status.operation_id,
                            &status.execution_id,
                            result,
                        )
                        .await?;
                }
                ExecutionState::Completed(ExecutionResult::Plugin(result)) => {
                    store.record_queried_plugins(session, &status.operation_id, &status.execution_id, result).await?;
                }
                ExecutionState::Completed(ExecutionResult::AgentSession(result)) => {
                    let AgentSessionResult::AgentSessionEnded(ended) = result;
                    if ended.node.node_id != session.node_id || store.original_agent_dispatch(session, &status.operation_id, &status.execution_id).await?.is_none() { return Err(Error::Conflict); }
                    // A query has no event sequence. Wait for the durable terminal envelope so
                    // Cloud cannot settle the session before preceding Thread records arrive.
                }
                ExecutionState::Completed(ExecutionResult::Revision(result)) => {
                    let (RevisionExecutionResult::RevisionDelivered(RevisionDelivered { node, .. })
                    | RevisionExecutionResult::RevisionUnchanged(RevisionUnchanged { node, .. })
                    | RevisionExecutionResult::RevisionFailed(RevisionFailed { node, .. })) = result.as_ref();
                    if node.node_id != session.node_id || store.original_delivery_dispatch(session, &status.operation_id, &status.execution_id).await?.is_none() { return Err(Error::Conflict); }
                    // Cloud accepts a delivery result only with the Node's sequenced receipt, so a
                    // query never settles it; the retained terminal envelope does.
                }
                // This Controller dispatches clones, plugins, sessions and deliveries; a Worktree
                // result cannot belong to one of its dispatches.
                ExecutionState::Completed(ExecutionResult::Worktree(_)) => {
                    return Err(Error::Conflict);
                }
                // A status for an unknown dispatch is a conflict even when it carries no result.
                ExecutionState::Unknown | ExecutionState::Accepted | ExecutionState::Running => {
                    if store.original_agent_dispatch(session, &status.operation_id, &status.execution_id).await?.is_none() && store.original_delivery_dispatch(session, &status.operation_id, &status.execution_id).await?.is_none() && store.original_plugin_dispatch(session, &status.operation_id, &status.execution_id).await?.is_none() {
                        store.original_dispatch(session, &status.operation_id, &status.execution_id).await?;
                    }
                }
            }
            Ok(None)
        }
        NodeToControllerMessage::Heartbeat(heartbeat) if heartbeat.payload.node == *session => {
            Ok(None)
        }
        NodeToControllerMessage::Heartbeat(_)
        | NodeToControllerMessage::HelloAccepted(_)
        | NodeToControllerMessage::WorktreeReady(_)
        | NodeToControllerMessage::WorktreeFailed(_)
        | NodeToControllerMessage::WorktreeRemoved(_)
        | NodeToControllerMessage::WorktreeRemovalFailed(_)
        // Session workers handle these ordered events, correlated replies and memory-only grant
        // requests; this single-message boundary cannot bypass them.
        | NodeToControllerMessage::ThreadEvent(_)
        | NodeToControllerMessage::AgentSessionEnded(_)
        | NodeToControllerMessage::SessionCommandAccepted(_)
        | NodeToControllerMessage::SessionCommandRejected(_)
        | NodeToControllerMessage::UploadGrantNeeded(_) => Err(Error::Conflict),
    }
}
