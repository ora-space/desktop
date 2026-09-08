use ora_contracts::{LoadSessionEvent, SessionHistoryNotice};
use ora_history::{AssembledRecord, HistoryIntegrity, HistoryRecord, SessionHistory};

/// Renders a damage notice when durable history could not be fully restored.
fn integrity_notice(integrity: HistoryIntegrity) -> Option<LoadSessionEvent> {
    match integrity {
        HistoryIntegrity::Complete => None,
        HistoryIntegrity::Damaged { unreadable_lines } => Some(LoadSessionEvent::HistoryNotice {
            notice: SessionHistoryNotice::UnreadableRecords {
                count: u32::try_from(unreadable_lines.get()).unwrap_or(u32::MAX),
            },
        }),
    }
}

/// Maps one durable record onto the load event the client renders, dropping bookkeeping records
/// that only matter for persistence or provider handoff.
///
/// `recorded_at` is the history line's wall-clock stamp so replay can restore bubble times.
/// Pending in-memory records pass `None` because they have not been written yet.
fn map_record(record: HistoryRecord, recorded_at: Option<String>) -> Option<LoadSessionEvent> {
    match record {
        HistoryRecord::Update { update } => Some(LoadSessionEvent::SessionUpdate {
            update: *update,
            recorded_at,
        }),
        HistoryRecord::TurnEnded { stop_reason } => Some(LoadSessionEvent::TurnEnded {
            stop_reason,
            recorded_at,
        }),
        HistoryRecord::Gap { reason } => Some(LoadSessionEvent::HistoryNotice {
            notice: SessionHistoryNotice::UnrecordedContent { reason },
        }),
        // These records govern persistence and provider handoff rather than the conversation view,
        // so replay keeps them on disk only.
        HistoryRecord::Meta(_)
        | HistoryRecord::AgentSwitched(_)
        | HistoryRecord::HandoffDelivered { .. } => None,
    }
}

/// Converts restored durable history into the finite event stream consumed by load clients.
pub(super) fn recorded_replay(history: SessionHistory) -> impl Iterator<Item = LoadSessionEvent> {
    let notice = integrity_notice(history.integrity);
    notice.into_iter().chain(
        history
            .lines
            .into_iter()
            .filter_map(|line| map_record(line.record, Some(line.at))),
    )
}

/// Merges durable history and the actor's pending (not-yet-durable) records into one replay prefix,
/// ordered by conversation position, so a load hands off from disk to live without a gap or a
/// duplicate. Persisted and pending positions are disjoint, so a sort by `seq` is a merge.
pub(super) fn replay_prefix(
    history: SessionHistory,
    pending: Vec<AssembledRecord>,
) -> Vec<LoadSessionEvent> {
    let mut merged: Vec<(u32, HistoryRecord, Option<String>)> = history
        .lines
        .into_iter()
        .map(|line| (line.seq, line.record, Some(line.at)))
        .chain(
            pending
                .into_iter()
                .map(|record| (record.seq, record.record, None)),
        )
        .collect();
    merged.sort_by_key(|(seq, _, _)| *seq);
    let mut events: Vec<LoadSessionEvent> =
        integrity_notice(history.integrity).into_iter().collect();
    events.extend(
        merged
            .into_iter()
            .filter_map(|(_, record, recorded_at)| map_record(record, recorded_at)),
    );
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol_schema::v1::{SessionUpdate, StopReason};
    use ora_history::HistoryLine;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::num::NonZeroUsize;

    const RECORDED_AT: &str = "2026-08-14T10:00:00+08:00";

    /// Creates a history line stamped with the fixture time so replay must forward it.
    fn line(seq: u32, record: HistoryRecord) -> HistoryLine {
        HistoryLine::new(RECORDED_AT, seq, record)
    }

    #[test]
    fn reports_damage_before_surviving_history() {
        let history = SessionHistory {
            lines: vec![line(
                0,
                HistoryRecord::TurnEnded {
                    stop_reason: StopReason::EndTurn,
                },
            )],
            next_seq: 1,
            integrity: HistoryIntegrity::Damaged {
                unreadable_lines: NonZeroUsize::new(1).expect("non-zero damage count"),
            },
        };

        assert_eq!(
            recorded_replay(history).collect::<Vec<_>>(),
            vec![
                LoadSessionEvent::HistoryNotice {
                    notice: SessionHistoryNotice::UnreadableRecords { count: 1 },
                },
                LoadSessionEvent::TurnEnded {
                    stop_reason: StopReason::EndTurn,
                    recorded_at: Some(RECORDED_AT.to_string()),
                },
            ],
        );
    }

    #[test]
    fn surfaces_recorded_gaps_and_skips_bookkeeping() {
        let history = SessionHistory {
            lines: vec![
                line(
                    0,
                    HistoryRecord::Gap {
                        reason: "no space left on device".to_string(),
                    },
                ),
                line(
                    1,
                    HistoryRecord::HandoffDelivered {
                        agent_session_id: "provider-session-1".to_string(),
                    },
                ),
            ],
            next_seq: 2,
            integrity: HistoryIntegrity::Complete,
        };

        assert_eq!(
            recorded_replay(history).collect::<Vec<_>>(),
            vec![LoadSessionEvent::HistoryNotice {
                notice: SessionHistoryNotice::UnrecordedContent {
                    reason: "no space left on device".to_string(),
                },
            }],
        );
    }

    /// A pending in-progress text at a lower position merges ahead of a later-persisted record, so
    /// the replay prefix is ordered even though durable writes can land out of position.
    #[test]
    fn replay_prefix_orders_pending_ahead_of_later_persisted() {
        let update: SessionUpdate = serde_json::from_value(json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": "partial" }
        }))
        .expect("valid session update");

        let history = SessionHistory {
            lines: vec![line(
                1,
                HistoryRecord::TurnEnded {
                    stop_reason: StopReason::EndTurn,
                },
            )],
            next_seq: 2,
            integrity: HistoryIntegrity::Complete,
        };
        let pending = vec![AssembledRecord {
            seq: 0,
            record: HistoryRecord::Update {
                update: Box::new(update.clone()),
            },
        }];

        let events = replay_prefix(history, pending);

        assert_eq!(
            events,
            vec![
                LoadSessionEvent::SessionUpdate {
                    update,
                    recorded_at: None,
                },
                LoadSessionEvent::TurnEnded {
                    stop_reason: StopReason::EndTurn,
                    recorded_at: Some(RECORDED_AT.to_string()),
                },
            ]
        );
    }
}
