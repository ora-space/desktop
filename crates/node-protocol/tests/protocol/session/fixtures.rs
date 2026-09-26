use super::*;

/// Pairs the hello typed fixture with an independent wire contract.
pub(super) fn hello() -> Case {
    Case {
        message: Message::Controller(ControllerToNodeMessage::Hello(HelloMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            payload: Hello {
                controller_id: ControllerId::new("controller-1"),
                supported_versions: vec![CURRENT_PROTOCOL_VERSION],
            },
        })),
        wire: json!({
            "message_type": "hello",
            "protocol_version": 1,
            "payload": {
                "controller_id": "controller-1",
                "supported_versions": [
                    1
                ]
            }
        }),
    }
}

/// Pairs the hello_accepted typed fixture with an independent wire contract.
pub(super) fn hello_accepted() -> Case {
    Case {
        message: Message::Node(NodeToControllerMessage::HelloAccepted(
            HelloAcceptedMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                payload: HelloAccepted {
                    selected_version: CURRENT_PROTOCOL_VERSION,
                    node: node(),
                    capabilities: vec![NodeCapability::WorktreeExecution],
                },
            },
        )),
        wire: json!({
            "message_type": "hello_accepted",
            "protocol_version": 1,
            "payload": {
                "selected_version": 1,
                "node": {
                    "node_id": "node-1",
                    "incarnation_id": "incarnation-1"
                },
                "capabilities": [
                    "worktree_execution"
                ]
            }
        }),
    }
}

/// Pairs the heartbeat typed fixture with an independent wire contract.
pub(super) fn heartbeat() -> Case {
    Case {
        message: Message::Node(NodeToControllerMessage::Heartbeat(HeartbeatMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            payload: Heartbeat { node: node() },
        })),
        wire: json!({
            "message_type": "heartbeat",
            "protocol_version": 1,
            "payload": {
                "node": {
                    "node_id": "node-1",
                    "incarnation_id": "incarnation-1"
                }
            }
        }),
    }
}

/// Pairs the Controller heartbeat typed fixture with an independent wire contract.
pub(super) fn controller_heartbeat() -> Case {
    Case {
        message: Message::Controller(ControllerToNodeMessage::Heartbeat(
            ControllerHeartbeatMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                payload: ControllerHeartbeat {
                    controller_id: ControllerId::new("controller-1"),
                },
            },
        )),
        wire: json!({
            "message_type": "heartbeat",
            "protocol_version": 1,
            "payload": {
                "controller_id": "controller-1"
            }
        }),
    }
}

/// Lists this business slice’s legal wire shapes.
pub(super) fn cases() -> Vec<Case> {
    vec![
        hello(),
        hello_accepted(),
        heartbeat(),
        controller_heartbeat(),
    ]
}
