//! Production composition and durable admission of Agent sessions.
use super::AgentConfig;
use crate::{AgentSessions, DirectoryPluginCatalog, ManagedNode, SessionConfig, SessionHost as _};
use ora_node_db::{CommandAdmission, SessionCommandInput, SessionJournal};
use ora_node_protocol::*;
use std::time::Duration;

pub(super) type SessionHost = AgentSessions<SessionJournal, SessionJournal, DirectoryPluginCatalog>;

/// Recovers every unfinished input before the control listener is published, even without Agent
/// configuration. Recovery never spawns an Agent or treats the old input as permission to resume.
pub(super) fn open(
    node: &mut ManagedNode,
    config: Option<&AgentConfig>,
    timezone: &str,
    catalog: DirectoryPluginCatalog,
) -> Result<Option<SessionHost>, crate::Error> {
    let journal = node.database.session_journal()?;
    let host = config
        .map(|config| {
            let timezone = timezone
                .parse()
                .map_err(|_| crate::Error::Configuration("invalid Agent timezone".into()))?;
            Ok::<_, crate::Error>(AgentSessions::new(
                SessionConfig {
                    home_directory: node.home_directory().to_path_buf(),
                    deno_path: config.deno_path.clone(),
                    timezone,
                    agent_ready_timeout: Duration::from_millis(config.ready_timeout_ms),
                    model_proxy: config.model_proxy.clone(),
                },
                node.identity().clone(),
                journal.clone(),
                journal.clone(),
                catalog,
            ))
        })
        .transpose()?;
    for record in node.database.recoverable_sessions()? {
        let ended = match &host {
            Some(host) => host.recover_interrupted(&record.command.execution_id),
            None => AgentSessionEnded {
                node: node.identity().clone(),
                reason: AgentSessionEndReason::Interrupted,
                detail: None,
            },
        };
        journal.end_session(&record.command.execution_id, ended)?;
    }
    Ok(host)
}

/// A repeated start only reports durable state; it never relaunches an existing session.
pub(super) fn start(
    node: &mut ManagedNode,
    host: &SessionHost,
    controller: &ControllerId,
    input: &StartAgentSessionMessage,
    permit: Option<&RuntimeBinding>,
) -> Result<Vec<NodeToControllerMessage>, crate::Error> {
    if input.payload.spec.model_binding_id.is_some() && !host.model_capable() {
        return Err(crate::Error::UnsupportedMessage);
    }
    node.database.check_controller_execution(
        controller,
        &input.operation_id,
        &input.execution_id,
    )?;
    let record = match permit {
        Some(permit) => {
            if permit.node_incarnation_id != node.identity().incarnation_id.as_str() {
                return Err(ora_node_db::Error::NodeMismatch.into());
            }
            node.database.accept_controlled_session(input, permit)?
        }
        None => node.database.accept_session(input)?,
    };
    // An earlier write failure may leave Accepted in this incarnation. It has never
    // started an actor and may retry its guarded transition; Running is never restarted.
    if record.state == ExecutionState::Accepted {
        if node
            .database
            .start_session(input, &node.identity().incarnation_id.clone())?
        {
            host.start(input.execution_id.clone(), input.payload.spec.clone());
        } else {
            node.database.session_journal()?.end_session(
                &input.execution_id,
                host.recover_interrupted(&input.execution_id),
            )?;
        }
    }
    let state = node
        .database
        .execution_state(&input.operation_id, &input.execution_id)?;
    Ok(vec![NodeToControllerMessage::ExecutionStatus(
        ExecutionStatusMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: input.operation_id.clone(),
            execution_id: input.execution_id.clone(),
            payload: ExecutionStatus {
                node: node.identity().clone(),
                state,
            },
        },
    )])
}

/// Admission does not wake the actor: the transport does that after sending the accepted reply.
pub(super) fn command(
    node: &mut ManagedNode,
    controller: &ControllerId,
    input: SessionCommandInput,
) -> Result<Vec<NodeToControllerMessage>, crate::Error> {
    node.database.check_controller_execution(
        controller,
        input.operation_id(),
        input.execution_id(),
    )?;
    let state = node
        .database
        .execution_state(input.operation_id(), input.execution_id())?;
    if !matches!(state, ExecutionState::Completed(_)) {
        node.database
            .authorize_session_command(input.execution_id(), &node.identity().incarnation_id)?;
    }
    let result = node.database.accept_session_command(&input)?;
    let operation_id = input.operation_id().clone();
    let execution_id = input.execution_id().clone();
    let command_id = input.command_id().clone();
    Ok(vec![match result {
        CommandAdmission::Accepted => {
            NodeToControllerMessage::SessionCommandAccepted(SessionCommandAcceptedMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id,
                execution_id,
                payload: SessionCommandAccepted { command_id },
            })
        }
        CommandAdmission::SessionEnded => {
            NodeToControllerMessage::SessionCommandRejected(SessionCommandRejectedMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id,
                execution_id,
                payload: SessionCommandRejected {
                    command_id,
                    reason: SessionCommandRejection::SessionEnded,
                },
            })
        }
    }])
}
