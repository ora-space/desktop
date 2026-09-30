//! Lossless translation of Cloud-owned session inputs into the Node protocol.
use crate::*;
use ora_controller_proto::v1 as proto;

/// Identifies only session work; delivery remains a separate execution family.
pub(in crate::cloud) fn is_agent(record: &proto::ExecutionRecord) -> bool {
    matches!(
        record.input.as_ref().and_then(|v| v.spec.as_ref()),
        Some(proto::execution_input::Spec::AgentSession(_))
    )
}

/// Reconstructs a recorded start, using Cloud's stable Node-local operation identity.
pub(in crate::cloud) fn start(
    record: &proto::ExecutionRecord,
    node: &NodeId,
) -> Result<StartAgentSessionMessage, Error> {
    if record.node_id != node.as_str() {
        return Err(Error::Conflict);
    }
    let Some(proto::execution_input::Spec::AgentSession(spec)) =
        record.input.as_ref().and_then(|v| v.spec.as_ref())
    else {
        return Err(Error::Conflict);
    };
    let identity = spec.git_identity.as_ref().ok_or(Error::Conflict)?;
    let command = StartAgentSessionMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: OperationId::new(record.node_operation_id.clone()),
        execution_id: ExecutionId::new(record.execution_id.clone()),
        payload: StartAgentSession {
            spec: AgentSessionSpec {
                node_id: node.clone(),
                agent_plugin_id: PluginId::new(spec.agent_plugin_id.clone()),
                agent_plugin_version: PluginVersion::new(spec.agent_plugin_version.clone()),
                checkout_execution_id: ExecutionId::new(spec.checkout_execution_id.clone()),
                git_identity: GitIdentity {
                    name: identity.name.clone(),
                    email: identity.email.clone(),
                },
                initial_turn: turn(spec.initial_turn.as_ref().ok_or(Error::Conflict)?)?,
            },
        },
    };
    command.validate()?;
    Ok(command)
}

/// Rejects unknown content blocks rather than silently losing part of a prompt.
fn turn(input: &proto::UserTurn) -> Result<UserTurn, Error> {
    Ok(UserTurn {
        turn_id: TurnId::new(input.turn_id.clone()),
        content: input
            .content
            .iter()
            .map(|block| match &block.block {
                Some(proto::content_block::Block::Text(text)) => Ok(ContentBlock::Text {
                    text: text.text.clone(),
                }),
                None => Err(Error::Conflict),
            })
            .collect::<Result<_, _>>()?,
    })
}

/// Correlates a command with its registered run, execution and destination before transmission.
pub(super) fn command(
    input: &proto::ThreadCommand,
    record: &proto::ExecutionRecord,
    node: &NodeId,
) -> Result<AgentCommand, Error> {
    let start = start(record, node)?;
    if input.run_id != record.operation_id
        || input.execution_id != record.execution_id
        || input
            .target
            .as_ref()
            .is_none_or(|target| target.node_id != node.as_str())
    {
        return Err(Error::Conflict);
    }
    let command = match input.command.as_ref().ok_or(Error::Conflict)? {
        proto::thread_command::Command::SubmitUserTurn(submit) => {
            AgentCommand::Submit(SubmitUserTurnMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: start.operation_id,
                execution_id: start.execution_id,
                payload: SubmitUserTurn {
                    node_id: node.clone(),
                    command_id: CommandId::new(input.command_id.clone()),
                    turn: turn(submit.turn.as_ref().ok_or(Error::Conflict)?)?,
                },
            })
        }
        proto::thread_command::Command::EndSession(end) => AgentCommand::End(EndSessionMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: start.operation_id,
            execution_id: start.execution_id,
            payload: EndSession {
                node_id: node.clone(),
                command_id: CommandId::new(input.command_id.clone()),
                reason: match proto::EndSessionReason::try_from(end.reason)
                    .map_err(|_| Error::Conflict)?
                {
                    proto::EndSessionReason::UserEnded => EndSessionReason::UserEnded,
                    proto::EndSessionReason::IdleTimeout => EndSessionReason::IdleTimeout,
                    proto::EndSessionReason::Cancelled => EndSessionReason::Cancelled,
                    proto::EndSessionReason::Unspecified => return Err(Error::Conflict),
                },
            },
        }),
    };
    command.message().validate()?;
    Ok(command)
}

/// Preserves the producing incarnation, including when a restarted Node replays old evidence.
pub(super) fn result(value: &AgentSessionResult) -> proto::ExecutionResult {
    let AgentSessionResult::AgentSessionEnded(ended) = value;
    proto::ExecutionResult {
        node: Some(proto::NodeIdentity {
            node_id: ended.node.node_id.as_str().into(),
            node_incarnation_id: ended.node.incarnation_id.as_str().into(),
        }),
        outcome: Some(proto::execution_result::Outcome::AgentSessionEnded(
            proto::AgentSessionEnded {
                reason: match ended.reason {
                    AgentSessionEndReason::UserEnded => proto::AgentSessionEndReason::UserEnded,
                    AgentSessionEndReason::IdleTimeout => proto::AgentSessionEndReason::IdleTimeout,
                    AgentSessionEndReason::Cancelled => proto::AgentSessionEndReason::Cancelled,
                    AgentSessionEndReason::AgentFailed => proto::AgentSessionEndReason::AgentFailed,
                    AgentSessionEndReason::Interrupted => proto::AgentSessionEndReason::Interrupted,
                } as i32,
                detail: ended.detail.clone(),
            },
        )),
    }
}
