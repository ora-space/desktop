use crate::assembler::{AssembledRecord, HistoryAssembler};
use crate::record::HistoryRecord;
use agent_client_protocol_schema::v1::StopReason;
use agent_client_protocol_schema::v1::{ContentBlock, TextContent};
use agent_client_protocol_schema::v1::{ContentChunk, MessageId, SessionUpdate};
use agent_client_protocol_schema::v1::{Plan, PlanEntry, PlanEntryPriority, PlanEntryStatus};
use agent_client_protocol_schema::v1::{
    ToolCall, ToolCallId, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields,
};
use pretty_assertions::assert_eq;

/// Builds one text chunk, optionally tied to a protocol message identity.
fn text_chunk(text: &str, message_id: Option<&str>) -> ContentChunk {
    let mut chunk = ContentChunk::new(ContentBlock::Text(TextContent::new(text)));
    chunk.message_id = message_id.map(MessageId::new);
    chunk
}

/// Builds one agent message chunk.
fn agent_text(text: &str, message_id: Option<&str>) -> SessionUpdate {
    SessionUpdate::AgentMessageChunk(text_chunk(text, message_id))
}

/// Builds one agent reasoning chunk sharing the message-chunk merging rules.
fn thought_text(text: &str, message_id: Option<&str>) -> SessionUpdate {
    SessionUpdate::AgentThoughtChunk(text_chunk(text, message_id))
}

/// Names the assembled message text at one position, for compact assertions.
fn message_at(seq: u32, text: &str, message_id: Option<&str>) -> AssembledRecord {
    AssembledRecord {
        seq,
        record: HistoryRecord::Update {
            update: Box::new(agent_text(text, message_id)),
        },
    }
}

fn tool_status_update(tool_call_id: &str, status: ToolCallStatus) -> SessionUpdate {
    SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
        ToolCallId::new(tool_call_id),
        ToolCallUpdateFields::new().status(status),
    ))
}

#[test]
fn merges_chunks_sharing_one_message_identity_into_a_single_record() {
    let mut assembler = HistoryAssembler::new(0);

    assert_eq!(
        assembler.push_update(&agent_text("Hel", Some("m1"))),
        vec![]
    );
    assert_eq!(
        assembler.push_update(&agent_text("lo ", Some("m1"))),
        vec![]
    );
    assert_eq!(
        assembler.push_update(&agent_text("there", Some("m1"))),
        vec![]
    );

    assert_eq!(
        assembler.end_turn(StopReason::EndTurn),
        vec![
            message_at(0, "Hello there", Some("m1")),
            AssembledRecord {
                seq: 1,
                record: HistoryRecord::TurnEnded {
                    stop_reason: StopReason::EndTurn,
                },
            },
        ],
    );
}

#[test]
fn settles_the_open_message_when_the_protocol_starts_a_new_one() {
    let mut assembler = HistoryAssembler::new(0);

    assembler.push_update(&agent_text("first", Some("m1")));

    // A changed messageId is ACP's signal that the previous message is complete,
    // so it reaches the file immediately rather than waiting for the turn.
    assert_eq!(
        assembler.push_update(&agent_text("second", Some("m2"))),
        vec![message_at(0, "first", Some("m1"))],
    );
    assert_eq!(
        assembler.end_turn(StopReason::EndTurn),
        vec![
            message_at(1, "second", Some("m2")),
            AssembledRecord {
                seq: 2,
                record: HistoryRecord::TurnEnded {
                    stop_reason: StopReason::EndTurn,
                },
            },
        ],
    );
}

#[test]
fn keeps_messages_and_thoughts_on_independent_streams() {
    let mut assembler = HistoryAssembler::new(0);

    assembler.push_update(&agent_text("answer ", None));
    assembler.push_update(&thought_text("reasoning ", None));
    assembler.push_update(&agent_text("continues", None));
    assembler.push_update(&thought_text("continues", None));

    assert_eq!(
        assembler.end_turn(StopReason::EndTurn),
        vec![
            message_at(0, "answer continues", None),
            AssembledRecord {
                seq: 1,
                record: HistoryRecord::Update {
                    update: Box::new(thought_text("reasoning continues", None)),
                },
            },
            AssembledRecord {
                seq: 2,
                record: HistoryRecord::TurnEnded {
                    stop_reason: StopReason::EndTurn,
                },
            },
        ],
    );
}

#[test]
fn keeps_text_that_resumed_after_a_tool_call_behind_it() {
    let mut assembler = HistoryAssembler::new(0);

    // ACP makes `messageId` optional, and an agent that omits it streams text
    // whose runs cannot be told apart. Contiguity is then the only evidence that
    // chunks are one message, so the tool call between these two proves they are
    // not: merging them would anchor the second run to position 0 and record the
    // agent summarizing work before the tool call it describes.
    assembler.push_update(&agent_text("Let me look", None));
    let opened = assembler.push_update(&SessionUpdate::ToolCall(
        ToolCall::new("t1", "Read file").status(ToolCallStatus::Completed),
    ));
    assembler.push_update(&agent_text("here is what I found", None));

    assert_eq!(
        opened,
        vec![
            message_at(0, "Let me look", None),
            AssembledRecord {
                seq: 1,
                record: HistoryRecord::Update {
                    update: Box::new(SessionUpdate::ToolCall(
                        ToolCall::new("t1", "Read file").status(ToolCallStatus::Completed),
                    )),
                },
            },
        ],
    );
    assert_eq!(
        assembler.end_turn(StopReason::EndTurn),
        vec![
            message_at(2, "here is what I found", None),
            AssembledRecord {
                seq: 3,
                record: HistoryRecord::TurnEnded {
                    stop_reason: StopReason::EndTurn,
                },
            },
        ],
    );
}

#[test]
fn keeps_an_identified_message_whole_across_a_tool_call_it_spans() {
    let mut assembler = HistoryAssembler::new(0);

    // A `messageId` is ACP's own statement that these chunks are one message, so
    // an interleaved tool call does not divide it and the position it opened at
    // still describes where it belongs.
    assembler.push_update(&agent_text("reading, ", Some("m1")));
    assembler.push_update(&SessionUpdate::ToolCall(
        ToolCall::new("t1", "Read file").status(ToolCallStatus::Completed),
    ));
    assembler.push_update(&agent_text("done", Some("m1")));

    assert_eq!(
        assembler.end_turn(StopReason::EndTurn),
        vec![
            message_at(0, "reading, done", Some("m1")),
            AssembledRecord {
                seq: 2,
                record: HistoryRecord::TurnEnded {
                    stop_reason: StopReason::EndTurn,
                },
            },
        ],
    );
}

#[test]
fn writes_a_tool_call_once_it_reaches_a_terminal_status() {
    let mut assembler = HistoryAssembler::new(0);

    let opened = ToolCall::new("t1", "Read file").status(ToolCallStatus::Pending);
    assert_eq!(
        assembler.push_update(&SessionUpdate::ToolCall(opened)),
        vec![],
    );
    assert_eq!(
        assembler.push_update(&tool_status_update("t1", ToolCallStatus::InProgress)),
        vec![],
    );

    let settled = ToolCall::new("t1", "Read file").status(ToolCallStatus::Completed);
    assert_eq!(
        assembler.push_update(&tool_status_update("t1", ToolCallStatus::Completed)),
        vec![AssembledRecord {
            seq: 0,
            record: HistoryRecord::Update {
                update: Box::new(SessionUpdate::ToolCall(settled)),
            },
        }],
    );
}

#[test]
fn reissues_a_settled_tool_call_under_its_original_position() {
    let mut assembler = HistoryAssembler::new(0);

    assembler.push_update(&SessionUpdate::ToolCall(
        ToolCall::new("t1", "Run tests").status(ToolCallStatus::Completed),
    ));

    // A provider that corrects a finished call must not create a second entry;
    // readers resolve the repeated position by keeping the last record.
    let corrected = assembler.push_update(&SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
        "t1",
        ToolCallUpdateFields::new().status(ToolCallStatus::Failed),
    )));
    assert_eq!(
        corrected,
        vec![AssembledRecord {
            seq: 0,
            record: HistoryRecord::Update {
                update: Box::new(SessionUpdate::ToolCall(
                    ToolCall::new("t1", "Run tests").status(ToolCallStatus::Failed),
                )),
            },
        }],
    );
}

#[test]
fn preserves_appearance_order_when_a_tool_settles_after_a_later_message() {
    let mut assembler = HistoryAssembler::new(0);

    assembler.push_update(&SessionUpdate::ToolCall(
        ToolCall::new("t1", "Search").status(ToolCallStatus::InProgress),
    ));
    assembler.push_update(&agent_text("meanwhile", Some("m1")));
    let settled = assembler.push_update(&tool_status_update("t1", ToolCallStatus::Completed));

    // The tool is written after the message but keeps the earlier position, which
    // is the only thing that lets a reader rebuild the timeline.
    assert_eq!(
        settled,
        vec![AssembledRecord {
            seq: 0,
            record: HistoryRecord::Update {
                update: Box::new(SessionUpdate::ToolCall(
                    ToolCall::new("t1", "Search").status(ToolCallStatus::Completed),
                )),
            },
        }],
    );
    assert_eq!(assembler.next_seq(), 2);
}

#[test]
fn settles_a_tool_call_the_provider_never_reported_finishing() {
    let mut assembler = HistoryAssembler::new(0);

    // The agent opens a call, moves straight on to unrelated output, and ends the
    // turn on its own terms without ever reporting a terminal status.
    assembler.push_update(&SessionUpdate::ToolCall(
        ToolCall::new("t1", "Read file").status(ToolCallStatus::InProgress),
    ));
    assembler.push_update(&agent_text("here is what I found", Some("m1")));

    assert_eq!(
        assembler.end_turn(StopReason::EndTurn),
        vec![
            AssembledRecord {
                seq: 0,
                record: HistoryRecord::Update {
                    update: Box::new(SessionUpdate::ToolCall(
                        ToolCall::new("t1", "Read file").status(ToolCallStatus::Completed),
                    )),
                },
            },
            message_at(1, "here is what I found", Some("m1")),
            AssembledRecord {
                seq: 2,
                record: HistoryRecord::TurnEnded {
                    stop_reason: StopReason::EndTurn,
                },
            },
        ],
    );
}

#[test]
fn keeps_an_unfinished_tool_call_unfinished_when_the_turn_was_cut_short() {
    // Every ending other than the agent stopping on its own terms may have
    // interrupted the call, so none of them may credit it with success. Ora's
    // runtime records a broken connection and a failed prompt as `Cancelled`, so
    // these cover network failure and user cancellation alike.
    for stop_reason in [
        StopReason::Cancelled,
        StopReason::MaxTokens,
        StopReason::MaxTurnRequests,
        StopReason::Refusal,
    ] {
        let mut assembler = HistoryAssembler::new(0);
        assembler.push_update(&SessionUpdate::ToolCall(
            ToolCall::new("t1", "Run tests").status(ToolCallStatus::InProgress),
        ));

        assert_eq!(
            assembler.end_turn(stop_reason),
            vec![
                AssembledRecord {
                    seq: 0,
                    record: HistoryRecord::Update {
                        update: Box::new(SessionUpdate::ToolCall(
                            ToolCall::new("t1", "Run tests").status(ToolCallStatus::InProgress),
                        )),
                    },
                },
                AssembledRecord {
                    seq: 1,
                    record: HistoryRecord::TurnEnded { stop_reason },
                },
            ],
            "{stop_reason:?} must not record an outcome the provider never gave",
        );
    }
}

#[test]
fn never_reinterprets_a_failure_the_provider_did_report() {
    let mut assembler = HistoryAssembler::new(0);

    assembler.push_update(&SessionUpdate::ToolCall(
        ToolCall::new("t1", "Fetch data").status(ToolCallStatus::Failed),
    ));

    // The turn ending cleanly says nothing about a call that already failed.
    assert_eq!(
        assembler.end_turn(StopReason::EndTurn),
        vec![AssembledRecord {
            seq: 1,
            record: HistoryRecord::TurnEnded {
                stop_reason: StopReason::EndTurn,
            },
        }],
    );
}

#[test]
fn synthesizes_a_tool_call_from_an_update_that_arrived_without_its_opening() {
    let mut assembler = HistoryAssembler::new(0);

    let records = assembler.push_update(&SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
        "t9",
        ToolCallUpdateFields::new().status(ToolCallStatus::Completed),
    )));

    assert_eq!(
        records,
        vec![AssembledRecord {
            seq: 0,
            record: HistoryRecord::Update {
                update: Box::new(SessionUpdate::ToolCall(
                    ToolCall::new("t9", "Tool call").status(ToolCallStatus::Completed),
                )),
            },
        }],
    );
}

#[test]
fn keeps_only_the_final_plan_snapshot() {
    let mut assembler = HistoryAssembler::new(0);
    let final_plan = Plan::new(vec![PlanEntry::new(
        "ship it",
        PlanEntryPriority::High,
        PlanEntryStatus::Completed,
    )]);

    assembler.push_update(&SessionUpdate::Plan(Plan::new(vec![PlanEntry::new(
        "ship it",
        PlanEntryPriority::High,
        PlanEntryStatus::Pending,
    )])));
    assert_eq!(
        assembler.push_update(&SessionUpdate::Plan(final_plan.clone())),
        vec![],
    );

    assert_eq!(
        assembler.end_turn(StopReason::EndTurn),
        vec![
            AssembledRecord {
                seq: 0,
                record: HistoryRecord::Update {
                    update: Box::new(SessionUpdate::Plan(final_plan)),
                },
            },
            AssembledRecord {
                seq: 1,
                record: HistoryRecord::TurnEnded {
                    stop_reason: StopReason::EndTurn,
                },
            },
        ],
    );
}

#[test]
fn records_the_prompt_ora_kept_and_ignores_the_provider_echo() {
    let mut assembler = HistoryAssembler::new(0);
    let prompt = vec![ContentBlock::Text(TextContent::new("what the user typed"))];

    let recorded = assembler.push_user_prompt(&prompt);
    let echo = assembler.push_update(&SessionUpdate::UserMessageChunk(ContentChunk::new(
        ContentBlock::Text(TextContent::new("injected context + what the user typed")),
    )));

    assert_eq!(
        recorded,
        vec![AssembledRecord {
            seq: 0,
            record: HistoryRecord::Update {
                update: Box::new(SessionUpdate::UserMessageChunk(ContentChunk::new(
                    ContentBlock::Text(TextContent::new("what the user typed")),
                ))),
            },
        }],
    );
    assert_eq!(echo, vec![]);
}

#[test]
fn drops_session_chrome_that_every_binding_reestablishes() {
    use agent_client_protocol_schema::v1::{SessionInfoUpdate, UsageUpdate};
    let mut assembler = HistoryAssembler::new(0);

    assert_eq!(
        assembler.push_update(&SessionUpdate::UsageUpdate(UsageUpdate::new(10, 100))),
        vec![],
    );
    assert_eq!(
        assembler.push_update(&SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new())),
        vec![],
    );
    assert_eq!(assembler.next_seq(), 0);
}

#[test]
fn settles_content_that_cannot_merge_where_it_arrived() {
    use agent_client_protocol_schema::v1::ImageContent;
    let mut assembler = HistoryAssembler::new(0);
    let image = ContentBlock::Image(ImageContent::new("data", "image/png"));
    let mut expected_chunk = ContentChunk::new(image.clone());
    expected_chunk.message_id = Some("m1".into());

    let mut chunk = ContentChunk::new(image);
    chunk.message_id = Some("m1".into());
    let records = assembler.push_update(&SessionUpdate::AgentMessageChunk(chunk));

    assert_eq!(
        records,
        vec![AssembledRecord {
            seq: 0,
            record: HistoryRecord::Update {
                update: Box::new(SessionUpdate::AgentMessageChunk(expected_chunk)),
            },
        }],
    );
}
