use crate::definition::{MountTarget, SurfaceDefinition, SurfaceSource};
use crate::ids::{
    OperationId, SurfaceDefinitionId, SurfaceInstanceId, ViewGeneration, WebviewLabel,
};
use crate::state::{
    StaleCompletion, SurfaceCommand, SurfaceCompletion, SurfaceEffect, SurfaceState, Transition,
    TransitionContext, TransitionError, apply_command, apply_completion,
};
use ora_domain::PluginId;
use ora_plugin_manager::InstancePolicy;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};
use thiserror::Error;

/// Process-wide table of live surface instances.
///
/// The lock only guards map updates and pure transitions; every side effect is handed back to
/// the caller so webview work never happens while the lock is held.
#[derive(Debug, Default)]
pub struct SurfaceRegistry {
    inner: Mutex<RegistryInner>,
}

#[derive(Debug, Default)]
struct RegistryInner {
    instances: HashMap<SurfaceInstanceId, SurfaceRecord>,
    /// Keyed by label text because hosts report raw strings from the webview runtime.
    by_label: HashMap<String, SurfaceInstanceId>,
    /// Conflict table for `InstancePolicy::Singleton`.
    by_definition: HashMap<SurfaceDefinitionId, SurfaceInstanceId>,
    next_instance: u64,
    next_operation: u64,
}

/// Snapshot of one instance; callers receive clones and never hold references into the registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceRecord {
    pub instance: SurfaceInstanceId,
    pub definition: SurfaceDefinition,
    pub label: WebviewLabel,
    pub state: SurfaceState,
}

/// Why an open request was refused.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum OpenError {
    /// A singleton surface is already live; callers should focus the existing instance.
    /// Boxed because the record is far larger than the `Ok` path's pointer-sized payload.
    #[error("surface {} is already open as instance {}", .0.label, .0.instance.value())]
    AlreadyOpen(Box<SurfaceRecord>),
}

/// Why a command could not be applied.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum CommandError {
    #[error("surface instance {} is not registered", .0.value())]
    UnknownInstance(SurfaceInstanceId),
    #[error(transparent)]
    Transition(#[from] TransitionError),
}

/// Why a completion could not be applied.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum CompleteError {
    #[error("surface instance {} is not registered", .0.value())]
    UnknownInstance(SurfaceInstanceId),
    #[error(transparent)]
    Stale(#[from] StaleCompletion),
}

impl SurfaceRegistry {
    /// Registers a new instance in `Opening` and returns the create request for the host.
    pub fn open(
        &self,
        definition: SurfaceDefinition,
        target: MountTarget,
    ) -> Result<(SurfaceRecord, Vec<SurfaceEffect>), OpenError> {
        let mut inner = self.lock();
        match definition.instance_policy {
            InstancePolicy::Singleton => {
                if let Some(existing) = inner
                    .by_definition
                    .get(&definition.id)
                    .and_then(|instance| inner.instances.get(instance))
                {
                    return Err(OpenError::AlreadyOpen(Box::new(existing.clone())));
                }
            }
        }
        let instance = SurfaceInstanceId::new(inner.next_instance);
        inner.next_instance += 1;
        let operation = inner.allocate_operation();
        // The label family decides which Tauri capability (if any) the webview receives, so it
        // must follow the content source rather than a caller-provided choice.
        let label = match &definition.source {
            SurfaceSource::RemoteSite(_) => WebviewLabel::remote(&definition.id, instance),
            SurfaceSource::Panel(_) => WebviewLabel::panel(&definition.id, instance),
        };
        let record = SurfaceRecord {
            instance,
            label: label.clone(),
            state: SurfaceState::Opening {
                target,
                operation,
                view: ViewGeneration::INITIAL,
                close_requested: false,
            },
            definition,
        };
        inner.by_label.insert(label.as_str().to_owned(), instance);
        inner
            .by_definition
            .insert(record.definition.id.clone(), instance);
        inner.instances.insert(instance, record.clone());
        Ok((
            record,
            vec![SurfaceEffect::CreateWebview { target, operation }],
        ))
    }

    /// Applies a command to one instance and returns the effects to execute outside the lock.
    pub fn command(
        &self,
        instance: SurfaceInstanceId,
        command: SurfaceCommand,
    ) -> Result<Vec<SurfaceEffect>, CommandError> {
        let mut inner = self.lock();
        let record = inner
            .instances
            .get(&instance)
            .ok_or(CommandError::UnknownInstance(instance))?;
        // Tickets are allocated from a local counter copy so a refused command, which never
        // calls the generator, cannot observe a half-updated registry.
        let mut next_operation = inner.next_operation;
        let context = TransitionContext {
            instance,
            definition: &record.definition,
        };
        let transition = apply_command(context, &record.state, command, || {
            next_operation += 1;
            OperationId::new(next_operation - 1)
        })?;
        inner.next_operation = next_operation;
        Ok(inner.settle(instance, transition))
    }

    /// Applies an operation completion to one instance.
    pub fn complete(
        &self,
        instance: SurfaceInstanceId,
        completion: SurfaceCompletion,
    ) -> Result<Vec<SurfaceEffect>, CompleteError> {
        let mut inner = self.lock();
        let record = inner
            .instances
            .get(&instance)
            .ok_or(CompleteError::UnknownInstance(instance))?;
        let mut next_operation = inner.next_operation;
        let context = TransitionContext {
            instance,
            definition: &record.definition,
        };
        let transition = apply_completion(context, &record.state, completion, || {
            next_operation += 1;
            OperationId::new(next_operation - 1)
        })?;
        inner.next_operation = next_operation;
        Ok(inner.settle(instance, transition))
    }

    /// Returns the current snapshot of one instance, if it is still live.
    pub fn record(&self, instance: SurfaceInstanceId) -> Option<SurfaceRecord> {
        self.lock().instances.get(&instance).cloned()
    }

    /// Resolves a webview label to its record; the authorization source for assets, downloads,
    /// and bridge calls. Unregistered labels yield `None`.
    pub fn resolve_label(&self, label: &str) -> Option<SurfaceRecord> {
        let inner = self.lock();
        inner
            .by_label
            .get(label)
            .and_then(|instance| inner.instances.get(instance))
            .cloned()
    }

    /// Lists every live instance contributed by one plugin, ordered by instance id.
    pub fn instances_of(&self, plugin_id: &PluginId) -> Vec<SurfaceRecord> {
        let mut records: Vec<SurfaceRecord> = self
            .lock()
            .instances
            .values()
            .filter(|record| &record.definition.id.plugin_id == plugin_id)
            .cloned()
            .collect();
        records.sort_by_key(|record| record.instance);
        records
    }

    /// Lists every live instance, ordered by instance id.
    pub fn snapshot(&self) -> Vec<SurfaceRecord> {
        let mut records: Vec<SurfaceRecord> = self.lock().instances.values().cloned().collect();
        records.sort_by_key(|record| record.instance);
        records
    }

    /// Recovers from a poisoned lock: the guarded data only holds plain maps that stay
    /// consistent because every mutation happens after the pure transition succeeded.
    fn lock(&self) -> MutexGuard<'_, RegistryInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl RegistryInner {
    /// Hands out the next operation ticket.
    fn allocate_operation(&mut self) -> OperationId {
        let operation = OperationId::new(self.next_operation);
        self.next_operation += 1;
        operation
    }

    /// Stores the transition result: updates the record or removes the ended instance.
    fn settle(
        &mut self,
        instance: SurfaceInstanceId,
        transition: Transition,
    ) -> Vec<SurfaceEffect> {
        match transition.next {
            Some(state) => {
                if let Some(record) = self.instances.get_mut(&instance) {
                    record.state = state;
                }
            }
            None => {
                if let Some(record) = self.instances.remove(&instance) {
                    self.by_label.remove(record.label.as_str());
                    self.by_definition.remove(&record.definition.id);
                }
            }
        }
        transition.effects
    }
}

#[cfg(test)]
mod tests {
    use super::{CommandError, CompleteError, OpenError, SurfaceRecord, SurfaceRegistry};
    use crate::definition::{MountTarget, RemoteSiteDefinition, SurfaceDefinition, SurfaceSource};
    use crate::events::SurfaceEvent;
    use crate::ids::{
        OperationId, SurfaceDefinitionId, SurfaceInstanceId, ViewGeneration, WebviewLabel,
    };
    use crate::navigation::NavigationPolicy;
    use crate::state::{
        StaleCompletion, SurfaceCommand, SurfaceCompletion, SurfaceEffect, SurfaceState,
        TransitionError,
    };
    use ora_domain::PluginId;
    use ora_plugin_manager::{InstancePolicy, SurfaceId, WebDataPolicy};
    use pretty_assertions::assert_eq;
    use url::Url;

    /// Builds a definition for the given plugin/surface pair.
    fn definition(plugin: &str, surface: &str) -> SurfaceDefinition {
        SurfaceDefinition {
            id: SurfaceDefinitionId {
                plugin_id: PluginId::new(plugin),
                surface_id: SurfaceId::parse(surface).expect("valid surface id"),
            },
            title: surface.to_owned(),
            source: SurfaceSource::RemoteSite(RemoteSiteDefinition {
                entry_url: Url::parse("https://www.skillhub.cn").expect("valid url"),
                navigation: NavigationPolicy::remote_site(vec![], vec![]),
                web_data: WebDataPolicy::EphemeralIsolated,
            }),
            instance_policy: InstancePolicy::Singleton,
        }
    }

    /// Verifies open registers an Opening record, hands out a create effect, and refuses a second
    /// singleton instance while the first is alive.
    #[test]
    fn open_registers_singleton_once() {
        let registry = SurfaceRegistry::default();
        let definition = definition("ora-space.skillhub", "market");

        let (record, effects) = registry
            .open(definition.clone(), MountTarget::Embedded)
            .expect("first open succeeds");
        let duplicate = registry.open(definition.clone(), MountTarget::Windowed);

        let expected = SurfaceRecord {
            instance: SurfaceInstanceId::new(0),
            definition,
            label: WebviewLabel::remote(
                &SurfaceDefinitionId {
                    plugin_id: PluginId::new("ora-space.skillhub"),
                    surface_id: SurfaceId::parse("market").expect("valid surface id"),
                },
                SurfaceInstanceId::new(0),
            ),
            state: SurfaceState::Opening {
                target: MountTarget::Embedded,
                operation: OperationId::new(0),
                view: ViewGeneration::INITIAL,
                close_requested: false,
            },
        };
        assert_eq!(
            (record, effects, duplicate, registry.snapshot()),
            (
                expected.clone(),
                vec![SurfaceEffect::CreateWebview {
                    target: MountTarget::Embedded,
                    operation: OperationId::new(0),
                }],
                Err(OpenError::AlreadyOpen(Box::new(expected.clone()))),
                vec![expected],
            )
        );
    }

    /// Verifies the full open -> popout -> close flow updates state, allocates tickets in order,
    /// and removes the instance (and its label) once closed.
    #[test]
    fn drives_lifecycle_and_removes_closed_instances() {
        let registry = SurfaceRegistry::default();
        let (record, _) = registry
            .open(
                definition("ora-space.skillhub", "market"),
                MountTarget::Embedded,
            )
            .expect("open");
        let instance = record.instance;

        let opened = registry.complete(
            instance,
            SurfaceCompletion::Opened {
                operation: OperationId::new(0),
                outcome: Ok(MountTarget::Embedded),
            },
        );
        let popout = registry.command(instance, SurfaceCommand::Popout);
        let busy = registry.command(instance, SurfaceCommand::Dock);
        let migrated = registry.complete(
            instance,
            SurfaceCompletion::Migrated {
                operation: OperationId::new(1),
                outcome: Ok(MountTarget::Windowed),
            },
        );
        let state_after_migrate = registry
            .resolve_label(record.label.as_str())
            .map(|r| r.state);
        let close = registry.command(instance, SurfaceCommand::Close);
        let closed = registry.complete(
            instance,
            SurfaceCompletion::Closed {
                operation: OperationId::new(2),
            },
        );
        let reopened = registry
            .open(
                definition("ora-space.skillhub", "market"),
                MountTarget::Embedded,
            )
            .map(|(record, _)| record.instance);

        assert_eq!(
            (
                opened,
                popout,
                busy,
                migrated,
                state_after_migrate,
                close,
                closed,
                registry.resolve_label(record.label.as_str()),
                reopened,
            ),
            (
                Ok(vec![SurfaceEffect::Emit(SurfaceEvent::Opened {
                    instance: 0,
                    plugin_id: "ora-space.skillhub".to_owned(),
                    surface_id: "market".to_owned(),
                    target: MountTarget::Embedded,
                    title: "market".to_owned(),
                })]),
                Ok(vec![SurfaceEffect::Reparent {
                    to: MountTarget::Windowed,
                    operation: OperationId::new(1),
                }]),
                Err(CommandError::Transition(TransitionError::Busy {
                    current: "migrating",
                })),
                Ok(vec![SurfaceEffect::Emit(SurfaceEvent::Migrated {
                    instance: 0,
                    target: MountTarget::Windowed,
                })]),
                Some(SurfaceState::Windowed {
                    view: ViewGeneration::INITIAL,
                }),
                Ok(vec![SurfaceEffect::DestroyWebview {
                    operation: OperationId::new(2),
                }]),
                Ok(vec![SurfaceEffect::Emit(SurfaceEvent::Closed {
                    instance: 0
                })]),
                None,
                Ok(SurfaceInstanceId::new(1)),
            )
        );
    }

    /// Verifies stale completions and unknown instances are reported without mutating state.
    #[test]
    fn rejects_stale_completions_and_unknown_instances() {
        let registry = SurfaceRegistry::default();
        let (record, _) = registry
            .open(
                definition("ora-space.skillhub", "market"),
                MountTarget::Embedded,
            )
            .expect("open");

        let stale = registry.complete(
            record.instance,
            SurfaceCompletion::Closed {
                operation: OperationId::new(0),
            },
        );
        let unknown_command = registry.command(SurfaceInstanceId::new(42), SurfaceCommand::Close);
        let unknown_completion = registry.complete(
            SurfaceInstanceId::new(42),
            SurfaceCompletion::Closed {
                operation: OperationId::new(0),
            },
        );

        assert_eq!(
            (
                stale,
                unknown_command,
                unknown_completion,
                registry.snapshot()
            ),
            (
                Err(CompleteError::Stale(StaleCompletion {
                    completion: "closed",
                    received: OperationId::new(0),
                    current: "opening",
                })),
                Err(CommandError::UnknownInstance(SurfaceInstanceId::new(42))),
                Err(CompleteError::UnknownInstance(SurfaceInstanceId::new(42))),
                vec![record],
            )
        );
    }

    /// Verifies a panel definition gets the panel label family, which is what its capability
    /// matches on.
    #[test]
    fn open_labels_panels_with_panel_prefix() {
        let registry = SurfaceRegistry::default();
        let mut definition = definition("ora-space.hello-panel", "counter");
        definition.source = SurfaceSource::Panel(crate::definition::PanelDefinition {
            asset_root: std::path::PathBuf::from("/plugins/hello-panel/ui"),
            entry: ora_utils::path::PortableRelativePath::parse("index.html").expect("entry"),
        });

        let (record, _) = registry
            .open(definition, MountTarget::Windowed)
            .expect("open succeeds");

        assert_eq!(
            record.label.as_str(),
            "panel-surface:ora-space_hello-panel:counter:0"
        );
    }

    /// Verifies per-plugin listing ignores other plugins and unknown labels resolve to nothing.
    #[test]
    fn lists_instances_per_plugin_and_resolves_labels() {
        let registry = SurfaceRegistry::default();
        let (first, _) = registry
            .open(
                definition("ora-space.skillhub", "market"),
                MountTarget::Embedded,
            )
            .expect("open first");
        let (second, _) = registry
            .open(
                definition("ora-space.skillhub", "docs"),
                MountTarget::Embedded,
            )
            .expect("open second");
        let (other, _) = registry
            .open(definition("acme.tools", "panel"), MountTarget::Windowed)
            .expect("open other");

        assert_eq!(
            (
                registry.instances_of(&PluginId::new("ora-space.skillhub")),
                registry.resolve_label(other.label.as_str()),
                registry.resolve_label("remote-surface:unknown:panel:0"),
            ),
            (vec![first, second], Some(other), None)
        );
    }
}
