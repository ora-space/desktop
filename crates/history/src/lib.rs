//! Ora-owned session history: the durable record of what a conversation was.
//!
//! Ora is an ACP client, so the model context behind a session lives inside the
//! agent CLI that serves it. Owning the transcript separately is what lets a
//! conversation outlive one provider — it can be replayed without asking the
//! agent to recite it, and it can be handed to a different agent entirely.
//!
//! Nothing here performs ACP work or touches the database. The crate assembles
//! streamed updates into settled records, appends them, reads them back, and
//! renders them for another agent; deciding when to do any of that belongs to the
//! runtime that owns the session.

mod assembler;
mod clock;
mod error;
mod handoff;
mod metadata;
mod path;
mod reader;
mod record;
mod writer;

#[cfg(test)]
mod assembler_tests;
#[cfg(test)]
mod handoff_tests;
#[cfg(test)]
mod projection_tests;
#[cfg(test)]
mod store_tests;

pub use assembler::{AssembledRecord, HistoryAssembler};
pub use clock::{FixedHistoryClock, HistoryClock};
pub use error::HistoryError;
pub use handoff::{
    binding_needs_handoff, final_assistant_message, final_completed_assistant_turn, render_handoff,
};
pub use metadata::{
    OPENCODE_CHILD_SESSION_META_KEY, SUBAGENT_UPDATE_META_KEY, TURN_END_SETTLED_META_KEY,
    child_source_id, has_child_source_namespace,
};
pub use path::history_path;
pub use reader::{
    HistoryIntegrity, SessionHistory, read_session_history, read_session_history_up_to,
};
pub use record::{
    AgentSwitch, HistoryLine, HistoryRecord, SCHEMA_VERSION, SessionMeta, ToolCallTiming,
};
pub use writer::{HistoryWriter, remove_session_history};
