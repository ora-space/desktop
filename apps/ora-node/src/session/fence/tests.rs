use super::*;
use agent_client_protocol_schema::v1::{ToolCall, ToolCallUpdate, ToolCallUpdateFields};
use pretty_assertions::assert_eq;

/// Builds one settled update line.
fn line(update: SessionUpdate) -> HistoryLine {
    HistoryLine::new(
        "2026-11-19T10:00:00.000+08:00",
        1,
        HistoryRecord::Update {
            update: Box::new(update),
            tool_timing: None,
        },
    )
}

/// A tool call still running when the turn ends leaves the turn unsettled.
#[test]
fn a_running_tool_leaves_the_turn_unsettled() {
    let fence = TurnFence::default();
    fence.begin_turn();
    assert_eq!(
        fence.observe(&line(SessionUpdate::ToolCall(
            ToolCall::new("tool-1", "Shell").status(ToolCallStatus::InProgress)
        ))),
        Admission::Record
    );
    assert!(!fence.turn_settled());
    fence.observe(&line(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
        "tool-1",
        ToolCallUpdateFields::new().status(ToolCallStatus::Completed),
    ))));
    assert!(fence.turn_settled());
}

/// A child source in the turn means the turn cannot be proven quiet.
#[test]
fn a_child_projection_leaves_the_turn_unsettled() {
    let fence = TurnFence::default();
    fence.begin_turn();
    let mut meta = serde_json::Map::new();
    meta.insert(
        "opencode/child-session".to_owned(),
        serde_json::json!({"id": "child-1", "parentID": "parent", "depth": 1}),
    );
    fence.observe(&line(SessionUpdate::ToolCall(
        ToolCall::new("tool-2", "Task")
            .status(ToolCallStatus::Completed)
            .meta(meta),
    )));
    assert!(!fence.turn_settled());
}

/// After a turn is fenced, a later turn's update naming its tool call is refused as a leak.
#[test]
fn a_fenced_tool_update_in_a_later_turn_is_a_leak() {
    let fence = TurnFence::default();
    fence.begin_turn();
    fence.observe(&line(SessionUpdate::ToolCall(
        ToolCall::new("tool-1", "Shell").status(ToolCallStatus::Completed),
    )));
    fence.fence_turn();
    fence.begin_turn();
    fence.begin_turn();
    assert!(!fence.leaked());
    assert_eq!(
        fence.observe(&line(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "tool-1",
            ToolCallUpdateFields::new(),
        )))),
        Admission::Leak
    );
    assert!(fence.leaked());
    assert_eq!(
        fence.observe(&line(SessionUpdate::ToolCall(ToolCall::new(
            "tool-3", "Read"
        )))),
        Admission::Record
    );
}
