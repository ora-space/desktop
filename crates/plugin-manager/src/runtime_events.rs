use crate::{
    CrashPolicy, PluginError, PluginEventHub, PluginManagerConfig, PluginRuntimeEvent,
    PluginRuntimeEventSink, PluginStateSnapshot, StateMutation, StateStore,
};
use ora_plugin_protocol::{ContentOwnerId, JsonSafeU64, PluginId};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, oneshot};

/// Cloneable critical-event port backed by a single ordered management actor.
#[derive(Debug, Clone)]
pub struct ManagementRuntimeEventSink {
    sender: mpsc::Sender<EventCommand>,
}

impl ManagementRuntimeEventSink {
    /// Starts an event actor that is the only owner of generation/sequence acceptance state.
    pub(crate) fn start(
        state: StateStore,
        config: &PluginManagerConfig,
        events: PluginEventHub,
    ) -> Self {
        let (sender, receiver) = mpsc::channel(256);
        tokio::spawn(run_event_actor(
            state,
            config.crash_window,
            config.crash_threshold,
            events,
            receiver,
        ));
        Self { sender }
    }
}

impl PluginRuntimeEventSink for ManagementRuntimeEventSink {
    async fn record(&self, event: PluginRuntimeEvent) -> Result<(), PluginError> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(EventCommand { event, reply })
            .await
            .map_err(|_| PluginError::BackendShuttingDown)?;
        response
            .await
            .unwrap_or(Err(PluginError::BackendShuttingDown))
    }
}

struct EventCommand {
    event: PluginRuntimeEvent,
    reply: oneshot::Sender<Result<(), PluginError>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EventCursor {
    content_owner: ContentOwnerId,
    generation: JsonSafeU64,
    sequence: JsonSafeU64,
}

/// Serializes critical lifecycle events and persists crash policy before acknowledging them.
async fn run_event_actor(
    state: StateStore,
    crash_window: std::time::Duration,
    crash_threshold: usize,
    events: PluginEventHub,
    mut receiver: mpsc::Receiver<EventCommand>,
) {
    let mut cursors = BTreeMap::new();
    while let Some(command) = receiver.recv().await {
        let event_for_observers = command.event.clone();
        let result = handle_event(
            &state,
            crash_window,
            crash_threshold,
            &mut cursors,
            command.event,
        )
        .await;
        let result = result.map(|accepted| {
            if accepted {
                events.publish_runtime(&event_for_observers);
            }
        });
        let _ = command.reply.send(result);
    }
}

/// Rejects current-owner protocol errors while harmlessly ignoring stale owner/generation events.
async fn handle_event(
    state: &StateStore,
    crash_window: std::time::Duration,
    crash_threshold: usize,
    cursors: &mut BTreeMap<PluginId, EventCursor>,
    event: PluginRuntimeEvent,
) -> Result<bool, PluginError> {
    let (plugin_id, content_owner, generation, sequence) = event_identity(&event);
    let snapshot = state.snapshot().await?;
    let Some(record) = snapshot.plugins.get(plugin_id) else {
        return Ok(false);
    };
    if &record.installation.content_owner != content_owner {
        return Ok(false);
    }

    match &event {
        PluginRuntimeEvent::Started { .. } => {
            if let Some(cursor) = cursors.get(plugin_id)
                && generation <= &cursor.generation
            {
                return Ok(false);
            }
        }
        PluginRuntimeEvent::Stopped { .. }
        | PluginRuntimeEvent::Crashed { .. }
        | PluginRuntimeEvent::TreeReaped { .. } => {
            let Some(cursor) = cursors.get(plugin_id) else {
                return Ok(false);
            };
            if cursor.content_owner != *content_owner
                || cursor.generation != *generation
                || sequence <= &cursor.sequence
            {
                return Ok(false);
            }
        }
    }

    if matches!(event, PluginRuntimeEvent::Crashed { .. }) {
        let policy = next_crash_policy(&snapshot, plugin_id, crash_window, crash_threshold)?;
        state
            .commit(StateMutation::SetCrashPolicy {
                plugin_id: plugin_id.clone(),
                policy,
            })
            .await?;
    }

    let terminal = matches!(
        event,
        PluginRuntimeEvent::Stopped { .. } | PluginRuntimeEvent::Crashed { .. }
    );
    cursors.insert(
        plugin_id.clone(),
        EventCursor {
            content_owner: content_owner.clone(),
            generation: *generation,
            sequence: *sequence,
        },
    );
    if terminal {
        cursors.remove(plugin_id);
    }
    Ok(true)
}

/// Extracts the generation identity shared by every closed event variant.
fn event_identity(
    event: &PluginRuntimeEvent,
) -> (&PluginId, &ContentOwnerId, &JsonSafeU64, &JsonSafeU64) {
    match event {
        PluginRuntimeEvent::Started {
            plugin_id,
            content_owner,
            generation,
            sequence,
        }
        | PluginRuntimeEvent::Stopped {
            plugin_id,
            content_owner,
            generation,
            sequence,
        }
        | PluginRuntimeEvent::Crashed {
            plugin_id,
            content_owner,
            generation,
            sequence,
            ..
        }
        | PluginRuntimeEvent::TreeReaped {
            plugin_id,
            content_owner,
            generation,
            sequence,
        } => (plugin_id, content_owner, generation, sequence),
    }
}

/// Advances the durable bounded crash window using Host wall time only for persistence.
fn next_crash_policy(
    snapshot: &PluginStateSnapshot,
    plugin_id: &PluginId,
    crash_window: std::time::Duration,
    crash_threshold: usize,
) -> Result<CrashPolicy, PluginError> {
    let record = snapshot
        .plugins
        .get(plugin_id)
        .ok_or_else(|| PluginError::NotFound {
            plugin_id: plugin_id.clone(),
        })?;
    let existing = match &record.crash_policy {
        CrashPolicy::Normal {
            recent_crashes_unix_ms,
        }
        | CrashPolicy::BlockedByCrashLoop {
            recent_crashes_unix_ms,
            ..
        } => recent_crashes_unix_ms,
    };
    let now = unix_time_ms()?;
    let window_ms = u64::try_from(crash_window.as_millis()).unwrap_or(u64::MAX);
    let oldest = now.get().saturating_sub(window_ms);
    let mut recent = existing
        .iter()
        .copied()
        .filter(|timestamp| timestamp.get() >= oldest && timestamp.get() <= now.get())
        .collect::<Vec<_>>();
    recent.push(now);
    if recent.len() >= crash_threshold.max(1) {
        Ok(CrashPolicy::BlockedByCrashLoop {
            recent_crashes_unix_ms: recent,
            blocked_at_unix_ms: now,
        })
    } else {
        Ok(CrashPolicy::Normal {
            recent_crashes_unix_ms: recent,
        })
    }
}

/// Reads a JSON-safe wall-clock value without exposing it to runtime deadline decisions.
fn unix_time_ms() -> Result<JsonSafeU64, PluginError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| PluginError::Internal {
            message: error.to_string(),
        })?
        .as_millis();
    let millis = u64::try_from(millis).map_err(|_| PluginError::Internal {
        message: "system time exceeds supported JSON integer".to_owned(),
    })?;
    JsonSafeU64::new(millis).map_err(|error| PluginError::Internal {
        message: error.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::ManagementRuntimeEventSink;
    use crate::{
        CrashPolicy, InstalledRecord, PluginManagerConfig, PluginRuntimeEvent,
        PluginRuntimeEventSink, PluginStateRecord, PluginStateSnapshot, StatePersistence,
        StateStore, UserEnablement,
    };
    use ora_plugin_protocol::{
        ContentDigest, ContentOwnerId, JsonSafeU64, OperationId, PluginId, PluginVersion,
    };
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    #[derive(Debug, Default)]
    struct MemoryPersistence;

    impl StatePersistence for MemoryPersistence {
        async fn commit(
            &mut self,
            _previous: &PluginStateSnapshot,
            _candidate: &PluginStateSnapshot,
        ) -> Result<(), crate::PluginError> {
            Ok(())
        }
    }

    /// Persists a crash window and ignores a crash from an obsolete content owner.
    #[tokio::test]
    async fn persists_current_owner_crash_loop_only() {
        let plugin_id = PluginId::parse("ora.events")
            .unwrap_or_else(|error| panic!("expected plugin id: {error}"));
        let owner = ContentOwnerId::parse(format!("sha256-{}", "a".repeat(64)))
            .unwrap_or_else(|error| panic!("expected owner: {error}"));
        let mut snapshot = PluginStateSnapshot::empty();
        snapshot.plugins.insert(
            plugin_id.clone(),
            PluginStateRecord {
                user_enablement: UserEnablement::Enabled,
                installation: InstalledRecord {
                    plugin_version: PluginVersion::parse("0.1.0")
                        .unwrap_or_else(|error| panic!("expected version: {error}")),
                    content_digest: ContentDigest::parse(format!("sha256:{}", "a".repeat(64)))
                        .unwrap_or_else(|error| panic!("expected digest: {error}")),
                    content_owner: owner.clone(),
                    install_operation_id: OperationId::parse(
                        "550e8400-e29b-41d4-a716-446655440000",
                    )
                    .unwrap_or_else(|error| panic!("expected operation id: {error}")),
                },
                crash_policy: CrashPolicy::normal(),
                enablement_epoch: json_safe(1),
            },
        );
        let state = StateStore::start(snapshot, MemoryPersistence, 16);
        let root =
            TempDir::new().unwrap_or_else(|error| panic!("expected temporary root: {error}"));
        let events = ManagementRuntimeEventSink::start(
            state.clone(),
            &PluginManagerConfig::new(root.path()),
            crate::PluginEventHub::new(16),
        );

        let obsolete = ContentOwnerId::parse(format!("sha256-{}", "b".repeat(64)))
            .unwrap_or_else(|error| panic!("expected obsolete owner: {error}"));
        events
            .record(started(&plugin_id, &obsolete, 1))
            .await
            .unwrap_or_else(|error| panic!("expected stale event ignore: {error}"));
        for generation in 1..=3 {
            events
                .record(started(&plugin_id, &owner, generation))
                .await
                .unwrap_or_else(|error| panic!("expected started event: {error}"));
            events
                .record(crashed(&plugin_id, &owner, generation))
                .await
                .unwrap_or_else(|error| panic!("expected crash event: {error}"));
        }

        let current = state
            .snapshot()
            .await
            .unwrap_or_else(|error| panic!("expected state: {error}"));
        assert_eq!(
            current
                .plugins
                .get(&plugin_id)
                .map(|record| record.crash_policy.is_blocked()),
            Some(true)
        );
    }

    fn started(
        plugin_id: &PluginId,
        owner: &ContentOwnerId,
        generation: u64,
    ) -> PluginRuntimeEvent {
        PluginRuntimeEvent::Started {
            plugin_id: plugin_id.clone(),
            content_owner: owner.clone(),
            generation: json_safe(generation),
            sequence: json_safe(generation * 10 + 1),
        }
    }

    fn crashed(
        plugin_id: &PluginId,
        owner: &ContentOwnerId,
        generation: u64,
    ) -> PluginRuntimeEvent {
        PluginRuntimeEvent::Crashed {
            plugin_id: plugin_id.clone(),
            content_owner: owner.clone(),
            generation: json_safe(generation),
            sequence: json_safe(generation * 10 + 3),
            exit_code: Some(1),
        }
    }

    fn json_safe(value: u64) -> JsonSafeU64 {
        JsonSafeU64::new(value).unwrap_or_else(|error| panic!("expected JSON integer: {error}"))
    }
}
