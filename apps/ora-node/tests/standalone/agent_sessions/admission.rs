//! Shared-checkout admission through the real executable, framed IPC and echo Agent processes.
use super::*;
use pretty_assertions::assert_eq;
use std::collections::HashSet;

/// Gives each execution a distinct identity while retaining the same clone checkout.
pub(super) fn named(id: &str, text: &str) -> StartAgentSessionMessage {
    let mut input = start(text);
    input.operation_id = OperationId::new(format!("operation-{id}"));
    input.execution_id = ExecutionId::new(format!("execution-{id}"));
    input.payload.spec.initial_turn.turn_id = TurnId::new(format!("initial-{id}"));
    input
}

/// Queues a turn for one exact session, independently of every sibling execution.
fn named_turn(id: &str, serial: u32) -> ControllerToNodeMessage {
    let input = named(id, "after sibling admission");
    ControllerToNodeMessage::SubmitUserTurn(SubmitUserTurnMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: input.operation_id,
        execution_id: input.execution_id,
        payload: SubmitUserTurn {
            node_id: NodeId::new("test-node"),
            command_id: CommandId::new(format!("turn-command-{id}-{serial}")),
            turn: UserTurn {
                turn_id: TurnId::new(format!("turn-{id}-{serial}")),
                content: input.payload.spec.initial_turn.content,
            },
        },
    })
}

/// Ends one exact session without naming or affecting a sibling.
fn named_end(input: &StartAgentSessionMessage) -> ControllerToNodeMessage {
    ControllerToNodeMessage::EndSession(EndSessionMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: input.operation_id.clone(),
        execution_id: input.execution_id.clone(),
        payload: EndSession {
            node_id: NodeId::new("test-node"),
            command_id: CommandId::new(format!("end-command-{}", input.execution_id.as_str())),
            reason: EndSessionReason::UserEnded,
        },
    })
}

/// Waits for a batch so events arriving out of order are not accidentally discarded.
async fn wait_turns(stream: &mut tokio::net::UnixStream, inputs: &[StartAgentSessionMessage]) {
    let mut pending: HashSet<(ExecutionId, TurnId)> = inputs
        .iter()
        .map(|input| {
            (
                input.execution_id.clone(),
                input.payload.spec.initial_turn.turn_id.clone(),
            )
        })
        .collect();
    while !pending.is_empty() {
        match receive(stream).await {
            NodeToControllerMessage::ThreadEvent(event) => {
                if event
                    .payload
                    .record
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    == Some("turnEnded")
                {
                    pending.remove(&(
                        event.execution_id,
                        event
                            .payload
                            .turn_id
                            .unwrap_or_else(|| TurnId::new("missing")),
                    ));
                }
            }
            NodeToControllerMessage::ExecutionStatus(_)
            | NodeToControllerMessage::SessionCommandAccepted(_) => {}
            other => panic!("unexpected while waiting for turns: {other:?}"),
        }
    }
}

/// Starts a batch and proves every initial turn ran on its own live session.
async fn start_batch(stream: &mut tokio::net::UnixStream, inputs: &[StartAgentSessionMessage]) {
    for input in inputs {
        send(
            stream,
            ControllerToNodeMessage::StartAgentSession(input.clone()),
        )
        .await;
    }
    wait_turns(stream, inputs).await;
}

/// Runs one subsequent turn to prove the same connection and session still answer commands.
async fn run_turn(stream: &mut tokio::net::UnixStream, id: &str, serial: u32) {
    send(stream, named_turn(id, serial)).await;
    let input = named(id, "");
    let turn = TurnId::new(format!("turn-{id}-{serial}"));
    loop {
        match receive(stream).await {
            NodeToControllerMessage::ThreadEvent(event)
                if event.execution_id == input.execution_id
                    && event.payload.turn_id == Some(turn.clone())
                    && event
                        .payload
                        .record
                        .get("type")
                        .and_then(serde_json::Value::as_str)
                        == Some("turnEnded") =>
            {
                break;
            }
            NodeToControllerMessage::ThreadEvent(_)
            | NodeToControllerMessage::SessionCommandAccepted(_) => {}
            other => panic!("unexpected while running a sibling turn: {other:?}"),
        }
    }
}

/// Extracts the Agent terminal detail from either a status result or terminal event payload.
fn detail(result: &AgentSessionResult) -> Option<&str> {
    let AgentSessionResult::AgentSessionEnded(ended) = result;
    ended.detail.as_deref()
}

/// A refused start has one immediate status and one durable sequence-1 terminal, with no actor.
async fn assert_refusal(
    stream: &mut tokio::net::UnixStream,
    input: &StartAgentSessionMessage,
    expected: &str,
) {
    send(
        stream,
        ControllerToNodeMessage::StartAgentSession(input.clone()),
    )
    .await;
    let (mut status, mut terminal) = (false, false);
    while !status || !terminal {
        match receive(stream).await {
            NodeToControllerMessage::ExecutionStatus(reply)
                if reply.execution_id == input.execution_id =>
            {
                let ExecutionState::Completed(ExecutionResult::AgentSession(result)) =
                    reply.payload.state
                else {
                    panic!("refused start was not completed: {reply:?}")
                };
                assert_eq!(detail(&result), Some(expected));
                status = true;
            }
            NodeToControllerMessage::AgentSessionEnded(event)
                if event.execution_id == input.execution_id =>
            {
                assert_eq!(event.sequence, Sequence::new(/*value*/ 1));
                assert_eq!(detail(&event.payload), Some(expected));
                terminal = true;
            }
            other => panic!("unexpected during refused start: {other:?}"),
        }
    }
}

/// Ends one session and returns its terminal for reason assertions.
async fn end_session(
    stream: &mut tokio::net::UnixStream,
    input: &StartAgentSessionMessage,
) -> AgentSessionEnded {
    send(stream, named_end(input)).await;
    loop {
        match receive(stream).await {
            NodeToControllerMessage::AgentSessionEnded(event)
                if event.execution_id == input.execution_id =>
            {
                let AgentSessionResult::AgentSessionEnded(ended) = event.payload;
                return ended;
            }
            NodeToControllerMessage::ThreadEvent(_)
            | NodeToControllerMessage::SessionCommandAccepted(_) => {}
            other => panic!("unexpected while ending a session: {other:?}"),
        }
    }
}

/// Pins every plugin the fixture journal names. Callers cannot map a line to a session — the
/// journal's order follows launch completion, which is racy under load — so they assert over
/// the whole set instead of one index.
fn pinned_plugins(root: &std::path::Path) -> Vec<ora_utils::process::LinuxPidFd> {
    fs::read_to_string(root.join("echo-agent.pids"))
        .unwrap()
        .lines()
        .map(|line| line.parse().unwrap())
        .map(|pid| {
            ora_utils::process::LinuxPidFd::from_observation(
                &ora_utils::process::linux_process(pid).unwrap(),
            )
            .unwrap()
        })
        .collect()
}

/// Two sessions share a checkout, and ending the first changes neither the second nor the tree.
#[test]
fn two_live_sessions_run_turns_and_end_independently() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let root = prepare(&fixture, &config, &server);
        let clone = checkout(&fixture);
        let checkout = PathBuf::from(clone.path.as_str());
        let head = checkout_git(&checkout, &["rev-parse", "HEAD"]);
        let mut child = launch(&fixture, &config);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (mut stream, _) = connect(&fixture).await;
                let inputs = [named("a", "hello A"), named("b", "hello B")];
                start_batch(&mut stream, &inputs).await;
                let plugins = pinned_plugins(&root);
                assert_eq!(plugins.len(), 2);

                let first_end = end_session(&mut stream, &inputs[0]).await;
                assert_eq!(first_end.reason, AgentSessionEndReason::UserEnded);
                // Ending A stops exactly A's plugin: B's is still running.
                assert_eq!(
                    plugins
                        .iter()
                        .filter(|plugin| !plugin.has_exited().unwrap())
                        .count(),
                    1
                );
                assert_eq!(checkout_git(&checkout, &["rev-parse", "HEAD"]), head);

                run_turn(&mut stream, "b", 1).await;
                let second_end = end_session(&mut stream, &inputs[1]).await;
                assert_eq!(second_end.reason, AgentSessionEndReason::UserEnded);
            });
        child.terminate();
    });
}

/// Both refusal classes stay on their execution while the connection and siblings keep working.
#[test]
fn checkout_refusals_keep_the_connection_and_sibling_sessions_alive() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let root = prepare(&fixture, &config, &server);
        let clone = checkout(&fixture);
        let head = checkout_git(
            PathBuf::from(clone.path.as_str()).as_path(),
            &["rev-parse", "HEAD"],
        );
        let mut child = launch(&fixture, &config);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (mut stream, _) = connect(&fixture).await;
                let first = [named("a", "hello A"), named("b", "hello B")];
                start_batch(&mut stream, &first).await;

                let mut restore = named("restore", "must not restore");
                restore.payload.spec.prior_revision = Some(PriorRevision {
                    revision_id: RevisionId::new("restore-revision"),
                    final_commit: CommitId::new("0123456789012345678901234567890123456789"),
                    git_repository_url: server.url.clone(),
                    delivery_ref: DeliveryRef::parse("refs/ora/deliveries/restore").unwrap(),
                    base_commit: clone.commit.clone(),
                });
                assert_refusal(&mut stream, &restore, "checkout_busy").await;
                // Nothing restored and no agent started, so the tree is exactly where it was.
                assert_eq!(
                    checkout_git(
                        PathBuf::from(clone.path.as_str()).as_path(),
                        &["rev-parse", "HEAD"]
                    ),
                    head
                );
                run_turn(&mut stream, "a", 1).await;

                let additional: Vec<_> = (0..6)
                    .map(|index| named(&format!("extra-{index}"), "parallel"))
                    .collect();
                start_batch(&mut stream, &additional).await;
                // The refused start selects an MCP server with a runtime Setting value.
                let mut ninth = named("ninth", "must not start");
                ninth.payload.spec.mcp = vec![SessionMcpBinding {
                    plugin_id: PluginId::new("official/acme.mcp"),
                    plugin_version: PluginVersion::new("1.0.0"),
                    settings: std::collections::BTreeMap::from([(
                        "token".into(),
                        SettingValue::String(REFUSED_SECRET.into()),
                    )]),
                }];
                assert_refusal(&mut stream, &ninth, "checkout_session_limit").await;
                // A retransmission with another value reports the same durable terminal on the
                // same connection instead of an identity conflict that would close it.
                ninth.payload.spec.mcp[0].settings.insert(
                    "token".into(),
                    SettingValue::String(REFUSED_OTHER_SECRET.into()),
                );
                send(
                    &mut stream,
                    ControllerToNodeMessage::StartAgentSession(ninth.clone()),
                )
                .await;
                match receive(&mut stream).await {
                    NodeToControllerMessage::ExecutionStatus(reply)
                        if reply.execution_id == ninth.execution_id =>
                    {
                        let ExecutionState::Completed(ExecutionResult::AgentSession(result)) =
                            reply.payload.state
                        else {
                            panic!("retransmitted refusal was not completed: {reply:?}")
                        };
                        assert_eq!(detail(&result), Some("checkout_session_limit"));
                    }
                    other => panic!("unexpected after a retransmitted refusal: {other:?}"),
                }
                run_turn(&mut stream, "a", 2).await;
                assert_eq!(
                    fs::read_to_string(root.join("echo-agent.pids"))
                        .unwrap()
                        .lines()
                        .count(),
                    8
                );

                for input in first.iter().chain(&additional) {
                    assert_eq!(
                        end_session(&mut stream, input).await.reason,
                        AgentSessionEndReason::UserEnded
                    );
                }
            });
        child.terminate();
        assert_no_secret(fixture.path());
    });
}

/// Runtime-only Setting values a refused start carries; they must reach no file.
const REFUSED_SECRET: &str = "r1187-refused-mcp-secret";
const REFUSED_OTHER_SECRET: &str = "r1187-refused-mcp-other-secret";

/// Scans every file the Node wrote (ledger, WAL, logs, histories) for the refused Settings.
fn assert_no_secret(path: &std::path::Path) {
    for entry in fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        let metadata = fs::symlink_metadata(&path).unwrap();
        if metadata.is_dir() {
            assert_no_secret(&path);
        } else if metadata.is_file() {
            let bytes = fs::read(&path).unwrap();
            for secret in [REFUSED_SECRET, REFUSED_OTHER_SECRET] {
                assert!(
                    !bytes
                        .windows(secret.len())
                        .any(|window| window == secret.as_bytes()),
                    "refused Setting value persisted at {}",
                    path.display()
                );
            }
        }
    }
}
