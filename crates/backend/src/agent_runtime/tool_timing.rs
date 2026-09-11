use agent_client_protocol_schema::v1::{SessionUpdate, ToolCallId, ToolCallStatus};
use ora_contracts::ToolCallTiming as ContractTiming;
use ora_history::ToolCallTiming as HistoryTiming;
use std::collections::HashMap;
use time::format_description::well_known::Rfc3339;
use tokio::time::Instant;

struct StartedTool {
    started_at: String,
    started: Instant,
}

/// Tracks tool lifecycles inside one prompt; callers create one tracker per session operation.
#[derive(Default)]
pub(super) struct ToolTimings {
    calls: HashMap<ToolCallId, StartedTool>,
}

impl ToolTimings {
    /// Observes a tool update and returns timing only when a real nonterminal start was seen.
    pub(super) fn observe(&mut self, update: &SessionUpdate) -> Option<ContractTiming> {
        let (id, status) = tool_identity(update)?;
        let terminal = matches!(status, Some(ToolCallStatus::Completed | ToolCallStatus::Failed));
        if !self.calls.contains_key(id) && !terminal {
            self.calls.insert(
                id.clone(),
                StartedTool {
                    started_at: ora_logging::clock::now_local()
                        .format(&Rfc3339)
                        .unwrap_or_default(),
                    started: Instant::now(),
                },
            );
        }
        let started = self.calls.get(id)?;
        Some(ContractTiming {
            started_at: started.started_at.clone(),
            duration_ms: terminal.then(|| elapsed_ms(started.started)),
        })
    }

    /// Freezes every observed tool so a turn boundary can persist stable durations.
    pub(super) fn finish_all(&self) -> Vec<(ToolCallId, HistoryTiming)> {
        self.calls
            .iter()
            .map(|(id, started)| {
                (
                    id.clone(),
                    HistoryTiming {
                        started_at: started.started_at.clone(),
                        duration_ms: Some(elapsed_ms(started.started)),
                    },
                )
            })
            .collect()
    }
}

/// Extracts the stable identity and optional status from either ACP tool update shape.
fn tool_identity(update: &SessionUpdate) -> Option<(&ToolCallId, Option<ToolCallStatus>)> {
    match update {
        SessionUpdate::ToolCall(call) => Some((&call.tool_call_id, Some(call.status))),
        SessionUpdate::ToolCallUpdate(update) => Some((&update.tool_call_id, update.fields.status)),
        _ => None,
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::ToolTimings;
    use agent_client_protocol_schema::v1::{SessionUpdate, ToolCall, ToolCallStatus};

    #[test]
    fn isolates_calls_and_does_not_invent_a_terminal_start() {
        ora_logging::initialize_test_clock();
        let mut timings = ToolTimings::default();
        let direct_terminal = SessionUpdate::ToolCall(
            ToolCall::new("done", "Done").status(ToolCallStatus::Completed),
        );
        assert_eq!(timings.observe(&direct_terminal), None);
        let running = SessionUpdate::ToolCall(
            ToolCall::new("run", "Run").status(ToolCallStatus::InProgress),
        );
        let started = timings.observe(&running).expect("timing starts");
        let completed = SessionUpdate::ToolCall(
            ToolCall::new("run", "Run").status(ToolCallStatus::Completed),
        );
        let finished = timings.observe(&completed).expect("timing finishes");
        assert_eq!(finished.started_at, started.started_at);
        assert!(finished.duration_ms.is_some());
    }
}
