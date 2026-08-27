use crate::assembler::AssembledRecord;
use crate::clock::FixedHistoryClock;
use crate::error::HistoryError;
use crate::path::history_path;
use crate::reader::{HistoryIntegrity, read_session_history, read_session_history_up_to};
use crate::record::{HistoryLine, HistoryRecord, SCHEMA_VERSION, SessionMeta};
use crate::writer::{HistoryWriter, remove_session_history};
use agent_client_protocol_schema::v1::StopReason;
use agent_client_protocol_schema::v1::{ContentBlock, TextContent};
use agent_client_protocol_schema::v1::{ContentChunk, SessionUpdate};
use ora_domain::AgentRef;
use pretty_assertions::assert_eq;
use std::path::{Path, PathBuf};
use time::OffsetDateTime;
use time::macros::datetime;

const SESSION_ID: &str = "6f1e2d3c-4b5a-6978-8a9b-0c1d2e3f4a5b";

fn clock() -> FixedHistoryClock {
    FixedHistoryClock::new(datetime!(2026-08-03 14:22:31.418 +08:00))
}

fn expected_timestamp() -> &'static str {
    "2026-08-03T14:22:31.418+08:00"
}

fn writer(root: &Path) -> HistoryWriter<FixedHistoryClock> {
    HistoryWriter::open(root, SESSION_ID, clock()).expect("open history writer")
}

fn message(text: &str) -> HistoryRecord {
    HistoryRecord::Update {
        update: Box::new(SessionUpdate::AgentMessageChunk(ContentChunk::new(
            ContentBlock::Text(TextContent::new(text)),
        ))),
    }
}

#[test]
fn derives_a_two_level_shard_from_the_session_identifier() {
    let path = history_path(Path::new("/data/sessions"), SESSION_ID).expect("derive path");

    assert_eq!(
        path,
        PathBuf::from("/data/sessions")
            .join("6f")
            .join("1e")
            .join(format!("{SESSION_ID}.jsonl")),
    );
}

#[test]
fn refuses_an_identifier_that_would_escape_the_history_root() {
    let error = history_path(Path::new("/data/sessions"), "../../etc/passwd");

    assert!(matches!(error, Err(HistoryError::InvalidSessionId { .. })));
}

#[test]
fn reports_an_empty_history_for_a_session_that_was_never_written() {
    let root = tempfile::tempdir().expect("create history root");

    let history = read_session_history(root.path(), SESSION_ID).expect("read history");

    assert_eq!(history.lines, vec![]);
    assert_eq!(history.next_seq, 0);
    assert_eq!(history.integrity, HistoryIntegrity::Complete);
}

#[test]
fn reads_only_the_durable_prefix_up_to_a_byte_cutoff() {
    let root = tempfile::tempdir().expect("create history root");
    let writer = writer(root.path());
    writer
        .append_record(0, message("first"))
        .expect("append first");
    let cutoff = writer.durable_bytes();
    writer
        .append_record(1, message("second"))
        .expect("append second");

    // Reading up to the cutoff returns only the first record, not the later append.
    let prefix = read_session_history_up_to(root.path(), SESSION_ID, cutoff).expect("read prefix");
    assert_eq!(prefix.lines.len(), 1);
    assert_eq!(prefix.next_seq, 1);

    // Reading the whole file returns both.
    let full = read_session_history(root.path(), SESSION_ID).expect("read full");
    assert_eq!(full.lines.len(), 2);
    assert_eq!(full.next_seq, 2);
}

#[test]
fn round_trips_appended_records_in_conversation_order() {
    let root = tempfile::tempdir().expect("create history root");
    let writer = writer(root.path());
    let meta = HistoryRecord::Meta(SessionMeta {
        schema_version: SCHEMA_VERSION,
        session_id: SESSION_ID.to_string(),
        workspace_id: "workspace-1".to_string(),
        agent_ref: AgentRef::parse("ora-space.nga").expect("agent identity"),
        agent_session_id: "provider-1".to_string(),
        cwd: PathBuf::from("/repo"),
    });

    writer.append_record(0, meta.clone()).expect("append meta");
    writer
        .append(&[
            AssembledRecord {
                seq: 1,
                record: message("hello"),
            },
            AssembledRecord {
                seq: 2,
                record: HistoryRecord::TurnEnded {
                    stop_reason: StopReason::EndTurn,
                },
            },
        ])
        .expect("append turn");

    let history = read_session_history(root.path(), SESSION_ID).expect("read history");

    assert_eq!(
        history.lines,
        vec![
            HistoryLine::new(expected_timestamp(), 0, meta),
            HistoryLine::new(expected_timestamp(), 1, message("hello")),
            HistoryLine::new(
                expected_timestamp(),
                2,
                HistoryRecord::TurnEnded {
                    stop_reason: StopReason::EndTurn,
                },
            ),
        ],
    );
    assert_eq!(history.next_seq, 3);
}

#[test]
fn restores_the_timeline_when_records_were_written_out_of_order() {
    let root = tempfile::tempdir().expect("create history root");
    let writer = writer(root.path());

    // A tool call that opened first but settled last is appended last.
    writer.append_record(1, message("later")).expect("append");
    writer.append_record(0, message("earlier")).expect("append");

    let history = read_session_history(root.path(), SESSION_ID).expect("read history");

    assert_eq!(
        history.lines,
        vec![
            HistoryLine::new(expected_timestamp(), 0, message("earlier")),
            HistoryLine::new(expected_timestamp(), 1, message("later")),
        ],
    );
}

#[test]
fn keeps_the_last_record_written_for_a_repeated_position() {
    let root = tempfile::tempdir().expect("create history root");
    let writer = writer(root.path());

    writer.append_record(0, message("first")).expect("append");
    writer
        .append_record(0, message("corrected"))
        .expect("append");

    let history = read_session_history(root.path(), SESSION_ID).expect("read history");

    assert_eq!(
        history.lines,
        vec![HistoryLine::new(
            expected_timestamp(),
            0,
            message("corrected"),
        )],
    );
    assert_eq!(history.next_seq, 1);
}

#[test]
fn discards_a_final_line_left_unfinished_by_an_interrupted_write() {
    let root = tempfile::tempdir().expect("create history root");
    let writer = writer(root.path());
    writer
        .append_record(0, message("complete"))
        .expect("append");
    // Emulate a process killed mid-append: a partial line with no terminator,
    // ending inside a multi-byte character.
    let mut torn = std::fs::read(writer.path()).expect("read history file");
    torn.extend_from_slice(b"{\"at\":\"2026\",\"seq\":1,\"type\":\"update\",\"upda\xe4\xbd");
    std::fs::write(writer.path(), torn).expect("write torn history file");

    let history = read_session_history(root.path(), SESSION_ID).expect("read history");

    assert_eq!(
        history.lines,
        vec![HistoryLine::new(
            expected_timestamp(),
            0,
            message("complete"),
        )],
    );
    assert_eq!(history.integrity, HistoryIntegrity::Complete);
}

#[test]
fn counts_a_damaged_line_that_is_not_the_interrupted_tail() {
    let root = tempfile::tempdir().expect("create history root");
    let writer = writer(root.path());
    writer.append_record(0, message("before")).expect("append");
    let mut damaged = std::fs::read(writer.path()).expect("read history file");
    damaged.extend_from_slice(b"{ not json }\n");
    std::fs::write(writer.path(), damaged).expect("write damaged history file");
    writer.append_record(1, message("after")).expect("append");

    let history = read_session_history(root.path(), SESSION_ID).expect("read history");

    assert_eq!(
        history.lines,
        vec![
            HistoryLine::new(expected_timestamp(), 0, message("before")),
            HistoryLine::new(expected_timestamp(), 1, message("after")),
        ],
    );
    assert_eq!(
        history.integrity,
        HistoryIntegrity::Damaged {
            unreadable_lines: std::num::NonZeroUsize::new(1).expect("one is non-zero"),
        },
    );
}

#[test]
fn removing_history_is_satisfied_by_a_file_that_never_existed() {
    let root = tempfile::tempdir().expect("create history root");
    let writer = writer(root.path());
    writer.append_record(0, message("hello")).expect("append");

    remove_session_history(root.path(), SESSION_ID).expect("remove history");
    remove_session_history(root.path(), SESSION_ID).expect("remove missing history");

    let history = read_session_history(root.path(), SESSION_ID).expect("read history");
    assert_eq!(history.lines, vec![]);
}

#[test]
fn stamps_every_line_with_the_local_time_the_clock_reported() {
    let root = tempfile::tempdir().expect("create history root");
    let at: OffsetDateTime = datetime!(2026-08-03 14:22:31.418 +08:00);
    let writer =
        HistoryWriter::open(root.path(), SESSION_ID, FixedHistoryClock::new(at)).expect("open");

    writer.append_record(0, message("hello")).expect("append");

    let history = read_session_history(root.path(), SESSION_ID).expect("read history");
    assert_eq!(
        history.lines.first().map(|line| line.at.clone()),
        Some(expected_timestamp().to_string()),
    );
}
