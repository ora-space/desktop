//! Checks source-preserving assembly through the same snapshots used for live reload and JSONL.

use crate::{HistoryAssembler, HistoryRecord};
use agent_client_protocol_schema::v1::{
    ContentChunk, Plan, SessionUpdate, StopReason, ToolCall, ToolCallStatus,
};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

/// Parses a real wire shape so unknown optional metadata participates in the assertion.
fn update(value: Value) -> SessionUpdate {
    serde_json::from_value(value).unwrap()
}

/// Reads the settled or pending payloads without losing metadata to a text-only assertion.
fn payloads(records: &[crate::AssembledRecord]) -> Vec<SessionUpdate> {
    records
        .iter()
        .filter_map(|record| match &record.record {
            HistoryRecord::Update { update, .. } => Some(*update.clone()),
            _ => None,
        })
        .collect()
}

/// Constructs a text stream with independently scoped message identity and complete metadata.
fn text(id: Option<&str>, source: &str, body: &str) -> SessionUpdate {
    update(
        json!({"sessionUpdate":"agent_message_chunk", "messageId":id,
        "content":{"type":"text","text":body,"_meta":{"format":"plain"},"annotations":{"audience":["assistant"]}},
        "_meta":{"opencode/child-session":{"id":source,"parentID":"root","depth":1}}}),
    )
}

#[test]
fn projected_text_keeps_sources_and_annotations_with_same_or_missing_message_ids() {
    for id in [Some("shared"), None] {
        let mut assembler = HistoryAssembler::new(0);
        let first = text(id, "left", "A");
        let second = text(id, "right", "B");
        let mut emitted = assembler.push_update(&first);
        emitted.extend(assembler.push_update(&second));
        emitted.extend(assembler.push_update(&text(id, "right", "C")));
        let mut pending = emitted.clone();
        pending.extend(assembler.pending_records());
        let expected = vec![first, text(id, "right", "BC")];
        assert_eq!(payloads(&pending), expected);
        emitted.extend(assembler.end_turn(StopReason::EndTurn, None, None));
        assert_eq!(payloads(&emitted), expected);
        // A serialized roundtrip exercises the existing schema rather than a parallel DTO.
        for record in emitted {
            let encoded = serde_json::to_value(&record.record).unwrap();
            let decoded: HistoryRecord = serde_json::from_value(encoded).unwrap();
            assert_eq!(decoded, record.record);
        }
    }
}

#[test]
fn non_text_chunks_and_thoughts_keep_their_full_metadata() {
    let mut assembler = HistoryAssembler::new(0);
    let image = update(
        json!({"sessionUpdate":"agent_message_chunk","content":{"type":"image","data":"YWJj","mimeType":"image/png"},"_meta":{"source":"child"}}),
    );
    assert_eq!(payloads(&assembler.push_update(&image)), vec![image]);
    let mut chunk = ContentChunk::new("reason".into());
    chunk.meta = Some(serde_json::from_value(json!({"source":"child"})).unwrap());
    let thought = SessionUpdate::AgentThoughtChunk(chunk);
    assembler.push_update(&thought);
    assert_eq!(payloads(&assembler.pending_records()), vec![thought]);
}

#[test]
fn late_tool_metadata_replaces_namespaces_and_survives_terminal_refinement() {
    let mut assembler = HistoryAssembler::new(0);
    assembler.push_update(&SessionUpdate::ToolCall(ToolCall::new("tool", "Work")));
    let settled = update(
        json!({"sessionUpdate":"tool_call_update","toolCallId":"tool","status":"completed","_meta":{"opencode/child-session":{"id":"child","parentID":"root","depth":1},"other":{"old":true}}}),
    );
    let first = assembler.push_update(&settled);
    let cleared = update(
        json!({"sessionUpdate":"tool_call_update","toolCallId":"tool","_meta":{"opencode/child-session":null,"other":{"new":true}}}),
    );
    let last = assembler.push_update(&cleared);
    assert_eq!(first[0].seq, last[0].seq);
    let SessionUpdate::ToolCall(call) = &payloads(&last)[0] else {
        panic!("tool snapshot")
    };
    assert_eq!(
        call.meta,
        Some(
            serde_json::from_value(json!({"opencode/child-session":null,"other":{"new":true}}))
                .unwrap()
        )
    );
    assert_eq!(call.status, ToolCallStatus::Completed);
}

#[test]
fn parent_end_does_not_complete_unsettled_projected_tools() {
    let mut assembler = HistoryAssembler::new(0);
    let child = update(
        json!({"sessionUpdate":"tool_call","toolCallId":"child:tool","title":"Work","status":"in_progress","_meta":{"opencode/child-session":{"id":"child","parentID":"root","depth":1}}}),
    );
    assembler.push_update(&child);
    assert_eq!(
        payloads(&assembler.end_turn(StopReason::EndTurn, None, None)),
        vec![child]
    );
}

#[test]
fn plans_from_distinct_sources_do_not_overwrite_each_other() {
    let mut assembler = HistoryAssembler::new(0);
    let parent = SessionUpdate::Plan(Plan::new(vec![]));
    let child = update(
        json!({"sessionUpdate":"plan","entries":[],"_meta":{"opencode/child-session":{"id":"child","parentID":"root","depth":1}}}),
    );
    assembler.push_update(&parent);
    assembler.push_update(&child);
    assert_eq!(
        payloads(&assembler.pending_records()),
        vec![parent.clone(), child.clone()]
    );
    assert_eq!(
        payloads(&assembler.end_turn(StopReason::EndTurn, None, None)),
        vec![parent, child]
    );
}

#[test]
fn cleared_or_invalid_child_sources_settle_as_parent_tools() {
    for source in [
        Value::Null,
        json!({}),
        json!({"id":"","parentID":"root","depth":1}),
        json!({"id":"  ","parentID":"root","depth":1}),
        json!({"id":"child","parentID":"","depth":1}),
        json!({"id":"child","parentID":"  ","depth":1}),
        json!({"id":"child","parentID":"root","depth":0}),
        json!({"id":"child","parentID":"root","depth":-1}),
        json!({"id":"child","parentID":"root","depth":1.5}),
        json!({"id":"child","parentID":"root","depth":"1"}),
        json!({"id":"child","parentID":"root","depth":9007199254740992_u64}),
    ] {
        let mut assembler = HistoryAssembler::new(0);
        assembler.push_update(&update(json!({
            "sessionUpdate":"tool_call","toolCallId":"tool","title":"Work",
            "status":"in_progress",
            "_meta":{"opencode/child-session":{"id":"child","parentID":"root","depth":1}}
        })));
        assembler.push_update(&update(json!({
            "sessionUpdate":"tool_call_update","toolCallId":"tool",
            "_meta":{"opencode/child-session":source}
        })));
        let expected = update(json!({
            "sessionUpdate":"tool_call","toolCallId":"tool","title":"Work",
            "status":"completed","_meta":{"opencode/child-session":source,"ora/turn-end-settled":true}
        }));
        let records = assembler.end_turn(StopReason::EndTurn, None, None);
        assert_eq!(payloads(&records), vec![expected.clone()]);
        assert_eq!(reload_payloads(&records), vec![expected]);
    }
}

#[test]
fn integral_float_depth_keeps_the_same_child_source_as_live_projection() {
    let mut assembler = HistoryAssembler::new(0);
    let child = update(json!({
        "sessionUpdate":"tool_call","toolCallId":"child:tool","title":"Work",
        "status":"in_progress",
        "_meta":{"opencode/child-session":{"id":"child","parentID":"root","depth":1.0}}
    }));
    assembler.push_update(&child);
    let records = assembler.end_turn(StopReason::EndTurn, None, None);
    assert_eq!(payloads(&records), vec![child.clone()]);
    assert_eq!(reload_payloads(&records), vec![child]);
}

/// Exercises the production JSONL writer and reader for settled snapshots.
fn reload_payloads(records: &[crate::AssembledRecord]) -> Vec<SessionUpdate> {
    let root = tempfile::tempdir().unwrap();
    let id = "c3000000-0000-4000-8000-000000000002";
    let writer = crate::HistoryWriter::open(
        root.path(),
        id,
        crate::FixedHistoryClock::new(time::macros::datetime!(2026-10-20 10:00:00 +08:00)),
    )
    .unwrap();
    writer.append(records).unwrap();
    let history = crate::read_session_history(root.path(), id).unwrap();
    payloads(
        &history
            .lines
            .iter()
            .map(|line| crate::AssembledRecord {
                seq: line.seq,
                record: line.record.clone(),
            })
            .collect::<Vec<_>>(),
    )
}

#[test]
fn plans_replace_by_source_despite_changing_metadata_through_reload() {
    let mut assembler = HistoryAssembler::new(0);
    let plan = |source: Value, revision| {
        update(json!({
            "sessionUpdate":"plan",
            "entries":[{"content":format!("revision {revision}"),"priority":"medium","status":"pending"}],
            "_meta":{"opencode/child-session":source,"revision":revision}
        }))
    };
    let child = json!({"id":"child","parentID":"root","depth":1});
    let sibling = plan(json!({"id":"sibling","parentID":"root","depth":1}), 1);
    assembler.push_update(&plan(Value::Null, 1));
    assembler.push_update(&plan(child, 1));
    assembler.push_update(&sibling);
    // Source identity is the id, not changing extension fields or relation details.
    let latest_child = plan(
        json!({"id":"child","parentID":"other","depth":2,"extra":true}),
        2,
    );
    let latest_parent = plan(json!({"id":"invalid"}), 2);
    assembler.push_update(&latest_child);
    assembler.push_update(&latest_parent);
    let expected = vec![latest_parent, latest_child, sibling];
    assert_eq!(payloads(&assembler.pending_records()), expected);
    let records = assembler.end_turn(StopReason::EndTurn, None, None);
    assert_eq!(payloads(&records), expected);
    assert_eq!(reload_payloads(&records), expected);
}

#[test]
fn unidentified_source_switches_preserve_order_through_disk_reload() {
    let root = tempfile::tempdir().unwrap();
    let id = "c3000000-0000-4000-8000-000000000001";
    let writer = crate::HistoryWriter::open(
        root.path(),
        id,
        crate::FixedHistoryClock::new(time::macros::datetime!(2026-10-20 10:00:00 +08:00)),
    )
    .unwrap();
    let mut assembler = HistoryAssembler::new(0);
    let expected = vec![
        text(None, "left", "A"),
        text(None, "right", "B"),
        text(None, "left", "C"),
    ];
    for event in &expected {
        writer.append(&assembler.push_update(event)).unwrap();
    }
    writer
        .append(&assembler.end_turn(StopReason::EndTurn, None, None))
        .unwrap();
    let history = crate::read_session_history(root.path(), id).unwrap();
    let records = history
        .lines
        .iter()
        .map(|line| crate::AssembledRecord {
            seq: line.seq,
            record: line.record.clone(),
        })
        .collect::<Vec<_>>();
    assert_eq!(payloads(&records), expected);
    assert_eq!(crate::SCHEMA_VERSION, 2);
}

#[test]
fn delegation_results_survive_a_late_snapshot_and_reload() {
    let mut assembler = HistoryAssembler::new(0);
    assembler.push_update(&update(json!({
        "sessionUpdate":"tool_call","toolCallId":"delegate","title":"subagent","kind":"think",
        "status":"in_progress","rawInput":{"prompt":"survey"}
    })));
    let settled = assembler.push_update(&update(json!({
        "sessionUpdate":"tool_call_update","toolCallId":"delegate","status":"completed",
        "rawOutput":{"metadata":{"sessionID":"ses_child","status":"completed"}}
    })));
    assert_eq!(settled.len(), 1);
    // A later snapshot that only repeats the status must keep the provider's delegation link,
    // because the child group is identified by this raw output rather than by the tool title.
    let late = assembler.push_update(&update(json!({
        "sessionUpdate":"tool_call_update","toolCallId":"delegate","status":"completed"
    })));
    let expected = vec![update(json!({
        "sessionUpdate":"tool_call","toolCallId":"delegate","title":"subagent","kind":"think",
        "status":"completed","rawInput":{"prompt":"survey"},
        "rawOutput":{"metadata":{"sessionID":"ses_child","status":"completed"}}
    }))];
    assert_eq!(payloads(&late), expected);
    assert_eq!(reload_payloads(&late), expected);
    // The settled snapshot was written already, so ending the turn must not add another copy.
    assert!(payloads(&assembler.end_turn(StopReason::EndTurn, None, None)).is_empty());
}

#[test]
fn subagent_update_sources_isolate_messages_tools_and_plans_through_reload() {
    let mut assembler = HistoryAssembler::new(0);
    let subagent_meta = |task_id: &str| {
        json!({
            "_subagent/update": {
                "sessionId": "session-main",
                "taskId": task_id,
                "toolCallId": format!("parent-{task_id}"),
                "agentType": "Explore",
            }
        })
    };
    let first_chunk = update(json!({
        "sessionUpdate": "agent_message_chunk",
        "content": { "type": "text", "text": "Part 1 " },
        "_meta": subagent_meta("task-a"),
    }));
    let other_task_chunk = update(json!({
        "sessionUpdate": "agent_message_chunk",
        "content": { "type": "text", "text": "Other task" },
        "_meta": subagent_meta("task-b"),
    }));
    let unfinished_tool = update(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "task-a:tool-1",
        "title": "Read file",
        "kind": "read",
        "status": "pending",
        "_meta": {
            "_subagent/update": {
                "sessionId": "session-main",
                "taskId": "task-a",
                "toolCallId": "parent-task-a",
                "agentType": "Explore",
            },
            "codeAgentCLI": { "toolName": "Read" },
        },
    }));
    let mut emitted = assembler.push_update(&first_chunk);
    // Identical `_subagent/update` attribution merges contiguous text chunks on the same task.
    emitted.extend(assembler.push_update(&update(json!({
        "sessionUpdate": "agent_message_chunk",
        "content": { "type": "text", "text": "Part 2" },
        "_meta": subagent_meta("task-a"),
    }))));
    emitted.extend(assembler.push_update(&other_task_chunk));
    emitted.extend(assembler.push_update(&unfinished_tool));
    emitted.extend(assembler.end_turn(StopReason::EndTurn, None, None));

    let expected = vec![
        update(json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": "Part 1 Part 2" },
            "_meta": subagent_meta("task-a"),
        })),
        other_task_chunk,
        // Unsettled subagent tools must not be synthesized as completed when the parent turn ends.
        unfinished_tool,
    ];
    assert_eq!(payloads(&emitted), expected);
    assert_eq!(reload_payloads(&emitted), expected);
}

#[test]
fn invalid_or_cleared_subagent_update_sources_settle_as_parent_tools() {
    for invalid in [
        Value::Null,
        json!({}),
        json!({"sessionId": " ", "taskId": "task-1"}),
        json!({"sessionId": "main", "taskId": " "}),
        json!({"sessionId": "main", "taskId": "task-1", "toolCallId": 42}),
        json!({"sessionId": "main", "taskId": "task-1", "agentType": false}),
    ] {
        let mut assembler = HistoryAssembler::new(0);
        assembler.push_update(&update(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "tool",
            "title": "Work",
            "status": "in_progress",
            "_meta": {
                "_subagent/update": {
                    "sessionId": "main",
                    "taskId": "task-1",
                }
            },
        })));
        assembler.push_update(&update(json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "tool",
            "_meta": { "_subagent/update": invalid },
        })));
        let expected = update(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "tool",
            "title": "Work",
            "status": "completed",
            "_meta": { "_subagent/update": invalid, "ora/turn-end-settled": true },
        }));
        let records = assembler.end_turn(StopReason::EndTurn, None, None);
        assert_eq!(payloads(&records), vec![expected.clone()]);
        assert_eq!(reload_payloads(&records), vec![expected]);
    }
}

#[test]
fn turn_end_settled_parent_tool_carries_implied_marker_while_provider_completion_strips_spoofs() {
    let mut assembler = HistoryAssembler::new(0);
    // A provider-reported completion strips any spoofed settlement marker, while an `in_progress`
    // parent call that streamed output and was never finished gets `ora/turn-end-settled: true`
    // when `end_turn` closes the turn.
    let mut records = assembler.push_update(&update(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "parent-reported",
        "title": "Task",
        "status": "completed",
        "rawOutput": "final output",
        "_meta": { "ora/turn-end-settled": true },
    })));
    records.extend(assembler.push_update(&update(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "parent-implied",
        "title": "Task",
        "status": "in_progress",
        "rawOutput": "partial output",
    }))));
    records.extend(assembler.end_turn(StopReason::EndTurn, None, None));
    let expected = vec![
        update(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "parent-reported",
            "title": "Task",
            "status": "completed",
            "rawOutput": "final output",
        })),
        update(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "parent-implied",
            "title": "Task",
            "status": "completed",
            "rawOutput": "partial output",
            "_meta": { "ora/turn-end-settled": true },
        })),
    ];
    assert_eq!(payloads(&records), expected);
    assert_eq!(reload_payloads(&records), expected);
}

#[test]
fn subagent_update_plans_replace_per_task_despite_extra_metadata_through_reload() {
    let mut assembler = HistoryAssembler::new(0);
    let plan = |task_id: &str, step: &str, rev: u64| {
        update(json!({
            "sessionUpdate": "plan",
            "entries": [{ "content": step, "priority": "medium", "status": "pending" }],
            "_meta": {
                "_subagent/update": {
                    "sessionId": "main",
                    "taskId": task_id,
                    "toolCallId": format!("parent-{task_id}"),
                },
                "codeAgentCLI": { "rev": rev },
            },
        }))
    };
    let sibling = plan("task-b", "Sibling step", 1);
    let latest_a = plan("task-a", "Step 2", 2);
    assembler.push_update(&plan("task-a", "Step 1", 1));
    assembler.push_update(&sibling);
    assembler.push_update(&latest_a);
    let expected = vec![latest_a, sibling];
    assert_eq!(payloads(&assembler.pending_records()), expected);
    let records = assembler.end_turn(StopReason::EndTurn, None, None);
    assert_eq!(payloads(&records), expected);
    assert_eq!(reload_payloads(&records), expected);
}
