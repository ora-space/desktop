//! Serialization compatibility of the `TurnEnded` attribution fields.
//!
//! Evidence for specs/test-cases/node/protocol/turn-failure-reason.md case 10.
use crate::record::{HistoryRecord, TurnEndedBy, TurnFailure, TurnFailureCode};
use agent_client_protocol_schema::v1::StopReason;
use pretty_assertions::assert_eq;

/// A turn without attribution writes exactly the shape written before the fields existed.
#[test]
fn turn_ended_without_attribution_matches_the_previous_shape() {
    let record = HistoryRecord::TurnEnded {
        stop_reason: StopReason::EndTurn,
        ended_by: None,
        failure: None,
    };
    let value = serde_json::to_value(&record).unwrap();
    assert!(!value.get("ended_by").is_some());
    assert!(!value.get("failure").is_some());
    assert_eq!(
        value,
        serde_json::json!({"type": "turnEnded", "stop_reason": "end_turn"})
    );
}

/// A record written before the fields existed decodes without attribution.
#[test]
fn old_turn_ended_decodes_without_attribution() {
    let record: HistoryRecord = serde_json::from_value(serde_json::json!({
        "type": "turnEnded",
        "stop_reason": "cancelled"
    }))
    .unwrap();
    assert_eq!(
        record,
        HistoryRecord::TurnEnded {
            stop_reason: StopReason::Cancelled,
            ended_by: None,
            failure: None,
        }
    );
}

/// The attribution fields serialize with their exact wire names.
#[test]
fn attribution_fields_serialize_with_their_wire_names() {
    for (ended_by, name) in [
        (TurnEndedBy::CallerCancel, "caller_cancel"),
        (TurnEndedBy::SessionEnd, "session_end"),
    ] {
        let value = serde_json::to_value(HistoryRecord::TurnEnded {
            stop_reason: StopReason::Cancelled,
            ended_by: Some(ended_by),
            failure: None,
        })
        .unwrap();
        assert_eq!(
            value,
            serde_json::json!({"type": "turnEnded", "stop_reason": "cancelled", "ended_by": name})
        );
    }
}

/// A reader folds an unknown failure code into `internal` and ignores unknown failure fields.
#[test]
fn a_reader_folds_unknown_failure_codes_into_internal() {
    let record: HistoryRecord = serde_json::from_value(serde_json::json!({
        "type": "turnEnded",
        "stop_reason": "cancelled",
        "failure": {
            "code": "some_future_code",
            "acp_code": -32000,
            "future_field": true
        }
    }))
    .unwrap();
    assert_eq!(
        record,
        HistoryRecord::TurnEnded {
            stop_reason: StopReason::Cancelled,
            ended_by: None,
            failure: Some(TurnFailure {
                code: TurnFailureCode::Internal,
                acp_code: Some(-32000),
                message: None,
                message_truncated: None,
                attempts: None,
                answer: None,
            }),
        }
    );
}
