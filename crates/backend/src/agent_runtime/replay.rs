use ora_contracts::{LoadSessionEvent, SessionHistoryNotice};
use ora_history::{HistoryIntegrity, HistoryRecord, SessionHistory};

/// Converts restored durable history into the finite event stream consumed by load clients.
pub(super) fn recorded_replay(history: SessionHistory) -> impl Iterator<Item = LoadSessionEvent> {
    let integrity_notice = match history.integrity {
        HistoryIntegrity::Complete => None,
        HistoryIntegrity::Damaged { unreadable_lines } => Some(LoadSessionEvent::HistoryNotice {
            notice: SessionHistoryNotice::UnreadableRecords {
                count: u32::try_from(unreadable_lines.get()).unwrap_or(u32::MAX),
            },
        }),
    };

    integrity_notice
        .into_iter()
        .chain(history.lines.into_iter().filter_map(|line| {
            match line.record {
                HistoryRecord::Update { update } => {
                    Some(LoadSessionEvent::SessionUpdate { update: *update })
                }
                HistoryRecord::TurnEnded { stop_reason } => {
                    Some(LoadSessionEvent::TurnEnded { stop_reason })
                }
                HistoryRecord::Gap { reason } => Some(LoadSessionEvent::HistoryNotice {
                    notice: SessionHistoryNotice::UnrecordedContent { reason },
                }),
                // These records govern persistence and provider handoff rather
                // than the conversation view, so replay keeps them on disk only.
                HistoryRecord::Meta(_)
                | HistoryRecord::AgentSwitched(_)
                | HistoryRecord::HandoffDelivered { .. } => None,
            }
        }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol_schema::v1::StopReason;
    use ora_history::HistoryLine;
    use pretty_assertions::assert_eq;
    use std::num::NonZeroUsize;

    /// Creates a history line whose timestamp is irrelevant to replay mapping.
    fn line(seq: u32, record: HistoryRecord) -> HistoryLine {
        HistoryLine::new("2026-08-14T10:00:00+08:00", seq, record)
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
}
