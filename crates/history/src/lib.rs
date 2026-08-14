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
mod path;
mod reader;
mod record;
mod writer;

#[cfg(test)]
mod assembler_tests;
#[cfg(test)]
mod handoff_tests;
#[cfg(test)]
mod store_tests;

pub use assembler::{AssembledRecord, HistoryAssembler};
pub use clock::{FixedHistoryClock, HistoryClock};
pub use error::HistoryError;
pub use handoff::{binding_needs_handoff, render_handoff};
pub use path::history_path;
pub use reader::{HistoryIntegrity, SessionHistory, read_session_history};
pub use record::{AgentSwitch, HistoryLine, HistoryRecord, SCHEMA_VERSION, SessionMeta};
pub use writer::{HistoryWriter, remove_session_history};
