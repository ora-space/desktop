//! Serves a session's conversation from Ora's own record, with no provider attached.

use super::CONTRACT_QUEUE_CAPACITY;
use super::replay::recorded_replay;
use super::stream::SessionEventStream;
use ora_contracts::LoadSessionEvent;
use ora_history::read_session_history;
use ora_logging::ora_warn;
use std::path::Path;
use tokio::sync::mpsc;

/// Why a detached load could not be served, in the words the recorder would have used.
///
/// The reason travels out rather than only into the log: a record no reader can see must not be
/// appended to either, so the caller degrades the session's history state with this same text.
pub(super) struct UnreadableHistory {
    pub(super) reason: String,
}

/// Streams one session's durable transcript to a client that opened it.
///
/// The conversation belongs to Ora rather than to the agent that produced it, so reading one asks
/// nothing of the agent: a session whose plugin was uninstalled, or whose CLI cannot start, still
/// opens. Nothing is registered and no lifecycle state changes — the provider is attached by the
/// next prompt, which is the first moment one is actually needed.
///
/// Used only for sessions with no live actor. An actor knows its own durable cutoff and the
/// records of a turn still in flight, so it answers its own loads and hands off from disk to live
/// without a gap.
pub(super) fn detached_replay(
    sessions_root: &Path,
    session_id: &str,
) -> Result<SessionEventStream<LoadSessionEvent>, UnreadableHistory> {
    let history = read_session_history(sessions_root, session_id).map_err(|error| {
        // Load is how a user asks to see the conversation, so a history that cannot be read is
        // reported rather than shown as an empty one, which would state that nothing was ever said.
        ora_warn!(
            session_id = %session_id,
            error = %error,
            "session history unreadable during load",
        );
        UnreadableHistory {
            reason: error.to_string(),
        }
    })?;
    let (events, receiver) = mpsc::channel(CONTRACT_QUEUE_CAPACITY);
    // A long history is far larger than the event queue, so the replay is driven by its own task
    // and lets `send` apply backpressure. A client that stops listening drops the receiver, which
    // ends the task at its next send rather than leaving it to finish into nothing.
    tokio::spawn(async move {
        for event in recorded_replay(history).chain(std::iter::once(LoadSessionEvent::Completed)) {
            if events.send(Ok(event)).await.is_err() {
                return;
            }
        }
    });
    Ok(SessionEventStream::with_cleanup(receiver, || {}))
}
