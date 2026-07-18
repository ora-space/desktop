use crate::{EffectiveEnablement, PluginDiagnostic, PluginRuntimeEvent};
use ora_plugin_protocol::{JsonSafeU64, OperationId, PluginId};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::broadcast;

/// Stable runtime lifecycle projections that never expose process handles or stderr payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeEventState {
    Started,
    Stopped,
    Crashed { exit_code: Option<i32> },
    TreeReaped,
}

/// Stable install/removal progress milestones safe for non-authoritative observers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallPhase {
    InstalledDisabled,
    Removed,
}

/// Bounded metadata-only events; snapshots remain the authority after lag or restart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginEvent {
    CatalogChanged {
        revision: JsonSafeU64,
    },
    RegistryChanged {
        revision: JsonSafeU64,
        added: Vec<PluginId>,
        removed: Vec<PluginId>,
    },
    EnablementChanged {
        plugin_id: PluginId,
        effective: EffectiveEnablement,
    },
    RuntimeChanged {
        plugin_id: PluginId,
        generation: JsonSafeU64,
        state: RuntimeEventState,
    },
    InstallProgress {
        operation_id: OperationId,
        phase: InstallPhase,
    },
    Diagnostic {
        plugin_id: Option<PluginId>,
        diagnostic: PluginDiagnostic,
    },
}

/// Distinguishes one delivered event from a lag signal requiring a fresh snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginEventDelivery {
    Event(PluginEvent),
    ResyncRequired { skipped: u64 },
}

/// One bounded observer that cannot backpressure management or runtime actors.
pub struct PluginEventSubscriber {
    receiver: broadcast::Receiver<PluginEvent>,
}

impl PluginEventSubscriber {
    pub async fn recv(&mut self) -> Result<PluginEventDelivery, crate::PluginError> {
        match self.receiver.recv().await {
            Ok(event) => Ok(PluginEventDelivery::Event(event)),
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                Ok(PluginEventDelivery::ResyncRequired { skipped })
            }
            Err(broadcast::error::RecvError::Closed) => {
                Err(crate::PluginError::BackendShuttingDown)
            }
        }
    }
}

#[derive(Default)]
struct PublishedRevisions {
    catalog: Option<JsonSafeU64>,
    registry: Option<JsonSafeU64>,
}

/// Shared non-blocking publisher with revision de-duplication for repeated equivalent scans.
#[derive(Clone)]
pub(crate) struct PluginEventHub {
    sender: broadcast::Sender<PluginEvent>,
    published: Arc<Mutex<PublishedRevisions>>,
}

impl PluginEventHub {
    pub(crate) fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity.max(1));
        Self {
            sender,
            published: Arc::new(Mutex::new(PublishedRevisions::default())),
        }
    }

    pub(crate) fn subscribe(&self) -> PluginEventSubscriber {
        PluginEventSubscriber {
            receiver: self.sender.subscribe(),
        }
    }

    pub(crate) fn publish_catalog(&self, revision: JsonSafeU64) {
        let mut published = self
            .published
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if published.catalog == Some(revision) {
            return;
        }
        published.catalog = Some(revision);
        drop(published);
        self.publish(PluginEvent::CatalogChanged { revision });
    }

    pub(crate) fn publish_registry(
        &self,
        previous: &crate::RegistrySnapshot,
        current: &crate::RegistrySnapshot,
    ) {
        let mut published = self
            .published
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if published.registry == Some(current.revision) {
            return;
        }
        published.registry = Some(current.revision);
        drop(published);
        let previous_ids = previous
            .plugins_by_id
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        let current_ids = current
            .plugins_by_id
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        self.publish(PluginEvent::RegistryChanged {
            revision: current.revision,
            added: current_ids.difference(&previous_ids).cloned().collect(),
            removed: previous_ids.difference(&current_ids).cloned().collect(),
        });
    }

    pub(crate) fn publish(&self, event: PluginEvent) {
        // Absence or lag of observers is expected; business actors never await this channel.
        let _ = self.sender.send(event);
    }

    pub(crate) fn publish_runtime(&self, event: &PluginRuntimeEvent) {
        let (plugin_id, generation, state) = match event {
            PluginRuntimeEvent::Started {
                plugin_id,
                generation,
                ..
            } => (plugin_id, generation, RuntimeEventState::Started),
            PluginRuntimeEvent::Stopped {
                plugin_id,
                generation,
                ..
            } => (plugin_id, generation, RuntimeEventState::Stopped),
            PluginRuntimeEvent::Crashed {
                plugin_id,
                generation,
                exit_code,
                ..
            } => (
                plugin_id,
                generation,
                RuntimeEventState::Crashed {
                    exit_code: *exit_code,
                },
            ),
            PluginRuntimeEvent::TreeReaped {
                plugin_id,
                generation,
                ..
            } => (plugin_id, generation, RuntimeEventState::TreeReaped),
        };
        self.publish(PluginEvent::RuntimeChanged {
            plugin_id: plugin_id.clone(),
            generation: *generation,
            state,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{PluginEvent, PluginEventDelivery, PluginEventHub};
    use ora_plugin_protocol::JsonSafeU64;
    use pretty_assertions::assert_eq;

    /// Equivalent catalog publications are deduplicated and a slow observer receives resync.
    #[tokio::test]
    async fn bounded_observer_reports_lag_without_blocking_publisher() {
        let hub = PluginEventHub::new(1);
        let mut subscriber = hub.subscribe();
        let one =
            JsonSafeU64::new(1).unwrap_or_else(|error| panic!("expected revision one: {error}"));
        let two =
            JsonSafeU64::new(2).unwrap_or_else(|error| panic!("expected revision two: {error}"));
        hub.publish_catalog(one);
        hub.publish_catalog(one);
        hub.publish_catalog(two);

        assert_eq!(
            subscriber
                .recv()
                .await
                .unwrap_or_else(|error| panic!("expected lag delivery: {error}")),
            PluginEventDelivery::ResyncRequired { skipped: 1 }
        );
        assert_eq!(
            subscriber
                .recv()
                .await
                .unwrap_or_else(|error| panic!("expected latest event: {error}")),
            PluginEventDelivery::Event(PluginEvent::CatalogChanged { revision: two })
        );
    }
}
