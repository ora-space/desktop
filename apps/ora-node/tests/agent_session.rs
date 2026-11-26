#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Session executions against the echo agent plugin, driven through the Node's `SessionHost`
//! with an in-memory ledger in place of the Node database.

#[path = "agent_session/cancel.rs"]
mod cancel;
#[path = "agent_session/host_mcp.rs"]
mod host_mcp;
#[path = "agent_session/mcp.rs"]
mod mcp;
#[path = "agent_session/resume.rs"]
mod resume;
#[path = "agent_session/support.rs"]
mod support;
#[path = "agent_session/turn_failure.rs"]
mod turn_failure;
#[path = "agent_session/turn_processes.rs"]
mod turn_processes;
#[path = "agent_session/workload.rs"]
mod workload;

use ora_node::{CommandSettlement, SessionCommand, SessionHost};
use ora_node_protocol::{AgentSessionEndReason, AgentSessionEnded, EndSessionReason, TurnId};
use pretty_assertions::assert_eq;
use serde_json::Value;
use support::*;

/// A committed end command wins over a racing ACP error even before its wake-up arrives.
#[test]
fn durable_cancel_wins_a_racing_provider_failure() {
    with_scoped_async_test(async {
        let fixture = Fixture::new();
        let sessions = fixture.sessions(PLUGIN_VERSION);
        let (reached, release) = fixture.ledger.pause_append(2);
        let ledger = fixture.ledger.clone();
        fixture.start(&sessions, PLUGIN_VERSION, "[request-failed]");
        tokio::task::spawn_blocking(move || {
            ora_logging::with_trace_logging(|| {
                reached
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .expect("provider failure must reach TurnEnded");
                // The synchronous append blocks this test's runtime. Commit and release from
                // its joined blocking task, deliberately without command_arrived's wake-up.
                ledger.accept(
                    "cancel",
                    SessionCommand::EndSession(EndSessionReason::Cancelled),
                );
                release.send(()).unwrap();
            });
        })
        .await
        .unwrap();
        assert_eq!(
            fixture.ended().await.reason,
            AgentSessionEndReason::Cancelled
        );
        assert_eq!(
            fixture.ledger.settlements(),
            vec![("cancel".into(), vec![CommandSettlement::Executed])]
        );
        sessions.shutdown().await;
    });
}

/// ACP request failure must terminate the Node session instead of silently admitting more turns.
#[test]
fn provider_failure_ends_the_session_with_a_safe_code_and_releases_resources() {
    with_scoped_async_test(async {
        let fixture = Fixture::new();
        let sessions = fixture.sessions(PLUGIN_VERSION);
        fixture.start(&sessions, PLUGIN_VERSION, "[request-failed]");
        let ended = tokio::time::timeout(std::time::Duration::from_secs(5), fixture.ended())
            .await
            .expect("a failed prompt must not leave the session idle");
        assert_eq!(ended.reason, AgentSessionEndReason::AgentFailed);
        assert_eq!(ended.detail.as_deref(), Some("agent_turn_failed"));
        // The failure keeps the Agent's sanitized text, but the terminal detail stays a code.
        let history = fixture.history_lines();
        let failure = history
            .iter()
            .find(|line| line.get("type").and_then(Value::as_str) == Some("turnEnded"))
            .and_then(|line| line.get("failure"))
            .expect("the failed turn records its reason");
        assert_eq!(failure.get("code"), Some(&serde_json::json!("agent_error")));
        assert_eq!(
            failure.get("message"),
            Some(&serde_json::json!("upstream diagnostic must not escape"))
        );
        assert_eq!(fixture.leases.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(
            fixture
                .plugin_pids()
                .into_iter()
                .all(|pid| !process_exists(pid))
        );
        sessions.shutdown().await;
    });
}

/// The known ACP login refusal becomes one bounded authentication code, never raw provider text.
#[test]
fn missing_agent_credentials_end_the_session_with_an_actionable_code() {
    with_scoped_async_test(async {
        let fixture = Fixture::new();
        let sessions = fixture.sessions(PLUGIN_VERSION);
        fixture.start(&sessions, PLUGIN_VERSION, "[authentication-required]");
        let ended = tokio::time::timeout(std::time::Duration::from_secs(5), fixture.ended())
            .await
            .expect("an unauthenticated prompt must end the session");
        assert_eq!(ended.reason, AgentSessionEndReason::AgentFailed);
        assert_eq!(
            ended.detail.as_deref(),
            Some("agent_authentication_required")
        );
        // The failure code carries the actionable reason, and the sanitized text stays in the
        // failure record rather than in the terminal detail.
        let history = fixture.history_lines();
        let failure = history
            .iter()
            .find(|line| line.get("type").and_then(Value::as_str) == Some("turnEnded"))
            .and_then(|line| line.get("failure"))
            .expect("the failed turn records its reason");
        assert_eq!(
            failure.get("code"),
            Some(&serde_json::json!("agent_authentication_required"))
        );
        assert_eq!(
            failure.get("message"),
            Some(&serde_json::json!("Authentication required"))
        );
        sessions.shutdown().await;
    });
}

fn with_scoped_async_test(action: impl std::future::Future<Output = ()>) {
    ora_logging::with_trace_logging(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(action);
    });
}

/// A model-bound input never falls back to Echo or an inherited direct API connection.
#[tokio::test]
async fn unconfigured_model_proxy_ends_before_launching_the_plugin() {
    use ora_node_protocol::{
        AgentSessionSpec, ExecutionId, ModelBindingId, OperationId, PluginId, PluginVersion,
    };
    let fixture = Fixture::new();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    sessions.start(
        OperationId::new(OPERATION),
        ExecutionId::new(EXECUTION),
        AgentSessionSpec {
            host_mcp_servers: Vec::new(),
            mcp_lifecycle: false,
            keep_session_on_cancel: false,
            keep_session_on_failure: false,
            mcp: Vec::new(),
            agent_configuration: None,
            node_id: fixture.node().node_id,
            agent_plugin_id: PluginId::new(PLUGIN_ID),
            agent_plugin_version: PluginVersion::new(PLUGIN_VERSION),
            checkout_execution_id: ExecutionId::new(CHECKOUT_EXECUTION),
            model_binding_id: Some(ModelBindingId::new("binding-1")),
            prior_revision: None,
            workspace_directory: vec![],
            git_identity: identity(),
            initial_turn: turn("turn-1", "read the repository"),
            trigger: None,
        },
    );
    assert_eq!(
        fixture.ended().await,
        AgentSessionEnded {
            node: fixture.node(),
            reason: AgentSessionEndReason::AgentFailed,
            detail: Some("model_proxy_unavailable".into()),
            plugin_id: None,
            setting_id: None,
            transport: None,
            scheduled_result: None,
            artifact_manifest: None,
        }
    );
    assert_eq!(
        (fixture.ledger.events(), fixture.plugin_pids()),
        (Vec::new(), Vec::new())
    );
    sessions.shutdown().await;
}

/// Every Thread event is a line of the session history, in file order, attributed to the turn
/// it belongs to; the user message itself carries its turn identity in the history.
///
/// Evidence for specs/test-cases/node/agent-runtime/session-execution.md#session-records-reach-jsonl-before-the-thread
#[tokio::test(flavor = "multi_thread")]
async fn records_reach_the_thread_as_history_lines_attributed_to_their_turns() {
    let fixture = Fixture::new();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start(&sessions, PLUGIN_VERSION, "hello");
    until(|| {
        fixture
            .ledger
            .events()
            .iter()
            .any(|event| field(&event.record, &["type"]) == "turnEnded")
            .then_some(())
    })
    .await;
    fixture.command(
        &sessions,
        "command-2",
        SessionCommand::SubmitUserTurn(turn("turn-2", "again")),
    );
    until(|| {
        (fixture
            .ledger
            .events()
            .iter()
            .filter(|event| field(&event.record, &["type"]) == "turnEnded")
            .count()
            == 2)
            .then_some(())
    })
    .await;
    fixture.command(
        &sessions,
        "command-3",
        SessionCommand::EndSession(EndSessionReason::UserEnded),
    );
    let ended = fixture.ended().await;

    let events = fixture.ledger.events();
    let attribution = events
        .iter()
        .map(|event| {
            (
                event.turn_id.clone(),
                field(&event.record, &["type"]),
                field(&event.record, &["update", "sessionUpdate"]),
                event
                    .record
                    .get("update")
                    .and_then(|update| update.get("messageId"))
                    .cloned()
                    .unwrap_or(Value::Null),
            )
        })
        .collect::<Vec<_>>();
    let turn = |id: &str| Some(TurnId::new(id));
    let entry = |turn_id: Option<TurnId>, kind: &str, update: &str, message_id: Value| {
        (turn_id, kind.to_string(), update.to_string(), message_id)
    };
    assert_eq!(
        (
            events
                .iter()
                .map(|event| event.record.clone())
                .collect::<Vec<_>>(),
            events.iter().any(|event| event.truncated),
        ),
        (fixture.history_lines(), false),
    );
    assert_eq!(
        attribution,
        vec![
            entry(None, "meta", "", Value::Null),
            entry(
                turn("turn-1"),
                "update",
                "user_message_chunk",
                "turn-1".into()
            ),
            entry(turn("turn-1"), "update", "agent_message_chunk", Value::Null),
            entry(turn("turn-1"), "turnEnded", "", Value::Null),
            entry(
                turn("turn-2"),
                "update",
                "user_message_chunk",
                "turn-2".into()
            ),
            entry(turn("turn-2"), "update", "agent_message_chunk", Value::Null),
            entry(turn("turn-2"), "turnEnded", "", Value::Null),
        ],
    );
    assert_eq!(
        (
            ended,
            fixture.ledger.settlements(),
            fixture.leases.load(std::sync::atomic::Ordering::SeqCst),
            fixture
                .plugin_pids()
                .into_iter()
                .filter(|pid| process_exists(*pid))
                .count(),
        ),
        (
            AgentSessionEnded {
                node: fixture.node(),
                reason: AgentSessionEndReason::UserEnded,
                detail: None,
                plugin_id: None,
                setting_id: None,
                transport: None,
                scheduled_result: None,
                artifact_manifest: None,
            },
            vec![
                ("command-2".to_string(), vec![CommandSettlement::Executed]),
                ("command-3".to_string(), vec![CommandSettlement::Executed]),
            ],
            0,
            0,
        ),
    );
}

/// User turns accepted while the initial turn runs wait for it, then run once each in acceptance
/// order, however many times the session is woken.
///
/// Evidence for specs/test-cases/node/agent-runtime/session-execution.md#session-commands-run-once-in-acceptance-order-and-an-end-discards-the-queue
#[tokio::test(flavor = "multi_thread")]
async fn queued_turns_run_once_each_in_acceptance_order() {
    let fixture = Fixture::new();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start(&sessions, PLUGIN_VERSION, "first");
    fixture.command(
        &sessions,
        "command-2",
        SessionCommand::SubmitUserTurn(turn("turn-2", "second")),
    );
    fixture.command(
        &sessions,
        "command-3",
        SessionCommand::SubmitUserTurn(turn("turn-3", "third")),
    );
    for _ in 0..3 {
        sessions.command_arrived(&ora_node_protocol::ExecutionId::new(EXECUTION));
    }
    until(|| (turn_ends(&fixture) == 3).then_some(())).await;
    fixture.command(
        &sessions,
        "command-4",
        SessionCommand::EndSession(EndSessionReason::UserEnded),
    );
    fixture.ended().await;

    assert_eq!(
        (user_messages(&fixture), fixture.ledger.settlements()),
        (
            vec![
                ("turn-1".to_string(), "first".to_string()),
                ("turn-2".to_string(), "second".to_string()),
                ("turn-3".to_string(), "third".to_string()),
            ],
            vec![
                ("command-2".to_string(), vec![CommandSettlement::Executed]),
                ("command-3".to_string(), vec![CommandSettlement::Executed]),
                ("command-4".to_string(), vec![CommandSettlement::Executed]),
            ],
        ),
    );
}

/// An end requested while a turn runs cancels that turn, discards the turns queued ahead of it,
/// and leaves no plugin process or lease behind.
///
/// Evidence for specs/test-cases/node/agent-runtime/session-execution.md#session-commands-run-once-in-acceptance-order-and-an-end-discards-the-queue
#[tokio::test(flavor = "multi_thread")]
async fn ending_cancels_the_running_turn_and_discards_queued_turns() {
    let fixture = Fixture::new();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start(&sessions, PLUGIN_VERSION, "[hold]");
    // The recorded user message is the turn's first settled line, so the turn is running.
    until(|| (!user_messages(&fixture).is_empty()).then_some(())).await;
    fixture.command(
        &sessions,
        "command-2",
        SessionCommand::SubmitUserTurn(turn("turn-2", "never")),
    );
    fixture.command(
        &sessions,
        "command-3",
        SessionCommand::EndSession(EndSessionReason::UserEnded),
    );
    let ended = fixture.ended().await;

    let last = fixture
        .ledger
        .events()
        .pop()
        .expect("the cancelled turn ended");
    assert_eq!(
        (
            ended.reason,
            user_messages(&fixture),
            (
                last.turn_id,
                field(&last.record, &["type"]),
                field(&last.record, &["stop_reason"]),
                field(&last.record, &["ended_by"])
            ),
            fixture.ledger.settlements(),
            fixture.leases.load(std::sync::atomic::Ordering::SeqCst),
            fixture
                .plugin_pids()
                .into_iter()
                .filter(|pid| process_exists(*pid))
                .count(),
        ),
        (
            AgentSessionEndReason::UserEnded,
            vec![("turn-1".to_string(), "[hold]".to_string())],
            (
                Some(TurnId::new("turn-1")),
                "turnEnded".to_string(),
                "cancelled".to_string(),
                "session_end".to_string(),
            ),
            vec![
                ("command-2".to_string(), vec![CommandSettlement::Discarded]),
                ("command-3".to_string(), vec![CommandSettlement::Executed]),
            ],
            0,
            0,
        ),
    );
}

/// A crash between the history append and the Thread append leaves the history exactly one line
/// ahead; the restarted Node ends the session as interrupted and delivers the history.
///
/// Evidence for specs/test-cases/node/agent-runtime/session-execution.md#session-records-reach-jsonl-before-the-thread
#[tokio::test(flavor = "multi_thread")]
async fn a_crash_before_the_thread_append_leaves_the_history_one_line_ahead() {
    let fixture = Fixture::new();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    // The third line is the agent's answer to the first turn.
    let (reached, release) = fixture.ledger.gate_append(2);
    fixture.start(&sessions, PLUGIN_VERSION, "hello");
    until(|| reached.try_recv().ok()).await;

    let history = fixture.history_lines();
    let thread = fixture
        .ledger
        .events()
        .into_iter()
        .map(|event| event.record)
        .collect::<Vec<_>>();
    let restarted = fixture.sessions(PLUGIN_VERSION);
    let execution = ora_node_protocol::ExecutionId::new(EXECUTION);
    let recovered = restarted.recover_interrupted(&execution);
    let delivered = restarted.sealed_history(&execution);
    assert_eq!(
        (
            history.len(),
            history[..thread.len()].to_vec(),
            recovered,
            delivered,
        ),
        (
            thread.len() + 1,
            thread,
            AgentSessionEnded {
                node: fixture.node(),
                reason: AgentSessionEndReason::Interrupted,
                detail: None,
                plugin_id: None,
                setting_id: None,
                transport: None,
                scheduled_result: None,
                artifact_manifest: None,
            },
            Ok(ora_history::history_path(&fixture.home().join("sessions"), EXECUTION).unwrap()),
        ),
    );

    // The original process learns its ledger is gone and stops without writing more Thread events.
    release.send(()).unwrap();
    let ended = fixture.ended().await;
    assert_eq!(
        (ended.reason, ended.detail, fixture.ledger.events().len()),
        (
            AgentSessionEndReason::AgentFailed,
            Some("thread_unavailable".to_string()),
            2
        ),
    );
}

/// A plugin that is not installed at the requested version ends the session before any plugin
/// process exists.
///
/// Evidence for specs/test-cases/node/agent-runtime/session-execution.md#sessions-start-only-their-plugin-with-identity-confined-to-its-process-tree
#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_of_another_version_ends_the_session_without_starting_it() {
    let fixture = Fixture::new();
    // The catalog names a version whose directory the Node's plugin root does not hold.
    let sessions = fixture.sessions("2.0.0");
    fixture.start(&sessions, "2.0.0", "hello");
    let mismatched = fixture.ended().await;

    let fixture = Fixture::new();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start(&sessions, "2.0.0", "hello");
    let missing = fixture.ended().await;

    let failed = AgentSessionEnded {
        node: fixture.node(),
        reason: AgentSessionEndReason::AgentFailed,
        detail: Some("agent_plugin_unavailable".to_string()),
        plugin_id: None,
        setting_id: None,
        transport: None,
        scheduled_result: None,
        artifact_manifest: None,
    };
    assert_eq!(
        (
            mismatched,
            missing,
            fixture.plugin_pids(),
            fixture.ledger.events(),
            fixture.leases.load(std::sync::atomic::Ordering::SeqCst),
        ),
        (failed.clone(), failed, Vec::new(), Vec::new(), 0),
    );
}

/// Commits made by the agent's process tree, whether spawned by the host or by the plugin itself,
/// carry the session's identity, and the checkout's Git configuration is untouched.
///
/// Evidence for specs/test-cases/node/agent-runtime/session-execution.md#sessions-start-only-their-plugin-with-identity-confined-to-its-process-tree
#[tokio::test(flavor = "multi_thread")]
async fn agent_commits_carry_the_session_identity_without_touching_git_config() {
    let fixture = Fixture::new();
    let config = std::fs::read(fixture.checkout().join(".git").join("config")).unwrap();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start(&sessions, PLUGIN_VERSION, "[commit-host]");
    fixture.command(
        &sessions,
        "command-2",
        SessionCommand::SubmitUserTurn(turn("turn-2", "[commit-direct]")),
    );
    until(|| (turn_ends(&fixture) == 2).then_some(())).await;
    fixture.command(
        &sessions,
        "command-3",
        SessionCommand::EndSession(EndSessionReason::UserEnded),
    );
    fixture.ended().await;

    let identity = identity();
    let signature = format!("{} <{}>", identity.name, identity.email);
    assert_eq!(
        (
            fixture.git(&["log", "--format=%s|%an <%ae>|%cn <%ce>"]),
            std::fs::read(fixture.checkout().join(".git").join("config")).unwrap(),
        ),
        (
            format!(
                "echo direct commit|{signature}|{signature}\necho host commit|{signature}|{signature}\n"
            ),
            config,
        ),
    );
}

/// A record too large for one Thread event is relayed as its position and kind, marked
/// truncated, while the history keeps it whole.
#[tokio::test(flavor = "multi_thread")]
async fn an_oversized_record_is_truncated_in_the_thread_only() {
    let fixture = Fixture::new();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start(&sessions, PLUGIN_VERSION, "[large]");
    until(|| (turn_ends(&fixture) == 1).then_some(())).await;
    fixture.command(
        &sessions,
        "command-2",
        SessionCommand::EndSession(EndSessionReason::UserEnded),
    );
    fixture.ended().await;

    let answer = fixture
        .ledger
        .events()
        .into_iter()
        .find(|event| event.truncated)
        .expect("the answer was truncated");
    let original = fixture
        .history_lines()
        .into_iter()
        .find(|line| line.get("seq") == answer.record.get("seq"))
        .expect("the history holds the answer");
    assert_eq!(
        (
            answer.record.keys().cloned().collect::<Vec<_>>(),
            field(&original, &["update", "content", "text"]).len(),
        ),
        (
            vec!["at".to_string(), "seq".to_string(), "type".to_string()],
            300 * 1024,
        ),
    );
}

/// Counts the turns the Thread shows as ended.
fn turn_ends(fixture: &Fixture) -> usize {
    fixture
        .ledger
        .events()
        .iter()
        .filter(|event| field(&event.record, &["type"]) == "turnEnded")
        .count()
}

/// Returns each recorded user message as (turn identity, text), in Thread order.
fn user_messages(fixture: &Fixture) -> Vec<(String, String)> {
    fixture
        .ledger
        .events()
        .iter()
        .filter(|event| field(&event.record, &["update", "sessionUpdate"]) == "user_message_chunk")
        .map(|event| {
            (
                field(&event.record, &["update", "messageId"]),
                field(&event.record, &["update", "content", "text"]),
            )
        })
        .collect()
}

/// Reads a string at `path` in a record, empty when absent.
pub(crate) fn field(record: &serde_json::Map<String, Value>, path: &[&str]) -> String {
    let mut value = record.get(path[0]);
    for key in &path[1..] {
        value = value.and_then(|value| value.get(key));
    }
    value
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// A failed durable Executed transition must stop the session before sending the queued prompt.
#[tokio::test(flavor = "multi_thread")]
async fn failed_command_settlement_never_runs_the_queued_turn() {
    let fixture = Fixture::new();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start(&sessions, PLUGIN_VERSION, "hello");
    until(|| {
        fixture
            .ledger
            .events()
            .iter()
            .any(|event| field(&event.record, &["type"]) == "turnEnded")
            .then_some(())
    })
    .await;
    fixture.ledger.fail_executed_settlements();
    fixture.command(
        &sessions,
        "failed-command",
        SessionCommand::SubmitUserTurn(turn("must-not-run", "must not run")),
    );
    let ended = fixture.ended().await;
    assert_eq!(ended.reason, AgentSessionEndReason::AgentFailed);
    assert_eq!(ended.detail.as_deref(), Some("ledger_unavailable"));
    assert!(
        !fixture
            .ledger
            .events()
            .iter()
            .any(|event| event.turn_id == Some(ora_node_protocol::TurnId::new("must-not-run")))
    );
    assert_eq!(
        fixture.ledger.settlements(),
        vec![("failed-command".into(), vec![CommandSettlement::Discarded])]
    );
    sessions.shutdown().await;
}

/// With a separate workload the agent runs from the session's package view with the session's
/// own home, and the session directory is gone once the session ended.
#[tokio::test(flavor = "multi_thread")]
async fn a_separate_workload_runs_the_agent_from_its_session_directory() {
    let fixture = Fixture::new();
    let sessions = fixture.sessions_with(PLUGIN_VERSION, fixture.separate_workload());
    fixture.start(&sessions, PLUGIN_VERSION, "[env]");
    let answer = until(|| {
        fixture.ledger.events().iter().find_map(|event| {
            (field(&event.record, &["update", "sessionUpdate"]) == "agent_message_chunk")
                .then(|| field(&event.record, &["update", "content", "text"]))
        })
    })
    .await;
    fixture.command(
        &sessions,
        "command-2",
        SessionCommand::EndSession(EndSessionReason::UserEnded),
    );
    let ended = fixture.ended().await;

    let name = ora_utils::hash::sha256_hex(EXECUTION.as_bytes());
    // The working directory is reported resolved; HOME is passed exactly as configured.
    let resolved = fixture
        .workload_directory()
        .canonicalize()
        .unwrap()
        .join(&name);
    let session = fixture.workload_directory().join(&name);
    assert_eq!(
        (
            answer,
            ended.reason,
            std::fs::read_dir(fixture.workload_directory())
                .unwrap()
                .count(),
        ),
        (
            format!(
                "cwd={} home={}",
                resolved.join("package").display(),
                session.join("home").display()
            ),
            AgentSessionEndReason::UserEnded,
            0,
        ),
    );
}

/// Grant routes the gateway fixture saw, without the renewals a slow run may add.
fn grant_routes(gateway: &gateway::Gateway) -> Vec<String> {
    gateway
        .requests
        .lock()
        .unwrap()
        .iter()
        .map(|(route, _)| route.clone())
        .filter(|route| !route.contains("/renew "))
        .collect()
}

/// A configured session hands its grant to the adapter in the startup context alone: the adapter
/// answers with the frozen model and the platform proxy, the token never reaches its environment,
/// history or Thread, none of the Node's own environment is inherited, and ending the session
/// revokes the grant.
#[tokio::test(flavor = "multi_thread")]
async fn a_configured_adapter_receives_its_grant_only_in_the_startup_context() {
    // The probe is the test runner's own environment, which the Node process inherits.
    assert!(std::env::vars().any(|(name, _)| name.starts_with("CARGO")));
    let fixture = Fixture::new();
    let gateway = gateway::Gateway::new(&fixture.home());
    let sessions = fixture.model_sessions(gateway.config.clone());
    fixture.start_configured(&sessions, "[model]", r#"{"effort":"high"}"#);
    let answer = until(|| {
        fixture.ledger.events().iter().find_map(|event| {
            (field(&event.record, &["update", "sessionUpdate"]) == "agent_message_chunk")
                .then(|| field(&event.record, &["update", "content", "text"]))
        })
    })
    .await;
    fixture.command(
        &sessions,
        "command-2",
        SessionCommand::EndSession(EndSessionReason::UserEnded),
    );
    let ended = fixture.ended().await;
    sessions.shutdown().await;
    let port = url::Url::parse(&gateway.config.gateway_url)
        .unwrap()
        .port()
        .unwrap();
    assert_eq!(
        (answer, ended.reason),
        (
            format!(
                "model=team/model proxy=https://localhost:{port}/runtime/openai/v1 token_in_environment=false host_environment=false"
            ),
            AgentSessionEndReason::UserEnded,
        )
    );
    let history = serde_json::to_string(&fixture.history_lines()).unwrap();
    let thread = format!("{:?}", fixture.ledger.events());
    assert!(!history.contains("synthetic-temporary-token"));
    assert!(!thread.contains("synthetic-temporary-token"));
    assert_eq!(
        grant_routes(&gateway),
        vec![
            "POST /internal/v1/model-grants HTTP/1.1",
            "DELETE /internal/v1/model-grants/grant-1 HTTP/1.1"
        ]
    );
    assert!(
        fixture
            .plugin_pids()
            .into_iter()
            .all(|pid| !process_exists(pid))
    );
}

/// An adapter that does not confirm the exact configuration it was given never receives a prompt:
/// the session fails before ACP starts, revokes the grant and stops the plugin.
#[tokio::test(flavor = "multi_thread")]
async fn an_unconfirmed_adapter_never_receives_a_prompt() {
    let fixture = Fixture::new();
    let gateway = gateway::Gateway::new(&fixture.home());
    let sessions = fixture.model_sessions(gateway.config.clone());
    fixture.start_configured(&sessions, "must not be sent", r#"{"refuse":true}"#);
    let ended = fixture.ended().await;
    sessions.shutdown().await;
    assert_eq!(
        (ended.reason, ended.detail.as_deref()),
        (
            AgentSessionEndReason::AgentFailed,
            Some("agent_unavailable")
        )
    );
    assert_eq!(fixture.ledger.events(), Vec::new());
    assert_eq!(
        grant_routes(&gateway),
        vec![
            "POST /internal/v1/model-grants HTTP/1.1",
            "DELETE /internal/v1/model-grants/grant-1 HTTP/1.1"
        ]
    );
    assert!(!fixture.plugin_pids().is_empty());
    assert!(
        fixture
            .plugin_pids()
            .into_iter()
            .all(|pid| !process_exists(pid))
    );
}

/// A configured session whose installed package no longer proves the frozen artifact,
/// declaration or Settings ends before any grant is requested or any plugin process starts.
#[tokio::test(flavor = "multi_thread")]
async fn a_configuration_the_installed_package_cannot_prove_never_gets_a_grant() {
    let fixture = Fixture::new();
    *fixture.configuration.lock().unwrap() = Err(ora_node::InstalledConfigurationError::Invalid);
    let gateway = gateway::Gateway::new(&fixture.home());
    let sessions = fixture.model_sessions(gateway.config.clone());
    fixture.start_configured(&sessions, "must not be sent", r#"{"effort":"high"}"#);
    let ended = fixture.ended().await;
    sessions.shutdown().await;
    assert_eq!(
        (ended.reason, ended.detail.as_deref()),
        (
            AgentSessionEndReason::AgentFailed,
            Some("agent_model_configuration_invalid")
        )
    );
    assert_eq!(
        (
            fixture.ledger.events(),
            grant_routes(&gateway),
            fixture.plugin_pids()
        ),
        (Vec::new(), Vec::<String>::new(), Vec::new())
    );
}

/// Only the legacy OpenCode package may take a model binding without a frozen configuration; any
/// other plugin is refused before a grant exists, so it never sees a live token.
#[tokio::test(flavor = "multi_thread")]
async fn an_undeclared_plugin_never_takes_the_legacy_model_route() {
    use ora_node_protocol::{
        AgentSessionSpec, ExecutionId, ModelBindingId, OperationId, PluginId, PluginVersion,
    };
    let fixture = Fixture::new();
    let gateway = gateway::Gateway::new(&fixture.home());
    let sessions = fixture.model_sessions(gateway.config.clone());
    sessions.start(
        OperationId::new(OPERATION),
        ExecutionId::new(EXECUTION),
        AgentSessionSpec {
            host_mcp_servers: Vec::new(),
            mcp_lifecycle: false,
            keep_session_on_cancel: false,
            keep_session_on_failure: false,
            mcp: Vec::new(),
            agent_configuration: None,
            node_id: fixture.node().node_id,
            agent_plugin_id: PluginId::new(PLUGIN_ID),
            agent_plugin_version: PluginVersion::new(PLUGIN_VERSION),
            checkout_execution_id: ExecutionId::new(CHECKOUT_EXECUTION),
            model_binding_id: Some(ModelBindingId::new("binding-1")),
            prior_revision: None,
            workspace_directory: vec![],
            git_identity: identity(),
            initial_turn: turn("turn-1", "must not be sent"),
            trigger: None,
        },
    );
    let ended = fixture.ended().await;
    sessions.shutdown().await;
    assert_eq!(
        (ended.reason, ended.detail.as_deref()),
        (
            AgentSessionEndReason::AgentFailed,
            Some("agent_model_configuration_required")
        )
    );
    assert_eq!(
        (
            fixture.ledger.events(),
            grant_routes(&gateway),
            fixture.plugin_pids()
        ),
        (Vec::new(), Vec::<String>::new(), Vec::new())
    );
}

/// Model access that ends at renewal stops the Agent first and records the end decision only
/// once the Controller's `EndSession` names the reason, so a restart can never publish the
/// provisional `model_access_revoked` (desktop#793 review).
#[tokio::test(flavor = "multi_thread")]
async fn a_model_ending_records_the_reason_of_the_end_session() {
    let fixture = Fixture::new();
    let gateway = gateway::Gateway::new(&fixture.home());
    gateway
        .behavior
        .ending_at_renewal
        .store(/*val*/ true, std::sync::atomic::Ordering::SeqCst);
    let sessions = fixture.model_sessions(gateway.config.clone());
    fixture.start_configured(&sessions, "[hold]", "{}");
    // The grant expires in two seconds; its renewal reports that Cloud is ending the session.
    until(|| {
        gateway
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|(route, _)| route.contains("/renew "))
            .then_some(())
    })
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let before = fixture.ledger.decision();
    fixture.command(
        &sessions,
        "command-2",
        SessionCommand::EndSession(EndSessionReason::Cancelled),
    );
    let ended = fixture.ended().await;
    sessions.shutdown().await;
    assert_eq!(
        (
            before,
            fixture.ledger.decision().map(|decision| decision.reason),
            ended.reason
        ),
        (
            None,
            Some(AgentSessionEndReason::Cancelled),
            AgentSessionEndReason::Cancelled
        )
    );
}

/// A cancel accepted while model authorization is still pending settles at once: the initial
/// turn was never admitted, so it is `not_started` before the grant is answered, and the session
/// then waits for its next turn instead of running it (desktop#793 review).
#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_during_model_authorization_settles_before_the_grant() {
    let fixture = Fixture::new();
    let gateway = gateway::Gateway::new(&fixture.home());
    gateway
        .behavior
        .hold_grant
        .store(/*val*/ true, std::sync::atomic::Ordering::SeqCst);
    let sessions = fixture.model_sessions(gateway.config.clone());
    fixture.start_configured_keeping(
        &sessions,
        "never answered",
        "{}",
        Keep {
            on_cancel: true,
            on_failure: false,
        },
    );
    until(|| (!gateway.requests.lock().unwrap().is_empty()).then_some(())).await;
    fixture.command(
        &sessions,
        "cancel-1",
        SessionCommand::CancelTurn(TurnId::new("turn-1")),
    );
    let settled = until(|| fixture.ledger.cancels().first().cloned()).await;
    gateway
        .behavior
        .hold_grant
        .store(/*val*/ false, std::sync::atomic::Ordering::SeqCst);
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    fixture.command(
        &sessions,
        "end",
        SessionCommand::EndSession(EndSessionReason::UserEnded),
    );
    fixture.ended().await;
    sessions.shutdown().await;
    assert_eq!(
        (settled, turn_ends(&fixture)),
        (
            (
                "cancel-1".to_string(),
                ora_node_protocol::TurnCancelOutcome::NotStarted
            ),
            0
        )
    );
}

/// Model access ending with a cancel pending: the cancel is settled within its deadline even
/// though the Controller's `EndSession` is later than that, and the end keeps the reason known
/// when the decision had to be made (desktop#793 review).
#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_during_a_model_ending_settles_within_its_deadline() {
    let mut fixture = Fixture::new();
    fixture.turn_cancel.grace = std::time::Duration::from_secs(1);
    let gateway = gateway::Gateway::new(&fixture.home());
    gateway
        .behavior
        .ending_at_renewal
        .store(/*val*/ true, std::sync::atomic::Ordering::SeqCst);
    // The revocation hangs, as a slow gateway's would: the decision must not wait for it.
    gateway
        .behavior
        .hold_revoke
        .store(/*val*/ true, std::sync::atomic::Ordering::SeqCst);
    let sessions = fixture.model_sessions(gateway.config.clone());
    fixture.start_configured_keeping(
        &sessions,
        "[hold]",
        "{}",
        Keep {
            on_cancel: true,
            on_failure: false,
        },
    );
    until(|| {
        gateway
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|(route, _)| route.contains("/renew "))
            .then_some(())
    })
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let accepted = std::time::Instant::now();
    fixture.ledger.accept(
        "cancel-1",
        SessionCommand::CancelTurn(TurnId::new("turn-1")),
    );
    sessions.turn_cancel_accepted(
        &ora_node_protocol::ExecutionId::new(EXECUTION),
        &ora_node_protocol::CommandId::new("cancel-1"),
        tokio::time::Instant::now(),
    );
    let settled = until(|| fixture.ledger.cancels().first().cloned()).await;
    let elapsed = accepted.elapsed();
    let decided_while_revoking =
        fixture.ledger.ended().is_none() && fixture.ledger.decision().is_some();
    gateway
        .behavior
        .hold_revoke
        .store(/*val*/ false, std::sync::atomic::Ordering::SeqCst);
    assert!(
        decided_while_revoking,
        "the decision is recorded before the revocation returns"
    );
    fixture.command(
        &sessions,
        "end",
        SessionCommand::EndSession(EndSessionReason::Cancelled),
    );
    let ended = fixture.ended().await;
    sessions.shutdown().await;
    assert_eq!(
        (settled.1, ended.detail),
        (
            ora_node_protocol::TurnCancelOutcome::SessionEnding,
            Some("model_access_revoked".to_string())
        )
    );
    // G is one second here; the Controller's end would only have been waited for 30 seconds.
    assert!(
        elapsed < std::time::Duration::from_secs(3),
        "settled after {elapsed:?}"
    );
}

/// A ledger error while a preparation step runs never ends the session ahead of that step: the
/// session stays unfinished until the held authorization returns, then ends with the ledger's
/// failure (desktop#793 review).
#[tokio::test(flavor = "multi_thread")]
async fn a_ledger_error_during_preparation_waits_for_the_step() {
    let fixture = Fixture::new();
    let gateway = gateway::Gateway::new(&fixture.home());
    gateway
        .behavior
        .hold_grant
        .store(/*val*/ true, std::sync::atomic::Ordering::SeqCst);
    let sessions = fixture.model_sessions(gateway.config.clone());
    fixture.start_configured_keeping(
        &sessions,
        "never answered",
        "{}",
        Keep {
            on_cancel: true,
            on_failure: false,
        },
    );
    until(|| (!gateway.requests.lock().unwrap().is_empty()).then_some(())).await;
    fixture.ledger.fail_next_queue_read();
    sessions.command_arrived(&ora_node_protocol::ExecutionId::new(EXECUTION));
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let ended_while_held = fixture.ledger.ended();
    gateway
        .behavior
        .hold_grant
        .store(/*val*/ false, std::sync::atomic::Ordering::SeqCst);
    let ended = fixture.ended().await;
    sessions.shutdown().await;
    assert_eq!(
        (ended_while_held, ended.detail),
        (None, Some("ledger_unavailable".to_string()))
    );
}
