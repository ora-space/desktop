//! Failures of a restored session's prompt that happen around its sends: the first write, the
//! watchdog's resend and the one rebuild after a stale `session/load`. Each must record the same
//! failure on `TurnEnded` as the error the prompt's owner receives.
//!
//! Evidence for specs/test-cases/node/protocol/turn-failure-reason.md cases 7 and 9.
use super::*;
use ora_history::{HistoryRecord, TurnEndedBy, TurnFailure, TurnFailureCode};
use pretty_assertions::{assert_eq, assert_ne};
use tokio::sync::Notify;

/// How the scripted Agent behaves.
#[derive(Clone, Copy)]
enum Script {
    /// Holds every prompt and answers the watchdog's cancel with `cancelled` once released. It
    /// keeps serving through a plugin shutdown until the process is killed, so a write refused by
    /// the shutting-down plugin is not also an end of the generation.
    Attached,
    /// Rejects the prompt as a stale session and refuses to create a replacement.
    RefuseReplacement,
    /// Rejects the prompt as a stale session and answers the replacement only when released.
    HoldReplacement,
}

/// A provider following `script`.
async fn scripted_agent(
    script: Script,
    released: Arc<Notify>,
    reader: DuplexStream,
    mut writer: DuplexStream,
    frames: mpsc::UnboundedSender<Value>,
    exited: watch::Sender<bool>,
) {
    let registration = json!({"jsonrpc":"2.0","method":"ora/register",
        "params":{"methods":[],"emits":["agent/acp"]}});
    write_message(&mut writer, &registration).await.unwrap();
    let (envelopes, mut received) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut reader = reader;
        while let Ok(Some(envelope)) = read_message(&mut reader).await {
            if envelopes.send(envelope).is_err() {
                break;
            }
        }
    });
    let mut killed = exited.subscribe();
    let mut held = Value::Null;
    loop {
        let envelope = tokio::select! {
            envelope = received.recv() => envelope,
            _ = killed.wait_for(|exited| *exited) => None,
        };
        let Some(envelope) = envelope else { break };
        if envelope["method"] == "ora/shutdown" {
            if matches!(script, Script::Attached) {
                continue;
            }
            break;
        }
        let frame = &envelope["params"];
        frames.send(frame.clone()).unwrap();
        let id = &frame["id"];
        let mut response = match (script, frame["method"].as_str()) {
            (_, Some("session/load")) => json!({"id":id, "result":{}}),
            (Script::Attached, Some("session/prompt")) => {
                held = id.clone();
                continue;
            }
            (Script::Attached, Some("session/cancel")) => {
                released.notified().await;
                json!({"id":std::mem::take(&mut held), "result":PromptResponse::new(StopReason::Cancelled)})
            }
            (Script::RefuseReplacement | Script::HoldReplacement, Some("session/prompt")) => {
                json!({"id":id, "error":{"code":-32603, "message":"stale session"}})
            }
            (Script::RefuseReplacement, Some("session/new")) => {
                json!({"id":id, "error":{"code":-32603, "message":"cannot create"}})
            }
            (Script::HoldReplacement, Some("session/new")) => {
                released.notified().await;
                json!({"id":id, "result":{"sessionId":"replacement"}})
            }
            _ => continue,
        };
        response["jsonrpc"] = json!("2.0");
        write_message(
            &mut writer,
            &json!({"jsonrpc":"2.0","method":"agent/acp","params":response}),
        )
        .await
        .unwrap();
    }
    exited.send_replace(true);
}

/// A fixture whose provider follows `script`, serving the session `restored`: already attached
/// for [`Script::Attached`], restored by `session/load` on its first prompt otherwise.
async fn scripted(script: Script) -> (ReviewDrainFixture, Arc<Notify>) {
    let released = Arc::new(Notify::new());
    let gate = released.clone();
    let fixture = ReviewDrainFixture::new_with(move |reader, writer, _, frames, exited| {
        scripted_agent(script, gate, reader, writer, frames, exited)
    })
    .await;
    match script {
        Script::Attached => {
            let channel = fixture
                .supervisor
                .open_session_channel("restored", "restored")
                .unwrap();
            fixture.add_session_with(
                "restored",
                crate::TitleAcquisition::disabled(),
                Some(channel),
                LiveMcpState::Active(crate::SessionMcpRevision::default()),
            );
        }
        Script::RefuseReplacement | Script::HoldReplacement => fixture.add_session("restored"),
    }
    (fixture, released)
}

/// Lets the plugin supervisor act on a shutdown request, after which every write is refused
/// while the process, and so the connection generation, still runs.
async fn begin_plugin_shutdown(fixture: &ReviewDrainFixture) {
    fixture.plugin.shutdown();
    for _ in 0..32 {
        tokio::task::yield_now().await;
    }
}

/// Every `TurnEnded` and `PromptRetried` the restored session recorded, in order.
fn turn_records(fixture: &ReviewDrainFixture) -> Vec<HistoryRecord> {
    ora_history::read_session_history(&fixture.manager.inner.sessions_root, "restored")
        .unwrap()
        .lines
        .into_iter()
        .filter_map(|line| match line.record {
            record @ (HistoryRecord::TurnEnded { .. } | HistoryRecord::PromptRetried) => {
                Some(record)
            }
            _ => None,
        })
        .collect()
}

/// Reads the stream until its failure.
async fn failure_of(stream: &mut SessionEventStream<PromptSessionEvent>) -> RuntimeError {
    while let Some(event) = stream.recv().await {
        if let Err(error) = event {
            return error;
        }
    }
    panic!("prompt ended without a failure")
}

/// The Agent's absence, recorded on a cancelled `TurnEnded` without attribution.
fn unavailable() -> TurnFailure {
    TurnFailure {
        code: TurnFailureCode::AgentUnavailable,
        acp_code: None,
        message: None,
        message_truncated: None,
        attempts: None,
        answer: None,
    }
}

/// A cancelled `TurnEnded` carrying `failure`.
fn failed(failure: TurnFailure) -> HistoryRecord {
    HistoryRecord::TurnEnded {
        stop_reason: StopReason::Cancelled,
        ended_by: None,
        failure: Some(failure),
    }
}

/// Case 7: a transport failure on the first `session/prompt` write is the Agent's absence.
#[test]
fn a_prompt_that_cannot_be_written_fails_as_agent_unavailable() {
    run_case(async {
        let (fixture, _) = scripted(Script::Attached).await;
        begin_plugin_shutdown(&fixture).await;
        let mut stream = fixture
            .manager
            .prompt_session(prompt("restored"))
            .await
            .unwrap();
        let error = failure_of(&mut stream).await;
        assert_eq!(error.turn_failure(), Some(&unavailable()));
        assert_eq!(turn_records(&fixture), vec![failed(unavailable())]);
        fixture.shutdown().await;
    });
}

/// Case 7: a transport failure on the watchdog's resend is the Agent's absence too, not a stall.
#[test]
fn a_resend_that_cannot_be_written_fails_as_agent_unavailable() {
    run_case(async {
        let (mut fixture, released) = scripted(Script::Attached).await;
        let mut stream = fixture
            .manager
            .prompt_session(prompt("restored"))
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        tokio::time::advance(Duration::from_secs(45)).await;
        assert_eq!(fixture.next_frame().await["method"], "session/cancel");
        begin_plugin_shutdown(&fixture).await;
        released.notify_one();
        let error = failure_of(&mut stream).await;
        assert_eq!(error.turn_failure(), Some(&unavailable()));
        assert_eq!(
            turn_records(&fixture),
            vec![HistoryRecord::PromptRetried, failed(unavailable())]
        );
        fixture.shutdown().await;
    });
}

/// Case 1: when the rebuild after a stale load fails, the turn records the Agent's prompt error,
/// which is the error its owner is told.
#[test]
fn a_failed_rebuild_records_the_failure_its_owner_is_told() {
    run_case(async {
        let (mut fixture, _) = scripted(Script::RefuseReplacement).await;
        let mut stream = fixture
            .manager
            .prompt_session(prompt("restored"))
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/load");
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        assert_eq!(fixture.next_frame().await["method"], "session/new");
        let error = failure_of(&mut stream).await;
        let failure = TurnFailure {
            code: TurnFailureCode::AgentError,
            acp_code: Some(-32603),
            message: Some("stale session".into()),
            ..unavailable()
        };
        assert_eq!(error.turn_failure(), Some(&failure));
        assert_eq!(turn_records(&fixture), vec![failed(failure)]);
        fixture.shutdown().await;
    });
}

/// Case 9: an owner gone while the stale session is rebuilt is the host ending the session.
#[test]
fn an_owner_gone_during_the_rebuild_ends_the_turn_as_session_end() {
    run_case(async {
        let (mut fixture, released) = scripted(Script::HoldReplacement).await;
        let stream = fixture
            .manager
            .prompt_session(prompt("restored"))
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/load");
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        assert_eq!(fixture.next_frame().await["method"], "session/new");
        drop(stream);
        released.notify_one();
        // The replacement prompt is never written; the actor can serve the next command.
        fixture
            .manager
            .stop_session(ora_contracts::StopSessionRequest {
                session_id: "restored".to_string(),
            })
            .await
            .unwrap();
        assert_eq!(
            turn_records(&fixture),
            vec![HistoryRecord::TurnEnded {
                stop_reason: StopReason::Cancelled,
                ended_by: Some(TurnEndedBy::SessionEnd),
                failure: None,
            }]
        );
        while let Ok(frame) = fixture.frames.try_recv() {
            assert_ne!(frame["method"], "session/prompt", "no replacement prompt");
        }
        fixture.shutdown().await;
    });
}

/// Case 7: a lost connection seen on the replacement session before the rebuilt prompt is resent
/// still fails the recorded turn, as the Agent's absence.
#[test]
fn a_connection_lost_before_the_rebuilt_resend_fails_as_agent_unavailable() {
    run_case(async {
        let (fixture, released) = scripted(Script::HoldReplacement).await;
        fixture.supervisor.routes.control_on_register(
            "replacement",
            crate::routing::SessionControl::ConnectionLost(crate::runtime_unavailable()),
        );
        released.notify_one();
        let mut stream = fixture
            .manager
            .prompt_session(prompt("restored"))
            .await
            .unwrap();
        failure_of(&mut stream).await;
        assert_eq!(turn_records(&fixture), vec![failed(unavailable())]);
        fixture.shutdown().await;
    });
}

/// Case 7: an event queue that overflowed on the replacement session before the rebuilt prompt is
/// resent fails the recorded turn on Ora's side, as `internal`.
#[test]
fn an_overflow_before_the_rebuilt_resend_fails_as_internal() {
    run_case(async {
        let (fixture, released) = scripted(Script::HoldReplacement).await;
        fixture
            .supervisor
            .routes
            .control_on_register("replacement", crate::routing::SessionControl::QueueOverflow);
        released.notify_one();
        let mut stream = fixture
            .manager
            .prompt_session(prompt("restored"))
            .await
            .unwrap();
        failure_of(&mut stream).await;
        let internal = TurnFailure {
            code: TurnFailureCode::Internal,
            ..unavailable()
        };
        assert_eq!(turn_records(&fixture), vec![failed(internal)]);
        fixture.shutdown().await;
    });
}
