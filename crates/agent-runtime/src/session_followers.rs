use super::replay::replay_prefix;
use super::support::{session_event_overflow, session_history_unreadable};
use crate::RuntimeError;
use agent_client_protocol_schema::v1::{SessionUpdate, StopReason};
use ora_contracts::{LoadSessionEvent, ToolCallTiming};
use ora_history::{AssembledRecord, read_session_history_up_to};
use ora_logging::ora_warn;
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

/// Keeps delivery and cleanup confirmation owned by the same relay task.
pub(super) struct FollowerOutput {
    pub(super) events: mpsc::Sender<Result<LoadSessionEvent, RuntimeError>>,
    pub(super) completion: tokio::sync::oneshot::Sender<Result<(), RuntimeError>>,
}

struct SessionFollower {
    events: mpsc::Sender<Result<LoadSessionEvent, RuntimeError>>,
    overflow: mpsc::UnboundedSender<()>,
    worker: tokio::task::JoinHandle<()>,
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
        output: FollowerOutput,
        sessions_root: PathBuf,
        session_id: String,
        cutoff: u64,
        pending: Vec<AssembledRecord>,
    ) {
        let FollowerOutput {
            events: contract_sender,
            completion,
        } = output;
        let (events, mut event_receiver) = mpsc::channel(FOLLOWER_QUEUE_CAPACITY);
        // Overflow travels independently from the bounded event queue so a slow view receives an
        // explicit failure instead of an ambiguous end-of-stream after it falls behind.
        let (overflow, mut overflow_receiver) = mpsc::unbounded_channel();
        let worker = tokio::spawn(async move {
            async {
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
                        // Both senders close on drop. A pending overflow must win over event EOF.
                        biased;
                        signal = overflow_receiver.recv(), if overflow_open => {
                            match signal {
                                Some(()) => {
                                    let _ = contract_sender
                                        .send(Err(session_event_overflow(
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
            }
            .await;
            let _ = completion.send(Ok(()));
        });
        self.followers.insert(
            operation_id,
            SessionFollower {
                events,
                overflow,
                worker,
            },
        );
    }

    /// Detaches a closed view without affecting the prompt that owns the turn.
    #[cfg(test)]
    pub(super) fn remove(&mut self, operation_id: u64) -> bool {
        self.followers.remove(&operation_id).is_some()
    }

    /// Identifies a follower before transferring its worker into asynchronous cleanup.
    pub(super) fn contains(&self, operation_id: u64) -> bool {
        self.followers.contains_key(&operation_id)
    }

    /// Confirms relay termination after removing this view from the prompt's fan-out.
    pub(super) fn remove_and_wait(
        &mut self,
        operation_id: u64,
    ) -> impl std::future::Future<Output = Result<(), RuntimeError>> + Send + 'static {
        let follower = self.followers.remove(&operation_id);
        async move {
            if let Some(SessionFollower {
                events,
                overflow,
                worker,
            }) = follower
            {
                drop(events);
                drop(overflow);
                worker.await.map_err(|error| {
                    RuntimeError::internal("load follower cleanup failed", error)
                })?;
            }
            Ok(())
        }
    }

    /// Mirrors one provider update to every view that still has the session open.
    pub(super) fn send_update(&mut self, update: &SessionUpdate, timing: Option<ToolCallTiming>) {
        self.followers.retain(|_, follower| {
            let event = LoadSessionEvent::SessionUpdate {
                update: update.clone(),
                recorded_at: None,
                tool_timing: timing.clone(),
            };
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
        self.finish_with_result(stop_reason, Ok(LoadSessionEvent::Completed));
    }

    /// Deliver a terminal reason after the recorded turn boundary, including to replaying views.
    pub(super) fn finish_with_result(
        &mut self,
        stop_reason: StopReason,
        result: Result<LoadSessionEvent, RuntimeError>,
    ) {
        for follower in std::mem::take(&mut self.followers).into_values() {
            let result = result.clone();
            tokio::spawn(async move {
                if follower
                    .events
                    .send(Ok(LoadSessionEvent::turn_ended(stop_reason)))
                    .await
                    .is_ok()
                {
                    let _ = follower.events.send(result).await;
                }
            });
        }
    }
}

impl Drop for SessionFollowers {
    /// An attempt that ends without handing its followers to the retry must still close them.
    ///
    /// The explicit `finish` calls remain the normal path; this guard covers every early return so
    /// a follower cannot silently observe EOF instead of a terminal boundary. A non-empty set at
    /// drop time means an explicit `finish` was missed, so warn instead of hiding that defect.
    fn drop(&mut self) {
        if self.followers.is_empty() {
            return;
        }
        ora_warn!(
            follower_count = self.followers.len(),
            "closing prompt followers without an explicit finish"
        );
        for follower in std::mem::take(&mut self.followers).into_values() {
            match follower.events.try_reserve_many(2) {
                Ok(mut permits) => {
                    if let Some(permit) = permits.next() {
                        permit.send(Ok(LoadSessionEvent::turn_ended(StopReason::Cancelled)));
                    }
                    if let Some(permit) = permits.next() {
                        permit.send(Ok(LoadSessionEvent::Completed));
                    }
                }
                Err(_) => {
                    let _ = follower.overflow.send(());
                }
            }
        }
    }
}

/// Streams the replay prefix — durable history up to `cutoff` merged with the in-progress pending
/// records — to the contract sender. Returns false when the consumer went away or the history
/// could not be read, so the caller stops without touching the live queue.
async fn send_replay_prefix(
    contract_sender: &mpsc::Sender<Result<LoadSessionEvent, RuntimeError>>,
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
                    .send(Err(session_history_unreadable()))
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
        sender: mpsc::Sender<Result<LoadSessionEvent, crate::RuntimeError>>,
    ) {
        followers.insert(
            operation_id,
            super::FollowerOutput {
                events: sender,
                completion: tokio::sync::oneshot::channel().0,
            },
            PathBuf::new(),
            "session-1".to_string(),
            /*cutoff*/ 0,
            /*pending*/ Vec::new(),
        );
    }

    /// A dropped attempt still closes its followers with a terminal boundary.
    #[tokio::test]
    async fn dropping_unfinished_followers_ends_them_cancelled() {
        let (sender, mut receiver) = mpsc::channel(4);
        let mut followers = SessionFollowers::new();
        insert_idle(&mut followers, 1, sender);
        drop(followers);

        let turn_ended = receiver
            .recv()
            .await
            .map(|result| result.map_err(|error| error.to_string()));
        let completed = receiver
            .recv()
            .await
            .map(|result| result.map_err(|error| error.to_string()));
        assert_eq!(
            [turn_ended, completed],
            [
                Some(Ok(LoadSessionEvent::turn_ended(StopReason::Cancelled))),
                Some(Ok(LoadSessionEvent::Completed)),
            ]
        );
    }

    /// A single free slot cannot represent the complete two-event terminal sequence.
    #[tokio::test]
    async fn dropping_with_one_free_slot_reports_overflow_without_a_partial_terminal() {
        let (sender, mut receiver) = mpsc::channel(2);
        let mut followers = SessionFollowers::new();
        insert_idle(&mut followers, 1, sender);
        // Holding permits keeps exactly one slot free without depending on relay scheduling.
        let queue = followers.followers[&1].events.clone();
        let held = queue.try_reserve_many(FOLLOWER_QUEUE_CAPACITY - 1).unwrap();
        drop(followers);
        drop(held);
        drop(queue);
        assert_eq!(
            receiver.recv().await.unwrap().unwrap_err().to_string(),
            "session load follower fell behind the active prompt"
        );
        assert!(receiver.recv().await.is_none());
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

        followers.send_update(&update, None);
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
            followers.send_update(&update, None);
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
