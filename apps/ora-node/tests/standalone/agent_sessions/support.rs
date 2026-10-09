use super::super::*;
use crate::support::{ChildGuard, until};
use std::{
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};
use tokio::net::UnixStream;

pub(super) const EXECUTION: &str = "agent-execution";

/// Supplies immutable input shared by real wire and seeded recovery tests.
pub(super) fn start(text: &str) -> StartAgentSessionMessage {
    StartAgentSessionMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: OperationId::new("agent-operation"),
        execution_id: ExecutionId::new(EXECUTION),
        payload: StartAgentSession {
            spec: AgentSessionSpec {
                node_id: NodeId::new("test-node"),
                agent_plugin_id: PluginId::new("official/ora-space.echo"),
                agent_plugin_version: PluginVersion::new("1.0.0"),
                checkout_execution_id: ExecutionId::new("clone-exec-agent"),
                git_identity: GitIdentity {
                    name: "Test".into(),
                    email: "test@example.com".into(),
                },
                initial_turn: UserTurn {
                    turn_id: TurnId::new("first"),
                    content: vec![ContentBlock::Text { text: text.into() }],
                },
            },
        },
    }
}

/// Writes a deployment config enabling the existing echo fixture as the plugin executable.
pub(super) fn launch(fixture: &Fixture, clone: &CloneConfig) -> ChildGuard {
    let path = ipc::write_config(fixture, clone, /*frame_timeout_ms*/ 40_000);
    let mut config: ora_node::ServiceConfig =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    config.agent = Some(ora_node::AgentConfig {
        deno_path: env!("CARGO_BIN_EXE_ora-node-echo-agent").into(),
        ready_timeout_ms: 5000,
    });
    config.control.as_mut().unwrap().heartbeat_ms = 20;
    fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
    let log = fixture.path().join("agent-service.log");
    let mut child = ChildGuard(
        Command::new(env!("CARGO_BIN_EXE_ora-node"))
            .arg(path)
            .stdin(Stdio::null())
            .stdout(fs::File::create(&log).unwrap())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    until(|| {
        assert!(child.0.try_wait().unwrap().is_none());
        fs::read_to_string(&log)
            .unwrap_or_default()
            .contains("Node IPC listening")
    });
    child
}

/// Installs a deterministic package and obtains its checkout through the real clone executor.
pub(super) fn prepare(fixture: &Fixture, clone: &CloneConfig, server: &HttpsRepository) -> PathBuf {
    let root = fixture
        .config()
        .home_directory
        .join("plugins")
        .join("installed")
        .join("official")
        .join("ora-space.echo")
        .join("1.0.0");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("main.js"), "export {};\n").unwrap();
    fs::write(root.join("orax.toml"), "resolver = 1\nidentifier = \"ora-space.echo\"\nkind = \"agent\"\nversion = \"1.0.0\"\ndescription = \"test\"\n").unwrap();
    let mut node = Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap();
    node.configure_clone(clone.clone()).unwrap();
    assert!(matches!(
        node.submit_clone(request(server, "agent", "main"))
            .unwrap()
            .state,
        ExecutionState::Completed(ExecutionResult::Clone(CloneExecutionResult::CloneReady(_)))
    ));
    drop(node);
    root
}

/// Bounds every expected response independently of heartbeat traffic.
pub(super) async fn receive(stream: &mut UnixStream) -> NodeToControllerMessage {
    tokio::time::timeout(Duration::from_secs(/*secs*/ 15), async {
        loop {
            let message = read_node_message(stream)
                .await
                .unwrap()
                .expect("Node disconnected");
            if !matches!(message, NodeToControllerMessage::Heartbeat(_)) {
                return message;
            }
        }
    })
    .await
    .expect("Node response deadline")
}

/// Completes the production handshake and verifies the advertised session capability.
pub(super) async fn connect(fixture: &Fixture) -> (UnixStream, NodeRuntimeIdentity) {
    let mut stream = ipc::connect(
        &fixture.config().home_directory.join("control.sock"),
        "owner",
    )
    .await;
    let NodeToControllerMessage::HelloAccepted(hello) = receive(&mut stream).await else {
        panic!("expected hello")
    };
    assert!(
        hello
            .payload
            .capabilities
            .contains(&NodeCapability::AgentSession)
    );
    // Delivery runs Git in the checkouts clone created, so clone configuration enables it.
    assert!(
        hello
            .payload
            .capabilities
            .contains(&NodeCapability::RevisionDelivery)
    );
    (stream, hello.payload.node)
}

/// Sends one actual framed Controller command.
pub(super) async fn send(stream: &mut UnixStream, message: ControllerToNodeMessage) {
    write_controller_message(stream, &message).await.unwrap();
}

/// Targets one exact durable sequence.
pub(super) fn ack(sequence: u64) -> ControllerToNodeMessage {
    ControllerToNodeMessage::EventAck(EventAckMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: start("").operation_id,
        execution_id: ExecutionId::new(EXECUTION),
        sequence: Sequence::new(sequence),
        payload: EventAck {
            node_id: NodeId::new("test-node"),
        },
    })
}

/// Status queries are a FIFO barrier after otherwise reply-free ACKs.
pub(super) fn query() -> ControllerToNodeMessage {
    ControllerToNodeMessage::GetExecutionStatus(GetExecutionStatusMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: start("").operation_id,
        execution_id: ExecutionId::new(EXECUTION),
        payload: GetExecutionStatus {
            node_id: NodeId::new("test-node"),
        },
    })
}

/// Produces an independently deduplicated user turn.
pub(super) fn turn() -> ControllerToNodeMessage {
    ControllerToNodeMessage::SubmitUserTurn(SubmitUserTurnMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: start("").operation_id,
        execution_id: ExecutionId::new(EXECUTION),
        payload: SubmitUserTurn {
            node_id: NodeId::new("test-node"),
            command_id: CommandId::new("second-command"),
            turn: UserTurn {
                turn_id: TurnId::new("second"),
                content: vec![ContentBlock::Text {
                    text: "second message".into(),
                }],
            },
        },
    })
}

/// Ends a session through the public command path.
pub(super) fn end() -> ControllerToNodeMessage {
    ControllerToNodeMessage::EndSession(EndSessionMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: start("").operation_id,
        execution_id: ExecutionId::new(EXECUTION),
        payload: EndSession {
            node_id: NodeId::new("test-node"),
            command_id: CommandId::new("end-command"),
            reason: EndSessionReason::UserEnded,
        },
    })
}

/// Verifies both the durable history and the absence of a second plugin launch after recovery.
pub(super) fn history(fixture: &Fixture) -> Vec<serde_json::Value> {
    let path =
        ora_history::history_path(&fixture.config().home_directory.join("sessions"), EXECUTION)
            .unwrap();
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Pins the launched plugin before checking its eventual exit, avoiding PID reuse races.
pub(super) fn plugin(root: &Path) -> ora_utils::process::LinuxPidFd {
    let pid = fs::read_to_string(root.join("echo-agent.pids"))
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    ora_utils::process::LinuxPidFd::from_observation(
        &ora_utils::process::linux_process(pid).unwrap(),
    )
    .unwrap()
}
