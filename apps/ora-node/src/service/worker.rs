use super::*;
use crate::{ManagedNode, Node};
use std::time::Instant;

/// Owns the only mutable Node; admission/revocation locks cover SQLite work, never blocking Git.
pub(super) fn run(
    config: ServiceConfig,
    receiver: mpsc::Receiver<Work>,
    ready: oneshot::Sender<Result<SessionInfo, String>>,
    shutdown: Shutdown,
) -> Result<(), String> {
    let initialized = (|| {
        let mut node =
            Node::open(config.node, config.process, shutdown.clone()).map_err(|e| e.to_string())?;
        if let Some(clone) = config.clone {
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
        Ok::<_, String>((node, controller))
    })();
    let (mut node, controller) = match initialized {
        Ok(value) => value,
        Err(error) => {
            let _ = ready.send(Err(error.clone()));
            return Err(error);
        }
    };
    let info = SessionInfo {
        identity: node.identity().clone(),
        controller: controller.clone(),
        capabilities: vec![NodeCapability::RepositoryClone],
    };
    if ready.send(Ok(info)).is_err() {
        return node.shutdown().map_err(|e| e.to_string());
    }
    ora_logging::ora_info!(node_id = %node.node_id().as_str(), "Node opened");
    let mut next = Instant::now();
    let mut previous = None;
    let result = (|| {
        while !shutdown.requested() {
            match receiver.recv_timeout(Duration::from_millis(/*millis*/ 25)) {
                Ok(work) => {
                    let active = work
                        .active
                        .lock()
                        .map_err(|_| "session admission poisoned".to_owned())?;
                    let result = if *active {
                        handle(&mut node, &controller, work.request).map_err(|e| e.to_string())
                    } else {
                        Err("session revoked".into())
                    };
                    let _ = work.reply.send(result);
                    drop(active);
                }
                Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => {}
            }
            if Instant::now() >= next {
                node.recover().map_err(|e| e.to_string())?;
                let state = node.recover_clones().map_err(|e| e.to_string())?;
                if previous != Some(state) {
                    ora_logging::ora_info!(state = ?state, "Node recovery pass completed");
                    previous = Some(state);
                }
                next = Instant::now() + Duration::from_millis(config.recovery_interval_ms);
            }
        }
        Ok(())
    })();
    node.shutdown().map_err(|e| e.to_string())?;
    result
}

/// Ownership checks precede each read, acknowledgement or new durable admission.
fn handle(
    node: &mut ManagedNode,
    controller: &ControllerId,
    request: Request,
) -> Result<Vec<NodeToControllerMessage>, crate::Error> {
    match request {
        Request::Replay => Ok(node.database.controller_events(controller)?),
        Request::Message(ControllerToNodeMessage::CloneRepository(command)) => {
            node.database.check_controller_execution(
                controller,
                &command.operation_id,
                &command.execution_id,
            )?;
            let (status, _) = node.reserve_clone(&command)?;
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
        // The session read loop consumes Controller heartbeats; they never reach admission.
        Request::Message(
            ControllerToNodeMessage::Hello(_)
            | ControllerToNodeMessage::Heartbeat(_)
            | ControllerToNodeMessage::EnsureWorktree(_)
            | ControllerToNodeMessage::RemoveWorktree(_),
        ) => Err(crate::Error::Configuration(
            "message is not supported in this session".into(),
        )),
    }
}
