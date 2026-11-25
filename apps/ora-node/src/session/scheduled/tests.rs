#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::artifacts;
use super::declare;
use crate::session::end::SessionEnd;
use agent_client_protocol_schema::v1::{
    ContentBlock, ContentChunk, SessionUpdate, StopReason, TextContent,
};
use ora_domain::{MAX_SCHEDULED_ARTIFACT_BYTES, MAX_SCHEDULED_ARTIFACTS_PER_RUN};
use ora_history::{HistoryLine, HistoryRecord, history_path};
use ora_node_protocol::{
    AgentSessionEndReason, AgentSessionEnded, AgentSessionEndedMessage, AgentSessionResult,
    CURRENT_PROTOCOL_VERSION, EndSessionReason, ExecutionId, ExecutionResult, ExecutionState,
    ExecutionStatus, ExecutionStatusMessage, MAX_FRAME_LENGTH, MAX_SCHEDULED_ARTIFACT_REJECTIONS,
    NodeId, NodeIncarnationId, NodeRuntimeIdentity, NodeToControllerMessage, OperationId,
    SCHEDULED_ARTIFACT_DIRECTORY, ScheduledArtifactItem, ScheduledArtifactKind,
    ScheduledArtifactRejection, ScheduledArtifactRejectionReason, ScheduledVerdict, Sequence,
    SessionTrigger, encode_node_frame,
};
use pretty_assertions::assert_eq;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Builds the checkout of one scheduled run.
fn checkout() -> TempDir {
    TempDir::new().expect("a checkout directory")
}

/// Returns the reserved artifact directory inside one checkout, creating it.
fn artifact_directory(checkout: &Path) -> PathBuf {
    let directory = checkout.join(SCHEDULED_ARTIFACT_DIRECTORY);
    std::fs::create_dir_all(&directory).expect("the artifact directory");
    directory
}

/// Writes one session's history as the sealed JSONL the Node leaves behind.
fn write_history(root: &Path, execution: &ExecutionId, records: Vec<HistoryRecord>) {
    let path = history_path(root, execution.as_str()).expect("a history path");
    std::fs::create_dir_all(path.parent().expect("a history parent")).expect("the history tree");
    let mut file = String::new();
    for (index, record) in records.into_iter().enumerate() {
        let line = HistoryLine::new("2026-10-23T10:00:00+08:00", index as u32, record);
        file.push_str(&serde_json::to_string(&line).expect("a history line"));
        file.push('\n');
    }
    std::fs::write(&path, file).expect("the history file");
}

/// Builds one settled Assistant message.
fn assistant(text: &str) -> HistoryRecord {
    HistoryRecord::Update {
        update: Box::new(SessionUpdate::AgentMessageChunk(ContentChunk::new(
            ContentBlock::Text(TextContent::new(text)),
        ))),
        tool_timing: None,
    }
}

/// Closes one turn, which is what makes the following messages a later Assistant turn.
fn turn_ended() -> HistoryRecord {
    HistoryRecord::TurnEnded {
        stop_reason: StopReason::EndTurn,
        ended_by: None,
        failure: None,
    }
}

/// Closes one turn the way an end command does while the agent is still working.
fn turn_cancelled() -> HistoryRecord {
    HistoryRecord::TurnEnded {
        stop_reason: StopReason::Cancelled,
        ended_by: None,
        failure: None,
    }
}

/// Builds one settled user message, which opens a turn without any Assistant output.
fn user(text: &str) -> HistoryRecord {
    HistoryRecord::Update {
        update: Box::new(SessionUpdate::UserMessageChunk(ContentChunk::new(
            ContentBlock::Text(TextContent::new(text)),
        ))),
        tool_timing: None,
    }
}

/// Identifies the Node that settles one execution.
fn node() -> NodeRuntimeIdentity {
    NodeRuntimeIdentity {
        node_id: NodeId::new("node"),
        incarnation_id: NodeIncarnationId::new("incarnation"),
    }
}

/// Builds one declared item for the artifact of `text` at `name`.
fn item(index: u32, name: &str, kind: ScheduledArtifactKind, text: &str) -> ScheduledArtifactItem {
    ScheduledArtifactItem {
        index,
        display_name: name.to_string(),
        kind,
        size_bytes: text.len() as u64,
        sha256: ora_utils::hash::sha256_hex(text.as_bytes()),
    }
}

#[test]
fn a_manual_run_declares_nothing() {
    let root = checkout();
    let execution = ExecutionId::new("execution-manual");
    write_history(root.path(), &execution, vec![assistant("QA_VERDICT: fail")]);

    assert_eq!(
        declare(
            &SessionEnd::requested(EndSessionReason::IdleTimeout),
            None,
            root.path(),
            &execution,
            root.path(),
        ),
        None
    );
}

#[test]
fn a_scheduled_run_that_did_not_end_normally_declares_nothing() {
    let root = checkout();
    let execution = ExecutionId::new("execution-cancelled");
    write_history(root.path(), &execution, vec![assistant("QA_VERDICT: fail")]);
    artifact_directory(root.path());

    for end in [
        SessionEnd::requested(EndSessionReason::Cancelled),
        SessionEnd::agent_failed("agent_plugin_unavailable"),
    ] {
        assert_eq!(
            declare(
                &end,
                Some(SessionTrigger::Scheduled),
                root.path(),
                &execution,
                root.path(),
            ),
            None
        );
    }
}

#[test]
fn declares_the_verdict_of_the_final_assistant_turn() {
    let root = checkout();
    let execution = ExecutionId::new("execution-verdict");
    write_history(
        root.path(),
        &execution,
        vec![
            assistant("QA_VERDICT: pass"),
            turn_ended(),
            assistant(
                "Reviewed every changed file.\nQA_VERDICT: fail\nQA_DETAIL: two critical defects",
            ),
            turn_ended(),
        ],
    );

    let declaration = declare(
        &SessionEnd::requested(EndSessionReason::IdleTimeout),
        Some(SessionTrigger::Scheduled),
        root.path(),
        &execution,
        root.path(),
    )
    .expect("a declaration");

    assert_eq!(
        declaration.result,
        Some(ora_node_protocol::ScheduledRunResult {
            verdict: ScheduledVerdict::Fail,
            verdict_detail: Some("two critical defects".to_string()),
        })
    );
}

#[test]
fn an_unusable_marker_reports_no_verdict() {
    let root = checkout();
    let execution = ExecutionId::new("execution-illegal");
    write_history(
        root.path(),
        &execution,
        vec![assistant("QA_VERDICT: banana"), turn_ended()],
    );

    let declaration = declare(
        &SessionEnd::requested(EndSessionReason::UserEnded),
        Some(SessionTrigger::Scheduled),
        root.path(),
        &execution,
        root.path(),
    );

    assert_eq!(declaration, None);
}

#[test]
fn a_cancelled_final_turn_declares_nothing() {
    let root = checkout();
    let execution = ExecutionId::new("execution-cancelled-turn");
    write_history(
        root.path(),
        &execution,
        vec![assistant("QA_VERDICT: pass"), turn_cancelled()],
    );
    let directory = artifact_directory(root.path());
    std::fs::write(directory.join("report.md"), "hello").expect("a report");

    // The end command cancelled the turn that was still running, so neither the verdict nor the
    // artifacts it left behind describe a completed run.
    assert_eq!(
        declare(
            &SessionEnd::requested(EndSessionReason::UserEnded),
            Some(SessionTrigger::Scheduled),
            root.path(),
            &execution,
            root.path(),
        ),
        None
    );
}

#[test]
fn an_unterminated_final_turn_declares_nothing() {
    let root = checkout();
    let execution = ExecutionId::new("execution-unterminated");
    write_history(root.path(), &execution, vec![assistant("QA_VERDICT: pass")]);
    artifact_directory(root.path());

    assert_eq!(
        declare(
            &SessionEnd::requested(EndSessionReason::IdleTimeout),
            Some(SessionTrigger::Scheduled),
            root.path(),
            &execution,
            root.path(),
        ),
        None
    );
}

#[test]
fn a_final_turn_without_assistant_text_does_not_fall_back_to_an_earlier_verdict() {
    let root = checkout();
    let execution = ExecutionId::new("execution-empty-turn");
    write_history(
        root.path(),
        &execution,
        vec![
            assistant("QA_VERDICT: fail"),
            turn_ended(),
            user("try again"),
            turn_ended(),
        ],
    );

    assert_eq!(
        declare(
            &SessionEnd::requested(EndSessionReason::UserEnded),
            Some(SessionTrigger::Scheduled),
            root.path(),
            &execution,
            root.path(),
        ),
        None
    );
}

#[test]
fn a_detail_the_wire_refuses_keeps_the_artifacts_but_reports_no_verdict() {
    let root = checkout();
    let execution = ExecutionId::new("execution-nul-detail");
    write_history(
        root.path(),
        &execution,
        vec![assistant("QA_VERDICT: pass\nQA_DETAIL: a\0b"), turn_ended()],
    );
    let directory = artifact_directory(root.path());
    std::fs::write(directory.join("report.md"), "hello").expect("a report");

    let declaration = declare(
        &SessionEnd::requested(EndSessionReason::IdleTimeout),
        Some(SessionTrigger::Scheduled),
        root.path(),
        &execution,
        root.path(),
    )
    .expect("a declaration");

    // A detail the wire refuses degrades to "not reported"; it must not fail the terminal write
    // that settles the run, and it does not hide the independently collected artifacts.
    assert_eq!(declaration.result, None);
    assert_eq!(
        declaration.manifest.expect("artifacts are independent"),
        ora_node_protocol::ScheduledArtifactManifest {
            items: vec![item(
                0,
                "report.md",
                ScheduledArtifactKind::Markdown,
                "hello"
            )],
            rejections: Vec::new(),
            omitted_rejections: 0,
            walk_truncated: false,
        }
    );
}

#[test]
fn one_name_in_two_unicode_spellings_is_declared_once() {
    let root = checkout();
    let execution = ExecutionId::new("execution-unicode-names");
    write_history(
        root.path(),
        &execution,
        vec![assistant("QA_VERDICT: pass"), turn_ended()],
    );
    let directory = artifact_directory(root.path());
    std::fs::write(directory.join("Á.txt"), "upper").expect("an upper-case name");
    std::fs::write(directory.join("á.txt"), "lower").expect("a lower-case name");
    std::fs::write(directory.join("é.txt"), "composed").expect("a composed name");
    std::fs::write(directory.join("e\u{301}.txt"), "decomposed").expect("a decomposed name");

    let declaration = declare(
        &SessionEnd::requested(EndSessionReason::IdleTimeout),
        Some(SessionTrigger::Scheduled),
        root.path(),
        &execution,
        root.path(),
    )
    .expect("a declaration");
    let manifest = declaration.manifest.expect("a manifest");

    // The walk sorts each level, so the first spelling of each identity is the one declared and the
    // second is refused instead of making the terminal result unrepresentable.
    assert_eq!(
        manifest.items,
        vec![
            item(0, "e\u{301}.txt", ScheduledArtifactKind::Text, "decomposed"),
            item(1, "Á.txt", ScheduledArtifactKind::Text, "upper"),
        ]
    );
    assert_eq!(
        manifest.rejections,
        vec![
            ScheduledArtifactRejection {
                display_name: "á.txt".to_string(),
                reason: ScheduledArtifactRejectionReason::PathUnsafe,
            },
            ScheduledArtifactRejection {
                display_name: "é.txt".to_string(),
                reason: ScheduledArtifactRejectionReason::PathUnsafe,
            },
        ]
    );
    manifest.validate().expect("the declaration is reportable");
}

#[test]
fn a_flood_of_unusable_names_stays_inside_one_frame() {
    let root = checkout();
    let execution = ExecutionId::new("execution-flood");
    write_history(
        root.path(),
        &execution,
        vec![assistant("QA_VERDICT: warn"), turn_ended()],
    );
    let directory = artifact_directory(root.path());
    for index in 0..600 {
        let name = format!("{index:04}-{}.pdf", "x".repeat(220));
        std::fs::write(directory.join(name), "").expect("an unusable candidate");
    }

    let declaration = declare(
        &SessionEnd::requested(EndSessionReason::IdleTimeout),
        Some(SessionTrigger::Scheduled),
        root.path(),
        &execution,
        root.path(),
    )
    .expect("a declaration");
    let manifest = declaration.manifest.expect("a manifest");

    // The refusal list is bounded and the rest is a visible count, so the manifest stays small
    // instead of repeating one entry per file the agent happened to leave behind. The walk itself
    // stopped at its candidate bound, so the 600 files were never all read.
    assert_eq!(manifest.rejections.len(), MAX_SCHEDULED_ARTIFACT_REJECTIONS);
    assert!(manifest.omitted_rejections > 0);
    assert!(manifest.walk_truncated);
    manifest.validate().expect("the declaration is reportable");

    // Both messages that can carry the terminal result must still fit one frame: the end event
    // itself and the status query a reconnecting Controller sends.
    let ended = AgentSessionEnded {
        node: node(),
        reason: AgentSessionEndReason::IdleTimeout,
        detail: None,
        plugin_id: None,
        setting_id: None,
        transport: None,
        scheduled_result: declaration.result,
        artifact_manifest: Some(manifest),
    };
    let result = AgentSessionResult::AgentSessionEnded(ended.clone());
    let messages = [
        NodeToControllerMessage::AgentSessionEnded(AgentSessionEndedMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: OperationId::new("operation"),
            execution_id: execution.clone(),
            sequence: Sequence::new(/*value*/ 9),
            payload: result.clone(),
        }),
        NodeToControllerMessage::ExecutionStatus(ExecutionStatusMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: OperationId::new("operation"),
            execution_id: execution.clone(),
            payload: ExecutionStatus {
                stop_unconfirmed: Vec::new(),
                node: node(),
                state: ExecutionState::Completed(ExecutionResult::AgentSession(result)),
            },
        }),
    ];
    for message in messages {
        let frame = encode_node_frame(&message).expect("the terminal result is encodable");
        assert!(frame.len() < MAX_FRAME_LENGTH / 4);
    }
}

#[test]
fn a_completed_final_turn_without_assistant_text_still_declares_its_artifacts() {
    let root = checkout();
    let execution = ExecutionId::new("execution-tool-only");
    // A turn that finishes its work through tools never writes an Assistant message, but its
    // `TurnEnded(end_turn)` proves the agent finished: what it wrote is the run's declaration.
    write_history(
        root.path(),
        &execution,
        vec![user("write the report"), turn_ended()],
    );
    let directory = artifact_directory(root.path());
    std::fs::write(directory.join("report.md"), "hello").expect("a report");

    let declaration = declare(
        &SessionEnd::requested(EndSessionReason::IdleTimeout),
        Some(SessionTrigger::Scheduled),
        root.path(),
        &execution,
        root.path(),
    )
    .expect("a declaration");

    // The empty turn states no verdict, and its artifacts are not lost with it.
    assert_eq!(declaration.result, None);
    assert_eq!(
        declaration.manifest.expect("artifacts are independent"),
        ora_node_protocol::ScheduledArtifactManifest {
            items: vec![item(
                0,
                "report.md",
                ScheduledArtifactKind::Markdown,
                "hello"
            )],
            rejections: Vec::new(),
            omitted_rejections: 0,
            walk_truncated: false,
        }
    );
}

#[test]
fn a_run_without_artifacts_or_a_verdict_declares_nothing() {
    let root = checkout();
    let execution = ExecutionId::new("execution-quiet");
    write_history(
        root.path(),
        &execution,
        vec![assistant("nothing to report")],
    );

    assert_eq!(
        declare(
            &SessionEnd::requested(EndSessionReason::IdleTimeout),
            Some(SessionTrigger::Scheduled),
            root.path(),
            &execution,
            root.path(),
        ),
        None
    );
}

#[test]
fn collects_whitelisted_files_recursively_with_their_sizes_and_digests() {
    let checkout = checkout();
    let directory = artifact_directory(checkout.path());
    std::fs::create_dir_all(directory.join("nested")).expect("a nested directory");
    std::fs::write(directory.join("report.md"), "hello").expect("a report");
    std::fs::write(directory.join("nested/result.json"), "{}").expect("a result");

    assert_eq!(
        artifacts::collect(checkout.path()),
        Some(ora_node_protocol::ScheduledArtifactManifest {
            // The walk sorts each directory level, so the nested file comes first.
            items: vec![
                item(0, "nested/result.json", ScheduledArtifactKind::Json, "{}"),
                item(1, "report.md", ScheduledArtifactKind::Markdown, "hello"),
            ],
            rejections: Vec::new(),
            omitted_rejections: 0,
            walk_truncated: false,
        })
    );
}

#[test]
fn a_run_without_the_artifact_directory_declares_no_manifest() {
    assert_eq!(artifacts::collect(checkout().path()), None);
}

#[test]
fn refuses_a_candidate_outside_the_type_whitelist() {
    let checkout = checkout();
    let directory = artifact_directory(checkout.path());
    std::fs::write(directory.join("report.pdf"), "pdf").expect("a refused report");

    assert_eq!(
        artifacts::collect(checkout.path()),
        Some(ora_node_protocol::ScheduledArtifactManifest {
            items: Vec::new(),
            rejections: vec![ScheduledArtifactRejection {
                display_name: "report.pdf".to_string(),
                reason: ScheduledArtifactRejectionReason::UnsupportedType,
            }],
            omitted_rejections: 0,
            walk_truncated: false,
        })
    );
}

#[test]
fn refuses_a_candidate_larger_than_the_single_artifact_bound() {
    let checkout = checkout();
    let directory = artifact_directory(checkout.path());
    let oversized = vec![b'a'; MAX_SCHEDULED_ARTIFACT_BYTES as usize + 1];
    std::fs::write(directory.join("huge.log"), oversized).expect("an oversized log");

    assert_eq!(
        artifacts::collect(checkout.path()),
        Some(ora_node_protocol::ScheduledArtifactManifest {
            items: Vec::new(),
            rejections: vec![ScheduledArtifactRejection {
                display_name: "huge.log".to_string(),
                reason: ScheduledArtifactRejectionReason::TooLarge,
            }],
            omitted_rejections: 0,
            walk_truncated: false,
        })
    );
}

#[test]
fn declares_at_most_the_per_run_artifact_count() {
    let checkout = checkout();
    let directory = artifact_directory(checkout.path());
    for index in 0..=MAX_SCHEDULED_ARTIFACTS_PER_RUN {
        std::fs::write(directory.join(format!("report-{index:02}.txt")), "x").expect("a report");
    }

    let manifest = artifacts::collect(checkout.path()).expect("a manifest");

    assert_eq!(manifest.items.len(), MAX_SCHEDULED_ARTIFACTS_PER_RUN);
    assert_eq!(manifest.omitted_rejections, 0);
    assert_eq!(
        manifest.rejections,
        vec![ScheduledArtifactRejection {
            display_name: format!("report-{MAX_SCHEDULED_ARTIFACTS_PER_RUN:02}.txt"),
            reason: ScheduledArtifactRejectionReason::TooMany,
        }]
    );
}

#[cfg(unix)]
#[test]
fn refuses_a_symbolic_link_out_of_the_artifact_directory() {
    let checkout = checkout();
    let directory = artifact_directory(checkout.path());
    let outside = checkout.path().join("outside.md");
    std::fs::write(&outside, "QA_VERDICT: pass").expect("a file outside the directory");
    std::os::unix::fs::symlink(&outside, directory.join("link.md")).expect("a symbolic link");

    assert_eq!(
        artifacts::collect(checkout.path()),
        Some(ora_node_protocol::ScheduledArtifactManifest {
            items: Vec::new(),
            rejections: vec![ScheduledArtifactRejection {
                display_name: "link.md".to_string(),
                reason: ScheduledArtifactRejectionReason::Symlink,
            }],
            omitted_rejections: 0,
            walk_truncated: false,
        })
    );
}

#[cfg(unix)]
#[test]
fn refuses_a_hard_link_whose_inode_is_reachable_elsewhere() {
    let checkout = checkout();
    let directory = artifact_directory(checkout.path());
    let outside = checkout.path().join("outside.log");
    std::fs::write(&outside, "log").expect("a file outside the directory");
    std::fs::hard_link(&outside, directory.join("linked.log")).expect("a hard link");

    assert_eq!(
        artifacts::collect(checkout.path()),
        Some(ora_node_protocol::ScheduledArtifactManifest {
            items: Vec::new(),
            rejections: vec![ScheduledArtifactRejection {
                display_name: "linked.log".to_string(),
                reason: ScheduledArtifactRejectionReason::HardLink,
            }],
            omitted_rejections: 0,
            walk_truncated: false,
        })
    );
}

#[cfg(unix)]
#[test]
fn refuses_a_symlinked_artifact_directory_whole() {
    let checkout = checkout();
    let elsewhere = checkout.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("a directory elsewhere");
    std::fs::write(elsewhere.join("report.md"), "hello").expect("a report elsewhere");
    std::os::unix::fs::symlink(
        &elsewhere,
        checkout.path().join(SCHEDULED_ARTIFACT_DIRECTORY),
    )
    .expect("a symbolic link");

    assert_eq!(
        artifacts::collect(checkout.path()),
        Some(ora_node_protocol::ScheduledArtifactManifest {
            items: Vec::new(),
            rejections: vec![ScheduledArtifactRejection {
                display_name: SCHEDULED_ARTIFACT_DIRECTORY.to_string(),
                reason: ScheduledArtifactRejectionReason::Symlink,
            }],
            omitted_rejections: 0,
            walk_truncated: false,
        })
    );
}

#[test]
fn the_walk_reads_no_more_directory_entries_than_it_may_examine() {
    // A directory holding far more names than the walk may examine must not be read into memory
    // first: the budget is spent while enumerating, and one probe tells that more remained.
    let pulled = std::cell::Cell::new(0);
    let entries = (0..20_000).inspect(|_| pulled.set(pulled.get() + 1));

    let bounded = artifacts::bounded_entries(entries, 4);

    assert_eq!(bounded.entries.len(), 4);
    assert!(bounded.truncated);
    assert_eq!(pulled.get(), 5);
}

#[test]
fn a_directory_that_ends_at_the_candidate_bound_is_not_truncated() {
    // Exactly as many entries as the budget: every one was examined, so nothing is missing and the
    // manifest must not claim a cut.
    let bounded = artifacts::bounded_entries(0..4, 4);

    assert_eq!(bounded.entries.len(), 4);
    assert!(!bounded.truncated);
}

#[test]
fn a_cut_walk_states_the_level_it_stopped_in() {
    let checkout = checkout();
    let directory = artifact_directory(checkout.path());
    let nested = directory.join("nested");
    std::fs::create_dir_all(&nested).expect("a nested directory");
    // Empty directories are legal candidates that refuse nothing, so the refusal list still has room
    // when the walk runs out of candidates and can name the level it stopped in.
    for index in 0..600 {
        std::fs::create_dir_all(nested.join(format!("{index:04}"))).expect("a nested directory");
    }

    assert_eq!(
        artifacts::collect(checkout.path()),
        Some(ora_node_protocol::ScheduledArtifactManifest {
            items: Vec::new(),
            rejections: vec![ScheduledArtifactRejection {
                display_name: "nested".to_string(),
                reason: ScheduledArtifactRejectionReason::CandidateLimit,
            }],
            omitted_rejections: 0,
            walk_truncated: true,
        })
    );
}

#[test]
fn a_full_refusal_list_cannot_hide_a_cut_walk() {
    let checkout = checkout();
    let directory = artifact_directory(checkout.path());
    // The list fills with the first 64 unusable names, so the stop has no room for its own detail;
    // the manifest still states the cut on the wire instead of presenting a complete refusal list.
    for index in 0..600 {
        std::fs::write(directory.join(format!("{index:04}.pdf")), "").expect("a candidate");
    }

    let manifest = artifacts::collect(checkout.path()).expect("a manifest");

    assert_eq!(
        (
            manifest.items.len(),
            manifest.rejections.len(),
            manifest.omitted_rejections,
            manifest.walk_truncated
        ),
        (0, MAX_SCHEDULED_ARTIFACT_REJECTIONS, 448, true)
    );
    assert!(
        !manifest
            .rejections
            .iter()
            .any(|rejection| rejection.reason == ScheduledArtifactRejectionReason::CandidateLimit)
    );
    manifest.validate().expect("the declaration is reportable");
    let wire = serde_json::to_value(&manifest).expect("a manifest serializes");
    assert_eq!(wire["walk_truncated"], serde_json::Value::Bool(true));
    assert_eq!(wire["omitted_rejections"], serde_json::json!(448));
}
