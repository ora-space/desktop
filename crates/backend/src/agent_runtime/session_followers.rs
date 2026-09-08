use super::replay::replay_prefix;
use super::support::runtime_internal;
use crate::BackendError;
use agent_client_protocol_schema::v1::{SessionUpdate, StopReason};
use ora_contracts::LoadSessionEvent;
use ora_history::{AssembledRecord, read_session_history_up_to};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

/// Extends a loaded session conversation with the updates from its active prompt.
///
/// The prompt keeps a single owner regardless of who started it. Loading the same session while
/// that prompt runs adds a follower which receives later events but cannot cancel or restart the
/// turn, matching the behavior of reopening any ordinary running conversation.
pub(super) struct SessionFollowers {
    followers: HashMap<u64, SessionFollower>,
}

struct SessionFollower {
    events: mpsc::Sender<Result<LoadSessionEvent, BackendError>>,
    overflow: mpsc::UnboundedSender<()>,
}

/// Separates live fan-out from the contract queue that may still contain the recorded history.
const FOLLOWER_QUEUE_CAPACITY: usize = 256;

impl SessionFollowers {
    /// Creates an empty follower set for one prompt operation.
    pub(super) fn new() -> Self {
        Self {
            followers: HashMap::new(),
        }
    }

    /// Continues a loaded conversation after its recorded history and in-progress pending records
    /// are delivered.
    ///
    /// The relay is the only writer to the contract sender: it streams the replay prefix (durable
    /// history up to `cutoff`, merged with `pending` by position) before draining live events, so
    /// a load hands off from disk to live without a gap, a duplicate, or an ordering violation.
    pub(super) fn insert(
        &mut self,
        operation_id: u64,
        contract_sender: mpsc::Sender<Result<LoadSessionEvent, BackendError>>,
        sessions_root: PathBuf,
        session_id: String,
        cutoff: u64,
        pending: Vec<AssembledRecord>,
    ) {
        let (events, mut event_receiver) = mpsc::channel(FOLLOWER_QUEUE_CAPACITY);
        // Overflow travels independently from the bounded event queue so a slow view receives an
        // explicit failure instead of an ambiguous end-of-stream after it falls behind.
        let (overflow, mut overflow_receiver) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            if !send_replay_prefix(
                &contract_sender,
                &sessions_root,
                &session_id,
                cutoff,
                pending,
            )
            .await
            {
                return;
            }
            let mut overflow_open = true;
            loop {
                tokio::select! {
                    signal = overflow_receiver.recv(), if overflow_open => {
                        match signal {
                            Some(()) => {
                                let _ = contract_sender
                                    .send(Err(runtime_internal(
                                        "session_follower_overflow",
                                        "session load follower fell behind the active prompt",
                                    )))
                                    .await;
                                break;
                            }
                            None => overflow_open = false,
                        }
                    }
                    event = event_receiver.recv() => {
                        let Some(event) = event else {
                            break;
                        };
                        if contract_sender.send(event).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });
        self.followers
            .insert(operation_id, SessionFollower { events, overflow });
    }

    /// Detaches a closed view without affecting the prompt that owns the turn.
    pub(super) fn remove(&mut self, operation_id: u64) -> bool {
        self.followers.remove(&operation_id).is_some()
    }

    /// Mirrors one provider update to every view that still has the session open.
    pub(super) fn send_update(&mut self, update: &SessionUpdate) {
        self.followers.retain(|_, follower| {
            let event = LoadSessionEvent::session_update(update.clone());
            match follower.events.try_send(Ok(event)) {
                Ok(()) => true,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    let _ = follower.overflow.send(());
                    false
                }
                Err(mpsc::error::TrySendError::Closed(_)) => false,
            }
        });
    }

    /// Closes every loaded conversation stream with the active turn's recorded boundary.
    pub(super) fn finish(&mut self, stop_reason: StopReason) {
        for follower in std::mem::take(&mut self.followers).into_values() {
            tokio::spawn(async move {
                if follower
                    .events
                    .send(Ok(LoadSessionEvent::turn_ended(stop_reason)))
                    .await
                    .is_ok()
                {
                    let _ = follower.events.send(Ok(LoadSessionEvent::Completed)).await;
                }
            });
        }
    }
}

/// Streams the replay prefix — durable history up to `cutoff` merged with the in-progress pending
/// records — to the contract sender. Returns false when the consumer went away or the history
/// could not be read, so the caller stops without touching the live queue.
async fn send_replay_prefix(
    contract_sender: &mpsc::Sender<Result<LoadSessionEvent, BackendError>>,
    sessions_root: &Path,
    session_id: &str,
    cutoff: u64,
    pending: Vec<AssembledRecord>,
) -> bool {
    // The read is blocking filesystem I/O, so it runs off the relay's worker.
    let root = sessions_root.to_path_buf();
    let id = session_id.to_string();
    let history =
        match tokio::task::spawn_blocking(move || read_session_history_up_to(&root, &id, cutoff))
            .await
        {
            Ok(Ok(history)) => history,
            Ok(Err(_)) | Err(_) => {
                let _ = contract_sender
                    .send(Err(runtime_internal(
                        "session_history_unreadable",
                        "session history could not be read",
                    )))
                    .await;
                return false;
            }
        };
    for event in replay_prefix(history, pending) {
        if contract_sender.send(Ok(event)).await.is_err() {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::{FOLLOWER_QUEUE_CAPACITY, SessionFollowers};
    use agent_client_protocol_schema::v1::{SessionUpdate, StopReason};
    use ora_contracts::LoadSessionEvent;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::path::PathBuf;
    use tokio::sync::mpsc;

    /// Registers a follower with an empty replay prefix (no durable history or pending records),
    /// matching a load against a session that is not actively generating.
    fn insert_idle(
        followers: &mut SessionFollowers,
        operation_id: u64,
        sender: mpsc::Sender<Result<LoadSessionEvent, crate::BackendError>>,
    ) {
        followers.insert(
            operation_id,
            sender,
            PathBuf::new(),
            "session-1".to_string(),
            /*cutoff*/ 0,
            /*pending*/ Vec::new(),
        );
    }

    /// A loaded session receives live updates and the active turn's finite ending.
    #[tokio::test]
    async fn continues_a_loaded_session_through_prompt_completion() {
        let (sender, mut receiver) = mpsc::channel(4);
        let mut followers = SessionFollowers::new();
        insert_idle(&mut followers, 7, sender);
        let parsed: Result<SessionUpdate, _> = serde_json::from_value(json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": "hello" }
        }));
        let update = match parsed {
            Ok(update) => update,
            Err(error) => panic!("valid session update: {error}"),
        };

        followers.send_update(&update);
        followers.finish(StopReason::EndTurn);

        assert_eq!(
            [
                receiver
                    .recv()
                    .await
                    .map(|result| result.map_err(|error| error.to_string())),
                receiver
                    .recv()
                    .await
                    .map(|result| result.map_err(|error| error.to_string())),
                receiver
                    .recv()
                    .await
                    .map(|result| result.map_err(|error| error.to_string())),
            ],
            [
                Some(Ok(LoadSessionEvent::session_update(update))),
                Some(Ok(LoadSessionEvent::turn_ended(StopReason::EndTurn))),
                Some(Ok(LoadSessionEvent::Completed)),
            ],
        );
    }

    /// Closing one view removes only that view's session stream.
    #[tokio::test]
    async fn removes_one_loaded_view_without_touching_others() {
        let (first, _first_receiver) = mpsc::channel(1);
        let (second, _second_receiver) = mpsc::channel(1);
        let mut followers = SessionFollowers::new();
        insert_idle(&mut followers, 1, first);
        insert_idle(&mut followers, 2, second);

        assert!(followers.remove(1));
        assert!(!followers.remove(1));
        assert!(followers.remove(2));
    }

    /// A view that cannot keep up gets a diagnostic terminal error without slowing the prompt.
    #[tokio::test]
    async fn reports_follower_queue_overflow_explicitly() {
        let (sender, mut receiver) = mpsc::channel(1);
        let mut followers = SessionFollowers::new();
        insert_idle(&mut followers, 1, sender);
        let parsed: Result<SessionUpdate, _> = serde_json::from_value(json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": "hello" }
        }));
        let update = match parsed {
            Ok(update) => update,
            Err(error) => panic!("valid session update: {error}"),
        };

        for _ in 0..=FOLLOWER_QUEUE_CAPACITY {
            followers.send_update(&update);
        }

        let terminal_error = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while let Some(event) = receiver.recv().await {
                if let Err(error) = event {
                    return Some(error.to_string());
                }
            }
            None
        })
        .await;

        assert_eq!(
            terminal_error,
            Ok(Some(
                "session load follower fell behind the active prompt".to_string()
            )),
        );
    }
}
