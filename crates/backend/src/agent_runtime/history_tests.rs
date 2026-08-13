//! Covers what a session's file says about a handoff after the actor holding the
//! in-memory flag is gone.
//!
//! The runtime clears `handoff_pending` in memory, but a lost connection or a
//! restart destroys the actor that owned it, and the debt is then re-derived from
//! the file alone. These tests exercise that derivation over histories the
//! recorder actually produces, which is the half of the guarantee no assertion on
//! the flag itself can reach.

use super::history::{RecordOutcome, SessionRecorder};
use agent_client_protocol_schema::v1::{ContentBlock, TextContent};
use agent_client_protocol_schema::v1::{ContentChunk, SessionUpdate};
use ora_domain::{AgentCli, HistoryState};
use ora_history::{
    AgentSwitch, FixedHistoryClock, HistoryLine, HistoryRecord, binding_needs_handoff,
    read_session_history,
};
use pretty_assertions::assert_eq;
use std::path::Path;
use tempfile::TempDir;
use time::macros::datetime;

const SESSION_ID: &str = "6f1e2d3c-4b5a-6978-8a9b-0c1d2e3f4a5b";
const NEW_PROVIDER_SESSION_ID: &str = "provider-2";
/// The instant every line carries, fixed so recorded output stays comparable.
const RECORDED_AT: &str = "2026-08-03T14:22:31.418+08:00";

fn recorder(
    root: &Path,
    next_seq: u32,
    history_state: &HistoryState,
) -> SessionRecorder<FixedHistoryClock> {
    SessionRecorder::open(
        root,
        SESSION_ID,
        next_seq,
        history_state,
        FixedHistoryClock::new(datetime!(2026-08-03 14:22:31.418 +08:00)),
    )
    .expect("open recorder")
}

fn prompt(text: &str) -> Vec<ContentBlock> {
    vec![ContentBlock::Text(TextContent::new(text))]
}

/// Replays what the runtime writes for a switch followed by one prompt.
///
/// The prompt is recorded before `session/prompt` is sent, so every history here
/// contains the user's turn; only the delivery line separates a transcript that
/// reached the new agent from one that never left Ora.
fn switch_then_prompt(root: &Path, deliver: Delivery) {
    let mut recorder = recorder(root, 0, &HistoryState::Writable);
    assert_eq!(
        recorder.record_agent_switch(
            AgentCli::OpenCode,
            AgentCli::Nga,
            NEW_PROVIDER_SESSION_ID.to_string(),
        ),
        RecordOutcome::Continued,
    );
    assert_eq!(
        recorder.record_prompt(&prompt("carry on")),
        RecordOutcome::Continued,
    );
    match deliver {
        Delivery::Accepted => assert_eq!(
            recorder.record_handoff_delivered(NEW_PROVIDER_SESSION_ID.to_string()),
            RecordOutcome::Continued,
        ),
        // The failed send leaves the file exactly as the prompt left it.
        Delivery::NeverSent => {}
    }
}

/// Whether the provider accepted the prompt that carried the transcript.
enum Delivery {
    Accepted,
    NeverSent,
}

#[test]
fn a_prompt_the_provider_never_accepted_leaves_the_handoff_owed_on_disk() {
    let root = TempDir::new().expect("create history root");
    switch_then_prompt(root.path(), Delivery::NeverSent);

    let history = read_session_history(root.path(), SESSION_ID).expect("read history");

    assert_eq!(binding_needs_handoff(&history), true);
}

#[test]
fn an_accepted_prompt_settles_the_handoff_on_disk() {
    let root = TempDir::new().expect("create history root");
    switch_then_prompt(root.path(), Delivery::Accepted);

    let history = read_session_history(root.path(), SESSION_ID).expect("read history");

    assert_eq!(binding_needs_handoff(&history), false);
}

#[test]
fn a_settled_handoff_records_the_delivery_after_the_prompt_that_carried_it() {
    let root = TempDir::new().expect("create history root");
    switch_then_prompt(root.path(), Delivery::Accepted);

    let history = read_session_history(root.path(), SESSION_ID).expect("read history");

    assert_eq!(
        history.lines,
        vec![
            HistoryLine::new(
                RECORDED_AT,
                0,
                HistoryRecord::AgentSwitched(AgentSwitch {
                    from: AgentCli::OpenCode,
                    to: AgentCli::Nga,
                    agent_session_id: NEW_PROVIDER_SESSION_ID.to_string(),
                }),
            ),
            HistoryLine::new(
                RECORDED_AT,
                1,
                HistoryRecord::Update {
                    update: Box::new(SessionUpdate::UserMessageChunk(ContentChunk::new(
                        ContentBlock::Text(TextContent::new("carry on")),
                    ))),
                },
            ),
            HistoryLine::new(
                RECORDED_AT,
                2,
                HistoryRecord::HandoffDelivered {
                    agent_session_id: NEW_PROVIDER_SESSION_ID.to_string(),
                },
            ),
        ],
    );
}

#[test]
fn a_recorder_that_stopped_writing_records_no_delivery_it_cannot_prove() {
    // A degraded session opens stopped, so the delivery line never reaches the
    // file. The next actor reads the binding as still owing its transcript and
    // hands it over again — the harmless direction to be wrong in.
    let root = TempDir::new().expect("create history root");
    let mut writable = recorder(root.path(), 0, &HistoryState::Writable);
    writable.record_agent_switch(
        AgentCli::OpenCode,
        AgentCli::Nga,
        NEW_PROVIDER_SESSION_ID.to_string(),
    );
    let mut degraded = recorder(
        root.path(),
        1,
        &HistoryState::Degraded {
            reason: "no space left on device".to_string(),
        },
    );
    degraded.record_prompt(&prompt("carry on"));
    degraded.record_handoff_delivered(NEW_PROVIDER_SESSION_ID.to_string());

    let history = read_session_history(root.path(), SESSION_ID).expect("read history");

    assert_eq!(binding_needs_handoff(&history), true);
}
