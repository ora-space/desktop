use agent_client_protocol_schema::v1::SessionUpdate;
use agent_client_protocol_schema::v1::StopReason;
use ora_domain::AgentRef;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Identifies the on-disk record schema so a later format change stays detectable.
pub const SCHEMA_VERSION: u32 = 1;

/// One complete line of a session history file.
///
/// `seq` is assigned when an item first appears in the conversation, not when it
/// is written. Items are appended as soon as they settle, and a tool call that
/// started early can settle after a message that started later, so write order
/// alone does not reproduce the timeline. Readers restore it by sorting on `seq`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryLine {
    /// Local wall-clock time this line was appended, RFC 3339 with UTC offset.
    pub at: String,
    pub seq: u32,
    #[serde(flatten)]
    pub record: HistoryRecord,
}

impl HistoryLine {
    /// Pairs one record with the position and time it should be replayed at.
    pub fn new(at: impl Into<String>, seq: u32, record: HistoryRecord) -> Self {
        Self {
            at: at.into(),
            seq,
            record,
        }
    }
}

/// Names everything a session history file can hold.
///
/// `Update` carries assembled ACP updates rather than the raw streamed chunks:
/// one update per settled message, thought, tool call, or plan. Replaying them
/// therefore reproduces the conversation without any chunk merging on the read
/// side. The remaining variants cover state ACP has no vocabulary for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum HistoryRecord {
    /// Opens the file and pins the schema and provider binding it started with.
    Meta(SessionMeta),
    /// One settled conversation item.
    Update { update: Box<SessionUpdate> },
    /// Closes one prompt turn with the provider's typed stop reason.
    ///
    /// Without this a replayed turn cannot be told apart from a completed one,
    /// which is exactly the information provider replay never carried.
    TurnEnded { stop_reason: StopReason },
    /// Records that the conversation moved to a different agent CLI.
    AgentSwitched(AgentSwitch),
    /// Records that the agent bound by the preceding switch was given the record.
    ///
    /// Written only once the provider accepted the prompt carrying the
    /// transcript, which is what separates it from the user turn recorded just
    /// before that prompt was sent. Ora records a prompt before sending it, so a
    /// user turn following a switch proves only that Ora meant to hand the
    /// transcript over — this proves it did.
    HandoffDelivered { agent_session_id: String },
    /// Marks a discontinuity left by a failed write, so a hole is never silent.
    Gap { reason: String },
}

/// Opens a history file with the identity and binding the session started with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMeta {
    pub schema_version: u32,
    pub session_id: String,
    pub task_id: String,
    pub agent_ref: AgentRef,
    pub agent_session_id: String,
    pub cwd: PathBuf,
}

/// Rebinds the conversation onto a new provider session on a different agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSwitch {
    pub from: AgentRef,
    pub to: AgentRef,
    /// The provider session the conversation continues on after the switch.
    pub agent_session_id: String,
}
