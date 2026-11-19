//! What one turn's traffic left open, and the fence that keeps a stopped turn out of later ones.
//!
//! A turn that ends by cancel or failure may be followed by another on the same provider session
//! only when it left nothing running (D2 quiescence condition 2), and any later update that names
//! one of its tool calls or child sources is a leak (cross-turn detection). Both facts come from
//! the settled history lines the Thread mirror already sees, so they are kept beside it: a leaked
//! line is refused before it becomes a Thread event of the later turn.

use agent_client_protocol_schema::v1::{SessionUpdate, ToolCallStatus};
use ora_history::{HistoryLine, HistoryRecord, child_source_id};
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, PoisonError};

/// Turn traffic bookkeeping shared by the Thread mirror and the conversation.
#[derive(Debug, Default)]
pub(super) struct TurnFence {
    state: Mutex<FenceState>,
}

#[derive(Debug, Default)]
struct FenceState {
    /// Tool calls of the current turn and whether each reached `completed` or `failed`.
    tools: HashMap<String, bool>,
    /// Child (subagent) sources the current turn projected.
    children: HashSet<String>,
    fenced_tools: HashSet<String>,
    fenced_children: HashSet<String>,
    leaked: bool,
}

/// Whether a settled line may join the current turn.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum Admission {
    Record,
    /// The line names a stopped turn's tool call or child source; it joins no turn.
    Leak,
}

impl TurnFence {
    /// Starts tracking a new turn; fenced identities of earlier turns stay fenced for the session.
    pub(super) fn begin_turn(&self) {
        let mut state = self.lock();
        state.tools.clear();
        state.children.clear();
    }

    /// Classifies one settled line and, when it belongs to the current turn, tracks it.
    pub(super) fn observe(&self, line: &HistoryLine) -> Admission {
        let HistoryRecord::Update { update, .. } = &line.record else {
            return Admission::Record;
        };
        let (tool, child) = identities(update);
        let mut state = self.lock();
        if tool.is_some_and(|(id, _)| state.fenced_tools.contains(id))
            || child.is_some_and(|id| state.fenced_children.contains(id))
        {
            state.leaked = true;
            return Admission::Leak;
        }
        if let Some((id, status)) = tool {
            let terminal = state.tools.entry(id.to_owned()).or_default();
            if let Some(status) = status {
                *terminal = matches!(status, ToolCallStatus::Completed | ToolCallStatus::Failed);
            }
        }
        if let Some(child) = child {
            state.children.insert(child.to_owned());
        }
        Admission::Record
    }

    /// D2 quiescence condition 2: every tool call of the turn is terminal and no child ran.
    pub(super) fn turn_settled(&self) -> bool {
        let state = self.lock();
        state.children.is_empty() && state.tools.values().all(|terminal| *terminal)
    }

    /// Fences the current turn's identities for the rest of the session.
    pub(super) fn fence_turn(&self) {
        let mut state = self.lock();
        let FenceState {
            tools,
            children,
            fenced_tools,
            fenced_children,
            ..
        } = &mut *state;
        fenced_tools.extend(tools.keys().cloned());
        fenced_children.extend(children.iter().cloned());
    }

    /// Whether a line naming a fenced identity was refused.
    pub(super) fn leaked(&self) -> bool {
        self.lock().leaked
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FenceState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The tool call (with its new status, if the update carries one) and child source an update
/// names.
fn identities(update: &SessionUpdate) -> (Option<(&str, Option<ToolCallStatus>)>, Option<&str>) {
    match update {
        SessionUpdate::ToolCall(call) => (
            Some((call.tool_call_id.0.as_ref(), Some(call.status))),
            child_source_id(call.meta.as_ref()),
        ),
        SessionUpdate::ToolCallUpdate(change) => (
            Some((change.tool_call_id.0.as_ref(), change.fields.status)),
            child_source_id(change.meta.as_ref()),
        ),
        SessionUpdate::AgentMessageChunk(chunk)
        | SessionUpdate::AgentThoughtChunk(chunk)
        | SessionUpdate::UserMessageChunk(chunk) => (None, child_source_id(chunk.meta.as_ref())),
        SessionUpdate::Plan(plan) => (None, child_source_id(plan.meta.as_ref())),
        _ => (None, None),
    }
}

#[cfg(test)]
mod tests;
