//! Covers when a session's command reports leave a trace for being beyond what a client shows.

use super::*;
use crate::discovery::tests::{Recorded, warnings};
use agent_client_protocol_schema::v1::{AvailableCommand, AvailableCommandsUpdate};
use ora_contracts::{SHOWN_AGENT_COMMAND_LIMIT, StartSessionRequest};
use pretty_assertions::assert_eq;

/// Frames a report of this many commands for one provider session.
fn report(session: &str, count: usize) -> Value {
    let commands = (0..count)
        .map(|index| AvailableCommand::new(format!("c{index}"), "Runs it"))
        .collect();
    json!({"jsonrpc":"2.0", "method":"session/update", "params": SessionNotification::new(
        session.to_string(),
        SessionUpdate::AvailableCommandsUpdate(AvailableCommandsUpdate::new(commands)))})
}

/// Reports these counts, the first right after setup and the others during the prompt.
///
/// It cannot restore a session, and reports `stale` for the one it is asked to before failing.
async fn reporting_agent(
    stale: Option<usize>,
    setup: usize,
    live: &'static [usize],
    mut reader: DuplexStream,
    mut writer: DuplexStream,
    exited: watch::Sender<bool>,
) {
    let registration = json!({"jsonrpc":"2.0","method":"ora/register",
        "params":{"methods":[],"emits":["agent/acp"]}});
    write_message(&mut writer, &registration).await.unwrap();
    while let Ok(Some(envelope)) = read_message(&mut reader).await {
        if envelope["method"] == "ora/shutdown" {
            break;
        }
        let frame = &envelope["params"];
        let answer = |result: Value| json!({"jsonrpc":"2.0", "id":frame["id"], "result":result});
        let replies = match frame["method"].as_str() {
            Some("session/load") => stale
                .map(|count| report("stale", count))
                .into_iter()
                .chain([json!({"jsonrpc":"2.0", "id":frame["id"],
                    "error":{"code":-32000, "message":"gone"}})])
                .collect(),
            Some("session/new") => vec![
                answer(json!({"sessionId":"created"})),
                report("created", setup),
            ],
            Some("session/prompt") => live
                .iter()
                .map(|count| report("created", *count))
                .chain([answer(json!(PromptResponse::new(StopReason::EndTurn)))])
                .collect(),
            _ => continue,
        };
        for reply in replies {
            let envelope = json!({"jsonrpc":"2.0", "method":"agent/acp", "params":reply});
            write_message(&mut writer, &envelope).await.unwrap();
        }
    }
    exited.send_replace(true);
}

/// Which session the turn runs on.
#[derive(Clone, Copy)]
enum Session {
    New,
    /// One whose provider session is gone, so the same actor gets a replacement.
    Rebuilt {
        stale: Option<usize>,
    },
}

/// Runs a session's first turn and returns the size of every report that was recorded.
fn recorded_counts(session: Session, setup: usize, live: &'static [usize]) -> Vec<String> {
    let recorded = Recorded::default();
    ora_logging::initialize_test_clock();
    ora_logging::with_recorded_trace_logging(recorded.clone(), || {
        block_on_paused_clock(async move {
            let stale = match session {
                Session::New => None,
                Session::Rebuilt { stale } => stale,
            };
            let fixture = ReviewDrainFixture::new_with(move |reader, writer, _, _, exited| {
                reporting_agent(stale, setup, live, reader, writer, exited)
            })
            .await;
            let session_id = match session {
                Session::New => {
                    let started = fixture
                        .manager
                        .start_session(StartSessionRequest {
                            workspace_id: "workspace".into(),
                            agent_ref: "official/test.agent".into(),
                            model: None,
                        })
                        .await
                        .unwrap();
                    started.session.id
                }
                Session::Rebuilt { .. } => {
                    fixture.add_session("stale");
                    "stale".into()
                }
            };
            let mut turn = fixture
                .manager
                .prompt_session(prompt(&session_id))
                .await
                .unwrap();
            assert_eq!(completion(&mut turn).await, StopReason::EndTurn);
            drop(turn);
            fixture.shutdown().await;
        });
    });
    warnings(&recorded)
}

/// Each excess is recorded once, in setup or live, however often the agent repeats it.
#[test]
fn a_session_records_each_excess_once() {
    const OVER: usize = SHOWN_AGENT_COMMAND_LIMIT + 1;
    for (setup, live, expected) in [
        (OVER, &[OVER, OVER][..], vec!["501"]),
        (OVER, &[1, OVER + 1][..], vec!["501", "502"]),
        (1, &[OVER, OVER, 1, OVER][..], vec!["501", "501"]),
        (SHOWN_AGENT_COMMAND_LIMIT, &[1][..], vec![]),
    ] {
        assert_eq!(recorded_counts(Session::New, setup, live), expected);
    }
}

/// A replacement provider session is held to its own setup, not to what its predecessor reported.
#[test]
fn a_rebuilt_session_records_each_excess_of_its_replacement_once() {
    const OVER: usize = SHOWN_AGENT_COMMAND_LIMIT + 1;
    for (stale, setup, live, expected) in [
        (None, OVER, &[OVER, OVER][..], vec!["501"]),
        (None, 1, &[OVER, OVER][..], vec!["501"]),
        (Some(OVER), 1, &[OVER + 1][..], vec!["501", "502"]),
        (Some(OVER), OVER + 1, &[OVER + 1][..], vec!["501", "502"]),
    ] {
        assert_eq!(
            recorded_counts(Session::Rebuilt { stale }, setup, live),
            expected
        );
    }
}
