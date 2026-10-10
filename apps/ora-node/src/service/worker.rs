use super::{clones::Clones, plugins::Plugins, revisions::Revisions, *};
use crate::{ManagedNode, Node};
use ora_node_db::Error as StorageError;
use ora_node_transport::CloseReason;
use std::time::Instant;

/// Owns the only mutable Node and database. Clone Git and plugin downloads run on separate
/// executors so admission waits only for SQLite and local directory work.
pub(super) fn run(
    config: ServiceConfig,
    receiver: mpsc::Receiver<Work>,
    ready: oneshot::Sender<Result<SessionInfo, String>>,
    shutdown: Shutdown,
) -> Result<(), String> {
    let target = config.control.as_ref().and_then(|c| c.target.clone());
    if config
        .control
        .as_ref()
        .is_some_and(|c| matches!(c.listen, ControlListen::MutualTlsWebSocket { .. }))
        && target.is_none()
    {
        return Err("network management requires a platform-assigned runtime scope".into());
    }
    let controlled = config.control.as_ref().is_some_and(|c| {
        matches!(
            c.listen,
            ControlListen::WebSocket { .. } | ControlListen::MutualTlsWebSocket { .. }
        )
    });
    // A stop may let the outstanding step settle its Run before the final cleanup.
    let drain = Duration::from_millis(
        config
            .process
            .shutdown_grace_ms
            .saturating_add(config.process.cleanup_timeout_ms),
    );
    let initialized = (|| {
        // Read before the process configuration moves into the Node.
        let workload =
            agents::workload(config.agent.as_ref(), &config.process).map_err(|e| e.to_string())?;
        let mut node =
            Node::open(config.node, config.process, shutdown.clone()).map_err(|e| e.to_string())?;
        if controlled {
            let incarnation = node.identity().incarnation_id.as_str().to_owned();
            node.database
                .enforce_runtime_incarnation(&incarnation)
                .map_err(|e| e.to_string())?;
        }
        if let Some(clone) = config.clone {
            if controlled {
                clone.validate_cloud_policy().map_err(|e| e.to_string())?;
            }
            node.configure_clone(clone).map_err(|e| e.to_string())?;
        }
        let controller = config
            .control
            .as_ref()
            .map(|control| control.controller_id.clone())
            .unwrap_or_else(|| ControllerId::new("recovery-only"));
        if config.control.is_some() {
            node.database
                .bind_controller(&controller)
                .map_err(|e| e.to_string())?;
        }
        let clones = Clones::start(&node).map_err(|e| e.to_string())?;
        let plugins = Plugins::start(&node).map_err(|e| e.to_string())?;
        let grants = crate::revision::GrantStore::new();
        let revisions =
            Revisions::start(&node, grants.clone(), shutdown.clone()).map_err(|e| e.to_string())?;
        let downloads = crate::revision::DownloadGrants::new();
        let restorer = agents::restorer(&node, config.agent.as_ref(), downloads.clone())
            .map_err(|e| e.to_string())?;
        let agents = agents::open(
            &mut node,
            config.agent.as_ref(),
            workload,
            &config.timezone,
            plugins.catalog.clone(),
            restorer,
        )
        .map_err(|e| e.to_string())?;
        Ok::<_, String>((
            node, controller, clones, plugins, agents, grants, downloads, revisions,
        ))
    })();
    let (mut node, controller, mut clones, mut plugins, agents, grants, downloads, mut revisions) =
        match initialized {
            Ok(value) => value,
            Err(error) => {
                let _ = ready.send(Err(error.clone()));
                return Err(error);
            }
        };
    let mut capabilities = vec![
        NodeCapability::RepositoryClone,
        NodeCapability::PluginInstall,
        NodeCapability::RuntimeControl,
    ];
    if agents.is_some() {
        capabilities.push(NodeCapability::AgentSession);
    }
    if revisions.is_some() {
        capabilities.push(NodeCapability::RevisionDelivery);
    }
    // Restore runs Git in session checkouts with delivery's policy, so it needs both.
    if agents.is_some() && revisions.is_some() {
        capabilities.push(NodeCapability::RevisionRestore);
    }
    let info = SessionInfo {
        agents: agents.clone(),
        grants,
        downloads,
        identity: node.identity().clone(),
        controller: controller.clone(),
        capabilities,
    };
    if ready.send(Ok(info)).is_err() {
        if let Some(agents) = &agents {
            tokio::runtime::Handle::current().block_on(agents.shutdown());
        }
        plugins.finish(&mut node).map_err(|e| e.to_string())?;
        if let Some(revisions) = revisions {
            revisions.finish(&mut node).map_err(|e| e.to_string())?;
        }
        clones.drain(&mut node, Duration::ZERO)?;
        return node.shutdown().map_err(|e| e.to_string());
    }
    ora_logging::ora_info!(node_id = %node.node_id().as_str(), "Node opened");
    let mut next = Instant::now();
    let result = (|| {
        while !shutdown.requested() {
            if agents.as_ref().is_some_and(agents::SessionHost::failed) {
                return Err("Agent actor stopped without durable terminal evidence".into());
            }
            match receiver.recv_timeout(Duration::from_millis(/*millis*/ 25)) {
                Ok(work) => {
                    let active = work
                        .active
                        .lock()
                        .map_err(|_| "session admission poisoned".to_owned())?;
                    let result = if *active {
                        handle(
                            &mut node,
                            &mut clones,
                            agents.as_ref(),
                            revisions.is_some(),
                            &controller,
                            controlled,
                            target.as_ref(),
                            work.request,
                        )
                        .map_err(|error| Rejection {
                            close: close_reason(&error),
                            message: error.to_string(),
                        })
                    } else {
                        // The session is already ending; nobody reads this reason.
                        Err(Rejection {
                            close: CloseReason::InternalError,
                            message: "session revoked".into(),
                        })
                    };
                    let _ = work.reply.send(result);
                    drop(active);
                }
                Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => {}
            }
            if Instant::now() >= next {
                // Worktree recovery stays here: the session never admits Worktree commands, so
                // only state written by other embeddings can reach it.
                node.recover().map_err(|e| e.to_string())?;
                clones.recovery_pass(&mut node)?;
                next = Instant::now() + Duration::from_millis(config.recovery_interval_ms);
            }
            clones.advance(&mut node)?;
            plugins.advance(&mut node).map_err(|e| e.to_string())?;
            if let Some(revisions) = revisions.as_mut() {
                revisions
                    .advance(&mut node, agents.as_ref())
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    })();
    if let Some(agents) = &agents {
        tokio::runtime::Handle::current().block_on(agents.shutdown());
    }
    let plugins_finished = plugins.finish(&mut node).map_err(|e| e.to_string());
    let revisions_finished = revisions
        .map_or(Ok(()), |revisions| revisions.finish(&mut node))
        .map_err(|e| e.to_string());
    let agents_finished = if agents.as_ref().is_some_and(agents::SessionHost::failed) {
        Err("Agent terminal persistence failed".to_owned())
    } else {
        Ok(())
    };
    let drained = clones.drain(&mut node, drain);
    node.shutdown().map_err(|e| e.to_string())?;
    result
        .and(drained)
        .and(plugins_finished)
        .and(revisions_finished)
        .and(agents_finished)
}

/// How a refused request closes the session: what the Controller got wrong is its protocol or
/// identity violation, a stopping Node is shutting down, and everything else is the Node's own
/// failure.
fn close_reason(error: &crate::Error) -> CloseReason {
    match error {
        crate::Error::Validation(_)
        | crate::Error::UnsupportedMessage
        | crate::Error::Storage(
            StorageError::Validation(_) | StorageError::IdentityConflict | StorageError::InvalidAck,
        ) => CloseReason::ProtocolViolation,
        crate::Error::Storage(StorageError::ControllerMismatch) => CloseReason::IdentityMismatch,
        crate::Error::Stopping | crate::Error::Shutdown(_) => CloseReason::Shutdown,
        crate::Error::Configuration(_)
        | crate::Error::Storage(
            StorageError::Io(_)
            | StorageError::Sql(_)
            | StorageError::Encoding(_)
            | StorageError::AlreadyRunning
            | StorageError::InvalidSchema
            | StorageError::NodeMismatch
            | StorageError::ResourceConflict
            | StorageError::InvalidTransition
            | StorageError::Injected(_),
        ) => CloseReason::InternalError,
    }
}

/// Ownership checks precede each read, acknowledgement or new durable admission.
#[allow(clippy::too_many_arguments)]
fn handle(
    node: &mut ManagedNode,
    clones: &mut Clones,
    agents: Option<&agents::SessionHost>,
    deliveries: bool,
    controller: &ControllerId,
    controlled: bool,
    target: Option<&RuntimeScope>,
    request: Request,
) -> Result<Vec<NodeToControllerMessage>, crate::Error> {
    match request {
        Request::Replay(cursors) => Ok(node
            .database
            .controller_events_after(controller, &cursors)?),
        Request::Message(ControllerToNodeMessage::CloneRepository(command)) => {
            if controlled {
                return Err(crate::Error::UnsupportedMessage);
            }
            node.database.check_controller_execution(
                controller,
                &command.operation_id,
                &command.execution_id,
            )?;
            let (status, fresh) = node.reserve_clone(&command)?;
            if fresh.is_some() {
                clones.admit(command.operation_id.clone(), command.execution_id.clone());
            }
            Ok(vec![NodeToControllerMessage::ExecutionStatus(
                ExecutionStatusMessage {
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    operation_id: command.operation_id,
                    execution_id: command.execution_id,
                    payload: status,
                },
            )])
        }
        Request::Message(ControllerToNodeMessage::InstallPlugins(command)) => accept_plugins(
            node,
            controller,
            controlled,
            PluginCommand::Install(command),
        ),
        Request::Message(ControllerToNodeMessage::RemovePlugins(command)) => {
            accept_plugins(node, controller, controlled, PluginCommand::Remove(command))
        }
        Request::Message(ControllerToNodeMessage::ControlledPlugins(envelope)) => {
            envelope.validate()?;
            if envelope.binding.node_incarnation_id != node.identity().incarnation_id.as_str() {
                return Err(StorageError::NodeMismatch.into());
            }
            node.database.check_controller_execution(
                controller,
                envelope.command.operation_id(),
                envelope.command.execution_id(),
            )?;
            let record = node.database.accept_controlled_plugins(&envelope)?;
            Ok(plugin_status(node, record))
        }
        Request::Message(ControllerToNodeMessage::StartAgentSession(input)) => {
            if controlled {
                return Err(crate::Error::UnsupportedMessage);
            }
            agents::start(
                node,
                agents.ok_or(crate::Error::UnsupportedMessage)?,
                controller,
                &input,
                /*permit*/ None,
            )
        }
        Request::Message(ControllerToNodeMessage::ControlledStartAgentSession(envelope)) => {
            envelope.validate()?;
            agents::start(
                node,
                agents.ok_or(crate::Error::UnsupportedMessage)?,
                controller,
                &envelope.command,
                Some(&envelope.binding),
            )
        }
        Request::Message(ControllerToNodeMessage::SubmitUserTurn(input)) => {
            agents.ok_or(crate::Error::UnsupportedMessage)?;
            agents::command(
                node,
                controller,
                ora_node_db::SessionCommandInput::SubmitUserTurn(input),
            )
        }
        Request::Message(ControllerToNodeMessage::EndSession(input)) => {
            agents.ok_or(crate::Error::UnsupportedMessage)?;
            agents::command(
                node,
                controller,
                ora_node_db::SessionCommandInput::EndSession(input),
            )
        }
        Request::Message(ControllerToNodeMessage::BindRuntime(binding)) => {
            if target.is_some_and(|scope| !scope.permits(&binding)) {
                return Err(StorageError::NodeMismatch.into());
            }
            if binding.node_incarnation_id != node.identity().incarnation_id.as_str() {
                return Err(StorageError::NodeMismatch.into());
            }
            let unfinished_execution_ids = node.database.bind_runtime(&binding)?;
            Ok(vec![NodeToControllerMessage::RuntimeControlState(
                RuntimeControlState {
                    binding,
                    unfinished_execution_ids,
                },
            )])
        }
        Request::Message(ControllerToNodeMessage::ControlledClone(envelope)) => {
            envelope.validate()?;
            if envelope.binding.node_incarnation_id != node.identity().incarnation_id.as_str() {
                return Err(StorageError::NodeMismatch.into());
            }
            let command = envelope.command;
            node.database.check_controller_execution(
                controller,
                &command.operation_id,
                &command.execution_id,
            )?;
            let (status, fresh) = node.reserve_controlled_clone(&command, &envelope.binding)?;
            if fresh.is_some() {
                clones.admit(command.operation_id.clone(), command.execution_id.clone());
            }
            Ok(vec![NodeToControllerMessage::ExecutionStatus(
                ExecutionStatusMessage {
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    operation_id: command.operation_id,
                    execution_id: command.execution_id,
                    payload: status,
                },
            )])
        }
        Request::Message(ControllerToNodeMessage::GetExecutionStatus(query)) => {
            node.database.check_controller_execution(
                controller,
                &query.operation_id,
                &query.execution_id,
            )?;
            Ok(vec![NodeToControllerMessage::ExecutionStatus(
                node.status(&query)?,
            )])
        }
        Request::Message(ControllerToNodeMessage::EventAck(ack)) => {
            node.database.check_controller_execution(
                controller,
                &ack.operation_id,
                &ack.execution_id,
            )?;
            node.acknowledge(&ack)?;
            Ok(vec![])
        }
        Request::Message(ControllerToNodeMessage::DeliverRevision(command)) => {
            if controlled || !deliveries {
                return Err(crate::Error::UnsupportedMessage);
            }
            node.database.check_controller_execution(
                controller,
                &command.operation_id,
                &command.execution_id,
            )?;
            let record = node.database.accept_delivery(&command)?;
            Ok(delivery_status(node, record))
        }
        Request::Message(ControllerToNodeMessage::ControlledDeliverRevision(envelope)) => {
            if !deliveries {
                return Err(crate::Error::UnsupportedMessage);
            }
            envelope.validate()?;
            if envelope.binding.node_incarnation_id != node.identity().incarnation_id.as_str() {
                return Err(StorageError::NodeMismatch.into());
            }
            node.database.check_controller_execution(
                controller,
                &envelope.command.operation_id,
                &envelope.command.execution_id,
            )?;
            let record = node.database.accept_controlled_delivery(&envelope)?;
            Ok(delivery_status(node, record))
        }
        // The session read loop consumes Controller heartbeats and transfer grants; they never
        // reach admission. Worktree executions are refused until this service implements them;
        // it does not advertise their capability, so a conforming Controller never sends them.
        Request::Message(
            ControllerToNodeMessage::Hello(_)
            | ControllerToNodeMessage::Heartbeat(_)
            | ControllerToNodeMessage::EnsureWorktree(_)
            | ControllerToNodeMessage::RemoveWorktree(_)
            | ControllerToNodeMessage::UploadGrant(_)
            | ControllerToNodeMessage::DownloadGrant(_),
        ) => Err(crate::Error::UnsupportedMessage),
    }
}

/// Private IPC can accept bare commands; persisted runtime enforcement still refuses bypasses.
fn accept_plugins(
    node: &mut ManagedNode,
    controller: &ControllerId,
    controlled: bool,
    command: PluginCommand,
) -> Result<Vec<NodeToControllerMessage>, crate::Error> {
    if controlled {
        return Err(crate::Error::UnsupportedMessage);
    }
    node.database.check_controller_execution(
        controller,
        command.operation_id(),
        command.execution_id(),
    )?;
    let record = node.database.accept_plugins(&command)?;
    Ok(plugin_status(node, record))
}

/// Admission only reports durable state and never waits for the downloader.
fn plugin_status(
    node: &ManagedNode,
    record: ora_node_db::PluginExecution,
) -> Vec<NodeToControllerMessage> {
    vec![NodeToControllerMessage::ExecutionStatus(
        ExecutionStatusMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: record.command.operation_id().clone(),
            execution_id: record.command.execution_id().clone(),
            payload: ExecutionStatus {
                node: node.identity().clone(),
                state: record.state,
            },
        },
    )]
}

/// Admission reports the durable state; the delivery executor picks the execution up on its own.
fn delivery_status(
    node: &ManagedNode,
    record: ora_node_db::RevisionDelivery,
) -> Vec<NodeToControllerMessage> {
    vec![NodeToControllerMessage::ExecutionStatus(
        ExecutionStatusMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: record.command.operation_id.clone(),
            execution_id: record.command.execution_id.clone(),
            payload: ExecutionStatus {
                node: node.identity().clone(),
                state: record.progress.state(),
            },
        },
    )]
}
