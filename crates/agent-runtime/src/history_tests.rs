//! Covers what a session's file says about a handoff after the actor holding the
//! in-memory flag is gone.
//!
//! The runtime clears `handoff_pending` in memory, but a lost connection or a
//! restart destroys the actor that owned it, and the debt is then re-derived from
//! the file alone. These tests exercise that derivation over histories the
//! recorder actually produces, which is the half of the guarantee no assertion on
//! the flag itself can reach.

use super::history::{RecordOutcome, SessionRecorder};
use crate::test_host::RecordedEvents;
use agent_client_protocol_schema::v1::{ContentBlock, TextContent};
use agent_client_protocol_schema::v1::{ContentChunk, SessionUpdate};
use ora_domain::{AgentRef, HistoryState, SessionId};
use ora_history::{
    AgentSwitch, FixedHistoryClock, HistoryLine, HistoryRecord, binding_needs_handoff,
    read_session_history,
};
use pretty_assertions::assert_eq;
use std::path::Path;
use tempfile::TempDir;

/// Builds one installed agent identity these histories record a binding against.
fn agent_ref(package_name: &str) -> AgentRef {
    AgentRef::parse(package_name).expect("agent identity")
}
use time::macros::datetime;

const SESSION_ID: &str = "6f1e2d3c-4b5a-6978-8a9b-0c1d2e3f4a5b";
const NEW_PROVIDER_SESSION_ID: &str = "provider-2";
/// The instant every line carries, fixed so recorded output stays comparable.
const RECORDED_AT: &str = "2026-08-03T14:22:31.418+08:00";

/// A preview host observes pending content before the durable assembler settles it.
#[test]
fn live_preview_precedes_durable_message_and_clears_at_turn_end() {
    use crate::RuntimeEvents;
    use std::sync::{Arc, Mutex};
    #[derive(Clone, Default)]
    struct PreviewEvents(Arc<Mutex<Vec<Vec<ora_history::AssembledRecord>>>>);
    impl RuntimeEvents for PreviewEvents {
        fn session_title_updated(&self, _session: &SessionId) {}
        fn session_options_changed(&self, _session: &SessionId) {}
        fn agent_models_invalidated(&self, _agent: &AgentRef) {}
        fn record_settled(&self, _session: &SessionId, _line: &HistoryLine) {}
        fn wants_record_previews(&self) -> bool {
            true
        }
        fn record_preview(&self, _session: &SessionId, records: &[ora_history::AssembledRecord]) {
            self.0.lock().unwrap().push(records.to_vec());
        }
    }
    let root = TempDir::new().unwrap();
    let events = PreviewEvents::default();
    let mut recorder = SessionRecorder::open(
        root.path(),
        SESSION_ID,
        0,
        &HistoryState::Writable,
        events.clone(),
        FixedHistoryClock::new(datetime!(2026-08-03 14:22:31.418 +08:00)),
    )
    .unwrap();
    let update = SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
        TextContent::new("first real chunk"),
    )));
    recorder.record_update(&update);
    assert_eq!(
        events.0.lock().unwrap()[0][0].record,
        HistoryRecord::Update {
            update: Box::new(update),
            tool_timing: None
        }
    );
    assert_eq!(recorder.durable_bytes(), 0);
    recorder.record_turn_end(agent_client_protocol_schema::v1::StopReason::EndTurn);
    assert!(events.0.lock().unwrap().last().unwrap().is_empty());
    assert!(recorder.durable_bytes() > 0);
}

fn recorder(
    root: &Path,
    next_seq: u32,
    history_state: &HistoryState,
) -> SessionRecorder<RecordedEvents, FixedHistoryClock> {
    recorder_reporting_to(root, next_seq, history_state, RecordedEvents::default())
}

/// Opens a recorder whose settled lines are reported to `events`.
fn recorder_reporting_to(
    root: &Path,
    next_seq: u32,
    history_state: &HistoryState,
    events: RecordedEvents,
) -> SessionRecorder<RecordedEvents, FixedHistoryClock> {
    SessionRecorder::open(
        root,
        SESSION_ID,
        next_seq,
        history_state,
        events,
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
            agent_ref("ora-space.claude"),
            agent_ref("ora-space.nga"),
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
                    from: agent_ref("ora-space.claude"),
                    to: agent_ref("ora-space.nga"),
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
                    tool_timing: None,
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
        agent_ref("ora-space.claude"),
        agent_ref("ora-space.nga"),
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

/// A host mirroring the conversation must receive exactly the lines the file holds, in file order,
/// and nothing a stopped recorder did not write.
#[test]
fn settled_lines_reach_the_host_exactly_as_the_file_holds_them() {
    let root = TempDir::new().expect("create history root");
    let events = RecordedEvents::default();
    let mut writable =
        recorder_reporting_to(root.path(), 0, &HistoryState::Writable, events.clone());
    writable.record_prompt(&prompt("hello"));
    writable.record_update(&SessionUpdate::AgentMessageChunk(ContentChunk::new(
        ContentBlock::Text(TextContent::new("hi")),
    )));
    writable.record_turn_end(agent_client_protocol_schema::v1::StopReason::EndTurn);
    let mut stopped = recorder_reporting_to(
        root.path(),
        3,
        &HistoryState::Degraded {
            reason: "no space left on device".to_string(),
        },
        events.clone(),
    );
    stopped.record_prompt(&prompt("lost"));

    let history = read_session_history(root.path(), SESSION_ID).expect("read history");

    assert_eq!(
        events.settled(),
        history
            .lines
            .into_iter()
            .map(|line| (SessionId::new(SESSION_ID), line))
            .collect::<Vec<_>>(),
    );
}

/// A host's turn identity lands on every recorded block of that user message, through ACP's own
/// `messageId`, while the rest of the record is exactly what an unidentified prompt writes.
#[test]
fn an_identified_prompt_stamps_its_message_id_on_every_user_block() {
    let root = TempDir::new().expect("create history root");
    let mut recorder = recorder(root.path(), 0, &HistoryState::Writable);
    recorder.record_identified_prompt(
        &[
            ContentBlock::Text(TextContent::new("first")),
            ContentBlock::Text(TextContent::new("second")),
        ],
        agent_client_protocol_schema::v1::MessageId::new("turn-7"),
    );

    let history = read_session_history(root.path(), SESSION_ID).expect("read history");

    let user_block = |seq: u32, text: &str| {
        HistoryLine::new(
            RECORDED_AT,
            seq,
            HistoryRecord::Update {
                update: Box::new(SessionUpdate::UserMessageChunk(
                    ContentChunk::new(ContentBlock::Text(TextContent::new(text)))
                        .message_id(agent_client_protocol_schema::v1::MessageId::new("turn-7")),
                )),
                tool_timing: None,
            },
        )
    };
    assert_eq!(
        history.lines,
        vec![user_block(0, "first"), user_block(1, "second")]
    );
}
