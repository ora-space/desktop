//! Lifecycle frames are authenticated memory operations, never durable commands or events.
use super::*;

pub(crate) enum Action {
    Update(UpdateSessionMcpMessage),
    Get(GetSessionMcpStateMessage),
    Probe(ProbeSessionMcpMessage),
}

/// Authorization precedes reading the session or retaining values; replies have no sequence or Ack.
pub(crate) fn handle(
    node: &ManagedNode,
    host: Option<&SessionHost>,
    controller: &ControllerId,
    action: Action,
) -> Result<Vec<NodeToControllerMessage>, crate::Error> {
    let (operation, execution, target, request) = match &action {
        Action::Update(input) => {
            input.validate()?;
            (
                &input.operation_id,
                &input.execution_id,
                &input.payload.node_id,
                &input.payload.request_id,
            )
        }
        Action::Get(input) => {
            input.validate()?;
            (
                &input.operation_id,
                &input.execution_id,
                &input.payload.node_id,
                &input.payload.request_id,
            )
        }
        Action::Probe(input) => {
            input.validate()?;
            (
                &input.operation_id,
                &input.execution_id,
                &input.payload.node_id,
                &input.payload.request_id,
            )
        }
    };
    if target != &node.identity().node_id {
        return Err(ora_node_db::Error::NodeMismatch.into());
    }
    node.database
        .check_controller_execution(controller, operation, execution)?;
    node.database
        .authorize_session_command(execution, &node.identity().incarnation_id)?;
    let record = node.database.find_session(operation, execution)?;
    let mut code = None;
    let mut state = None;
    let result = match record {
        None => SessionMcpResult::NotFound,
        Some(record) if matches!(record.state, ExecutionState::Completed(_)) => {
            SessionMcpResult::SessionEnded
        }
        Some(record) if !record.command.payload.spec.mcp_lifecycle => SessionMcpResult::NotEnabled,
        Some(_) => match host.and_then(|host| host.mcp_control(execution)) {
            None => SessionMcpResult::SessionEnded,
            Some(control) => {
                let result = match &action {
                    Action::Update(input) => {
                        let result = control.update(&input.payload);
                        code = result.1;
                        result.0
                    }
                    Action::Get(_) => SessionMcpResult::Accepted,
                    Action::Probe(input) => control.probe(&input.payload),
                };
                if result != SessionMcpResult::SessionEnded {
                    state = Some(
                        control.snapshot(host.is_some_and(SessionHost::probe_capacity_available)),
                    );
                }
                result
            }
        },
    };
    Ok(vec![NodeToControllerMessage::SessionMcpReply(
        SessionMcpReplyMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: operation.clone(),
            execution_id: execution.clone(),
            payload: SessionMcpReply {
                node: node.identity().clone(),
                request_id: request.clone(),
                result,
                code,
                state,
            },
        },
    )])
}
