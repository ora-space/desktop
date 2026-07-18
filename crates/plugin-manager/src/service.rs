use crate::{
    AuthorityClock, CandidateAuthority, CandidateHandle, CandidateSelection,
    EffectiveDisableReason, EffectiveEnablement, EnablementFacts, FileStatePersistence,
    IdentifiedPlugin, InstallPhase, InstallReconciler, InstalledPlugin, InstalledScan,
    InstalledScanner, IntegrityStatus, ManagementRuntimeEventSink, ManagementSessionId,
    ManagerLease, ManifestValidity, NoopPluginRuntimeControl, PackageStoreCoordinator,
    PackageValidator, PendingOperation, PendingRemoval, PendingRemovalPhase, PluginCatalogSnapshot,
    PluginError, PluginEvent, PluginEventHub, PluginEventSubscriber, PluginInstaller,
    PluginLaunchGrant, PluginManagerConfig, PluginMutationCoordinator, PluginRuntimeControl,
    RegistryCandidate, RegistrySnapshot, RemovalMarker, RuntimeCompatibility, RuntimeRegistry,
    RuntimeSupport, SafeTreeDeleter, SelectionHandle, StateMutation, StateStore, StopReason,
    UserEnablement, ValidatedLaunchDescriptor, derive_effective_enablement, ensure_removal_marker,
};
use ora_plugin_protocol::{CandidateAuditId, OperationId, PluginId, PluginKind, PluginManifest};
use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

/// A configured discovery root identifier that never exposes its path over the API.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DiscoveryRootId(String);

impl DiscoveryRootId {
    pub fn parse(value: impl Into<String>) -> Result<Self, PluginError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 128
            || !value.is_ascii()
            || value
                .bytes()
                .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')))
        {
            return Err(PluginError::Internal {
                message: "discovery root id is invalid".to_string(),
            });
        }
        Ok(Self(value))
    }
}

/// Names the mutable-data scope explicitly for destructive management operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataRemovalScope {
    CurrentContentOwner,
    AllOwners,
}

/// The complete backend management API; paths never cross this public boundary.
pub trait PluginManagement {
    /// Mints one session-bound selection only from a trusted native-picker result.
    fn register_native_selection(
        &self,
        session: ManagementSessionId,
        path: &Path,
    ) -> Result<CandidateSelection, PluginError>;

    fn scan_installed(
        &self,
    ) -> impl Future<Output = Result<PluginCatalogSnapshot, PluginError>> + Send;

    fn scan_candidates(
        &self,
        session: &ManagementSessionId,
        roots: Vec<DiscoveryRootId>,
    ) -> impl Future<Output = Result<Vec<CandidateSelection>, PluginError>> + Send;

    fn identify(
        &self,
        session: &ManagementSessionId,
        selection: SelectionHandle,
    ) -> impl Future<Output = Result<IdentifiedPlugin, PluginError>> + Send;

    fn install_authorized_candidate(
        &self,
        session: &ManagementSessionId,
        candidate: CandidateHandle,
    ) -> impl Future<Output = Result<InstalledPlugin, PluginError>> + Send;

    fn enable(&self, plugin_id: PluginId) -> impl Future<Output = Result<(), PluginError>> + Send;

    fn disable(&self, plugin_id: PluginId) -> impl Future<Output = Result<(), PluginError>> + Send;

    fn uninstall(
        &self,
        plugin_id: PluginId,
    ) -> impl Future<Output = Result<(), PluginError>> + Send;

    fn set_launch_grant(
        &self,
        grant: PluginLaunchGrant,
    ) -> impl Future<Output = Result<(), PluginError>> + Send;

    fn get_launch_grant(
        &self,
        plugin_id: &PluginId,
    ) -> impl Future<Output = Result<Option<PluginLaunchGrant>, PluginError>> + Send;

    fn revoke_launch_grant(
        &self,
        plugin_id: PluginId,
    ) -> impl Future<Output = Result<(), PluginError>> + Send;

    fn reset_crash_loop(
        &self,
        plugin_id: PluginId,
    ) -> impl Future<Output = Result<(), PluginError>> + Send;

    fn remove_plugin_data(
        &self,
        plugin_id: PluginId,
        scope: DataRemovalScope,
    ) -> impl Future<Output = Result<(), PluginError>> + Send;
}

/// Owns the process-lifetime lease and composes every management subsystem around one state actor.
pub struct PluginManagementService<Clock, Control> {
    config: PluginManagerConfig,
    _lease: Arc<ManagerLease>,
    validator: PackageValidator,
    state: StateStore,
    package_store: Arc<PackageStoreCoordinator>,
    mutations: Arc<PluginMutationCoordinator>,
    authority: CandidateAuthority<Clock>,
    scanner: InstalledScanner,
    installer: PluginInstaller,
    registry: Mutex<RuntimeRegistry>,
    runtime_control: Control,
    runtime_events: ManagementRuntimeEventSink,
    events: PluginEventHub,
    discovery_roots: BTreeMap<DiscoveryRootId, PathBuf>,
}

impl<Clock, Control> PluginManagementService<Clock, Control>
where
    Clock: AuthorityClock,
    Control: PluginRuntimeControl,
{
    /// Acquires the lease, recovers state, and constructs one authoritative management service.
    pub async fn bootstrap(
        config: PluginManagerConfig,
        clock: Clock,
        runtime_control: Control,
        discovery_roots: BTreeMap<DiscoveryRootId, PathBuf>,
    ) -> Result<Self, PluginError> {
        let lease = Arc::new(ManagerLease::acquire(&config)?);
        Self::bootstrap_with_lease(config, clock, runtime_control, discovery_roots, lease).await
    }

    /// Composes management around an already acquired lease so asset bootstrap shares its owner.
    pub async fn bootstrap_with_lease(
        config: PluginManagerConfig,
        clock: Clock,
        runtime_control: Control,
        discovery_roots: BTreeMap<DiscoveryRootId, PathBuf>,
        lease: Arc<ManagerLease>,
    ) -> Result<Self, PluginError> {
        lease.assert_held()?;
        let persistence = FileStatePersistence::new(config.plugin_system_dir());
        let recovery = persistence.load_or_recover().await?;
        let state = StateStore::start(recovery.snapshot, persistence, 64);
        let validator = PackageValidator::new(
            config.limits.clone(),
            config.host_version.clone(),
            config.bun_version.clone(),
        );
        let package_store = Arc::new(PackageStoreCoordinator::new());
        let mutations = Arc::new(PluginMutationCoordinator::new());
        let events = PluginEventHub::new(128);
        let runtime_events =
            ManagementRuntimeEventSink::start(state.clone(), &config, events.clone());
        InstallReconciler::new(
            config.clone(),
            validator.clone(),
            state.clone(),
            package_store.clone(),
        )
        .reconcile()
        .await?;
        let scanner = InstalledScanner::new(
            config.clone(),
            validator.clone(),
            state.clone(),
            package_store.clone(),
        );
        let installer = PluginInstaller::new(
            config.clone(),
            validator.clone(),
            state.clone(),
            lease.clone(),
            package_store.clone(),
            mutations.clone(),
        );
        let service = Self {
            authority: CandidateAuthority::new(clock, config.selection_ttl, config.candidate_ttl),
            config,
            _lease: lease,
            validator,
            state,
            package_store,
            mutations,
            scanner,
            installer,
            registry: Mutex::new(RuntimeRegistry::new()),
            runtime_control,
            runtime_events,
            events,
            discovery_roots,
        };
        service.scan_and_reconcile().await?;
        Ok(service)
    }

    /// Registers a trusted native-picker path without returning it to the caller.
    pub fn register_native_selection(
        &self,
        session: ManagementSessionId,
        path: &Path,
    ) -> Result<CandidateSelection, PluginError> {
        self.authority
            .register_selection(session, path, new_audit_id())
    }

    pub async fn registry_snapshot(&self) -> RegistrySnapshot {
        self.registry.lock().await.snapshot()
    }

    /// Returns the critical runtime event port bound to this service's durable state actor.
    pub fn runtime_event_sink(&self) -> ManagementRuntimeEventSink {
        self.runtime_events.clone()
    }

    /// Subscribes a non-authoritative observer to bounded metadata-only plugin events.
    pub fn subscribe_events(&self) -> PluginEventSubscriber {
        self.events.subscribe()
    }

    /// Refreshes catalog and atomically derives the only runtime-eligible registry snapshot.
    async fn scan_and_reconcile(&self) -> Result<InstalledScan, PluginError> {
        let scan = self.scanner.scan_installed().await?;
        let mut candidates = Vec::new();
        for (plugin_id, package) in &scan.validated {
            let Some(record) = scan.state.plugins.get(plugin_id) else {
                continue;
            };
            let Some(entry) = scan
                .catalog
                .entries
                .iter()
                .find(|entry| entry.plugin_id.as_ref() == Some(plugin_id))
            else {
                continue;
            };
            let effective_enablement = effective_enablement(&scan, plugin_id, entry, record);
            candidates.push(RegistryCandidate {
                package,
                content_owner: &record.installation.content_owner,
                enablement_epoch: record.enablement_epoch,
                effective_enablement,
            });
        }
        let mut registry = self.registry.lock().await;
        let previous = registry.snapshot();
        let current = registry
            .reconcile(scan.catalog.revision, scan.state.revision, &candidates)
            .map_err(|error| PluginError::Internal {
                message: error.to_string(),
            })?;
        drop(registry);
        self.events.publish_catalog(scan.catalog.revision);
        self.events.publish_registry(&previous, &current);
        Ok(scan)
    }

    /// Verifies a catalog entry can be enabled before persisting user intent.
    fn require_enableable(
        &self,
        scan: &InstalledScan,
        plugin_id: &PluginId,
    ) -> Result<(), PluginError> {
        let entry = scan
            .catalog
            .entries
            .iter()
            .find(|entry| entry.plugin_id.as_ref() == Some(plugin_id))
            .ok_or_else(|| PluginError::NotFound {
                plugin_id: plugin_id.clone(),
            })?;
        if entry.validity != ManifestValidity::Valid {
            return Err(PluginError::InvalidManifest {
                diagnostics: entry.diagnostics.clone(),
            });
        }
        if let RuntimeCompatibility::Incompatible(reason) = entry.compatibility {
            return Err(PluginError::Incompatible { reason });
        }
        if let RuntimeSupport::UnsupportedKind { kind } = entry.support {
            return Err(PluginError::UnsupportedKind { kind });
        }
        if entry.integrity != IntegrityStatus::Verified {
            return Err(PluginError::IntegrityMismatch {
                plugin_id: plugin_id.clone(),
            });
        }
        Ok(())
    }

    /// Returns the current running descriptor from fresh scan and registry facts.
    async fn admitted_descriptor(
        &self,
        plugin_id: &PluginId,
    ) -> Result<ValidatedLaunchDescriptor, PluginError> {
        let scan = self.scan_and_reconcile().await?;
        let package = scan
            .validated
            .get(plugin_id)
            .ok_or_else(|| PluginError::Disabled {
                plugin_id: plugin_id.clone(),
                reason: EffectiveDisableReason::IntegrityMismatch,
            })?;
        let record = scan
            .state
            .plugins
            .get(plugin_id)
            .ok_or_else(|| PluginError::NotFound {
                plugin_id: plugin_id.clone(),
            })?;
        let registry = self.registry.lock().await.snapshot();
        if !registry.plugins_by_id.contains_key(plugin_id) {
            let entry = scan
                .catalog
                .entries
                .iter()
                .find(|entry| entry.plugin_id.as_ref() == Some(plugin_id))
                .ok_or_else(|| PluginError::Internal {
                    message: "validated package has no catalog entry".to_owned(),
                })?;
            return match effective_enablement(&scan, plugin_id, entry, record) {
                EffectiveEnablement::Disabled(reason) => Err(PluginError::Disabled {
                    plugin_id: plugin_id.clone(),
                    reason,
                }),
                EffectiveEnablement::Enabled => Err(PluginError::Internal {
                    message: "enabled package is absent from the runtime registry".to_owned(),
                }),
            };
        }
        let PluginManifest::Agent {
            main, contributes, ..
        } = &package.manifest.ora
        else {
            return Err(PluginError::UnsupportedKind {
                kind: PluginKind::Workbench,
            });
        };
        let storage_path = self
            .config
            .plugin_data_dir()
            .join(plugin_id.as_str())
            .join(record.installation.content_owner.as_str());
        std::fs::create_dir_all(&storage_path).map_err(internal_io)?;
        Ok(ValidatedLaunchDescriptor {
            plugin_id: plugin_id.clone(),
            plugin_version: package.manifest.version.clone(),
            kind: PluginKind::Agent,
            content_digest: package.digest.digest.clone(),
            content_owner: record.installation.content_owner.clone(),
            extension_path: package.root.clone(),
            entry_path: package.root.join(main.as_str()),
            storage_path,
            declared_agents: contributes
                .agents
                .iter()
                .map(|agent| agent.id.clone())
                .collect(),
            enablement_epoch: record.enablement_epoch,
            registry_revision: registry.revision,
            launch_grant: scan.state.launch_grants.get(plugin_id).cloned(),
        })
    }
}

/// Derives one plugin's admission fact identically for registry publication and API errors.
fn effective_enablement(
    scan: &InstalledScan,
    plugin_id: &PluginId,
    entry: &crate::CatalogEntry,
    record: &crate::PluginStateRecord,
) -> EffectiveEnablement {
    let pending_removal = scan.state.pending_operations.iter().any(|operation| {
        matches!(operation, PendingOperation::Remove(removal) if &removal.plugin_id == plugin_id)
    });
    derive_effective_enablement(
        entry,
        record.user_enablement,
        EnablementFacts {
            pending_removal,
            missing_install_files: false,
            policy_denied: false,
            crash_loop: record.crash_policy.is_blocked(),
        },
    )
}

impl<Clock, Control> PluginManagement for PluginManagementService<Clock, Control>
where
    Clock: AuthorityClock,
    Control: PluginRuntimeControl,
{
    fn register_native_selection(
        &self,
        session: ManagementSessionId,
        path: &Path,
    ) -> Result<CandidateSelection, PluginError> {
        PluginManagementService::register_native_selection(self, session, path)
    }

    async fn scan_installed(&self) -> Result<PluginCatalogSnapshot, PluginError> {
        Ok(self.scan_and_reconcile().await?.catalog)
    }

    async fn scan_candidates(
        &self,
        session: &ManagementSessionId,
        roots: Vec<DiscoveryRootId>,
    ) -> Result<Vec<CandidateSelection>, PluginError> {
        let mut selections = Vec::new();
        for root_id in roots {
            let root = self
                .discovery_roots
                .get(&root_id)
                .ok_or_else(|| PluginError::Internal {
                    message: "unknown discovery root".to_string(),
                })?;
            for entry in std::fs::read_dir(root).map_err(internal_io)? {
                let entry = entry.map_err(internal_io)?;
                if entry.file_type().map_err(internal_io)?.is_dir() {
                    selections.push(self.authority.register_selection(
                        session.clone(),
                        &entry.path(),
                        new_audit_id(),
                    )?);
                }
            }
        }
        Ok(selections)
    }

    async fn identify(
        &self,
        session: &ManagementSessionId,
        selection: SelectionHandle,
    ) -> Result<IdentifiedPlugin, PluginError> {
        self.authority.identify(session, selection, &self.validator)
    }

    async fn install_authorized_candidate(
        &self,
        session: &ManagementSessionId,
        candidate: CandidateHandle,
    ) -> Result<InstalledPlugin, PluginError> {
        let authorized = self.authority.consume_candidate(session, candidate)?;
        let installed = self
            .installer
            .install_authorized_candidate(authorized)
            .await?;
        self.scan_and_reconcile().await?;
        self.events.publish(PluginEvent::InstallProgress {
            operation_id: installed.receipt.operation_id.clone(),
            phase: InstallPhase::InstalledDisabled,
        });
        Ok(installed)
    }

    async fn enable(&self, plugin_id: PluginId) -> Result<(), PluginError> {
        let gate = self.mutations.gate(&plugin_id).await;
        let _guard = gate.lock().await;
        let scan = self.scan_and_reconcile().await?;
        self.require_enableable(&scan, &plugin_id)?;
        self.state
            .commit(StateMutation::SetEnablement {
                plugin_id: plugin_id.clone(),
                enablement: UserEnablement::Enabled,
                advance_epoch: true,
            })
            .await?;
        self.runtime_control.reset_crash_loop(&plugin_id).await?;
        self.runtime_control.open_admission(&plugin_id).await?;
        self.scan_and_reconcile().await?;
        self.events.publish(PluginEvent::EnablementChanged {
            plugin_id,
            effective: EffectiveEnablement::Enabled,
        });
        Ok(())
    }

    async fn disable(&self, plugin_id: PluginId) -> Result<(), PluginError> {
        let gate = self.mutations.gate(&plugin_id).await;
        let _guard = gate.lock().await;
        self.runtime_control.close_admission(&plugin_id).await?;
        self.state
            .commit(StateMutation::SetEnablement {
                plugin_id: plugin_id.clone(),
                enablement: UserEnablement::Disabled,
                advance_epoch: true,
            })
            .await?;
        self.scan_and_reconcile().await?;
        let result = self
            .runtime_control
            .stop_and_reap(&plugin_id, StopReason::Disable)
            .await;
        if result.is_ok() {
            self.events.publish(PluginEvent::EnablementChanged {
                plugin_id,
                effective: EffectiveEnablement::Disabled(EffectiveDisableReason::User),
            });
        }
        result
    }

    async fn uninstall(&self, plugin_id: PluginId) -> Result<(), PluginError> {
        let gate = self.mutations.gate(&plugin_id).await;
        let _guard = gate.lock().await;
        self.runtime_control.close_admission(&plugin_id).await?;
        let snapshot = self.state.snapshot().await?;
        let record = snapshot
            .plugins
            .get(&plugin_id)
            .ok_or_else(|| PluginError::NotFound {
                plugin_id: plugin_id.clone(),
            })?
            .installation
            .clone();
        let operation_id = new_operation_id();
        let operation_id_for_event = operation_id.clone();
        let trash_location = operation_id.as_str().to_string();
        let mut pending = PendingRemoval {
            operation_id: operation_id.clone(),
            plugin_id: plugin_id.clone(),
            expected_digest: record.content_digest.clone(),
            install_operation_id: record.install_operation_id.clone(),
            trash_location: trash_location.clone(),
            phase: PendingRemovalPhase::Prepared,
        };
        self.state
            .commit(StateMutation::AddPending {
                operation: PendingOperation::Remove(pending.clone()),
            })
            .await?;
        self.scan_and_reconcile().await?;
        self.runtime_control
            .stop_and_reap(&plugin_id, StopReason::Uninstall)
            .await?;

        let _write_permit = self.package_store.write_permit().await;
        let final_path = self.config.plugins_dir().join(plugin_id.as_str());
        let trash_path = self.config.trash_dir().join(&trash_location);
        if final_path.exists() {
            std::fs::rename(&final_path, &trash_path).map_err(|_| PluginError::RemovalPending {
                plugin_id: plugin_id.clone(),
            })?;
        }
        ensure_removal_marker(
            &trash_path,
            &RemovalMarker {
                marker_version: 1,
                removal_operation_id: operation_id.clone(),
                plugin_id: plugin_id.clone(),
                expected_digest: record.content_digest,
                install_operation_id: record.install_operation_id,
            },
        )?;
        pending.phase = PendingRemovalPhase::FilesMoved;
        self.state
            .commit(StateMutation::ReplacePending {
                operation: PendingOperation::Remove(pending),
            })
            .await?;
        self.state
            .commit(StateMutation::CompleteRemoval {
                plugin_id: plugin_id.clone(),
                operation_id,
            })
            .await?;
        let _ = SafeTreeDeleter::new(self.config.trash_dir()).delete(&trash_path);
        drop(_write_permit);
        self.scan_and_reconcile().await?;
        self.events.publish(PluginEvent::InstallProgress {
            operation_id: operation_id_for_event,
            phase: InstallPhase::Removed,
        });
        Ok(())
    }

    async fn set_launch_grant(&self, grant: PluginLaunchGrant) -> Result<(), PluginError> {
        let plugin_id = grant.plugin_id.clone();
        let gate = self.mutations.gate(&plugin_id).await;
        let _guard = gate.lock().await;
        self.runtime_control.close_admission(&plugin_id).await?;
        self.runtime_control
            .stop_and_reap(&plugin_id, StopReason::GrantChanged)
            .await?;
        self.state
            .commit(StateMutation::SetLaunchGrant { grant })
            .await?;
        self.runtime_control.open_admission(&plugin_id).await?;
        Ok(())
    }

    async fn get_launch_grant(
        &self,
        plugin_id: &PluginId,
    ) -> Result<Option<PluginLaunchGrant>, PluginError> {
        Ok(self
            .state
            .snapshot()
            .await?
            .launch_grants
            .get(plugin_id)
            .cloned())
    }

    async fn revoke_launch_grant(&self, plugin_id: PluginId) -> Result<(), PluginError> {
        let gate = self.mutations.gate(&plugin_id).await;
        let _guard = gate.lock().await;
        self.runtime_control.close_admission(&plugin_id).await?;
        self.runtime_control
            .stop_and_reap(&plugin_id, StopReason::GrantChanged)
            .await?;
        self.state
            .commit(StateMutation::RevokeLaunchGrant {
                plugin_id: plugin_id.clone(),
            })
            .await?;
        self.runtime_control.open_admission(&plugin_id).await?;
        Ok(())
    }

    async fn reset_crash_loop(&self, plugin_id: PluginId) -> Result<(), PluginError> {
        let gate = self.mutations.gate(&plugin_id).await;
        let _guard = gate.lock().await;
        self.state
            .commit(StateMutation::ResetCrashLoop {
                plugin_id: plugin_id.clone(),
            })
            .await?;
        self.runtime_control.reset_crash_loop(&plugin_id).await?;
        self.runtime_control.open_admission(&plugin_id).await?;
        self.scan_and_reconcile().await?;
        Ok(())
    }

    async fn remove_plugin_data(
        &self,
        plugin_id: PluginId,
        scope: DataRemovalScope,
    ) -> Result<(), PluginError> {
        let gate = self.mutations.gate(&plugin_id).await;
        let _guard = gate.lock().await;
        self.runtime_control.close_admission(&plugin_id).await?;
        self.runtime_control
            .stop_and_reap(&plugin_id, StopReason::ManualStop)
            .await?;
        let snapshot = self.state.snapshot().await?;
        let record = snapshot
            .plugins
            .get(&plugin_id)
            .ok_or_else(|| PluginError::NotFound {
                plugin_id: plugin_id.clone(),
            })?;
        let plugin_data_root = self.config.plugin_data_dir().join(plugin_id.as_str());
        let target = match scope {
            DataRemovalScope::CurrentContentOwner => {
                plugin_data_root.join(record.installation.content_owner.as_str())
            }
            DataRemovalScope::AllOwners => plugin_data_root,
        };
        SafeTreeDeleter::new(self.config.plugin_data_dir())
            .delete(&target)
            .map_err(|_| PluginError::Internal {
                message: "safe plugin-data deletion failed".to_owned(),
            })?;
        self.runtime_control.open_admission(&plugin_id).await
    }
}

impl<Clock, Control> crate::RuntimeAdmissionProvider for PluginManagementService<Clock, Control>
where
    Clock: AuthorityClock,
    Control: PluginRuntimeControl,
{
    async fn admit(&self, plugin_id: &PluginId) -> Result<ValidatedLaunchDescriptor, PluginError> {
        self.admitted_descriptor(plugin_id).await
    }

    async fn recheck_after_activate(
        &self,
        descriptor: &ValidatedLaunchDescriptor,
    ) -> Result<(), PluginError> {
        let current = self.admitted_descriptor(&descriptor.plugin_id).await?;
        if current.enablement_epoch != descriptor.enablement_epoch
            || current.registry_revision != descriptor.registry_revision
            || current.content_digest != descriptor.content_digest
            || current.content_owner != descriptor.content_owner
        {
            return Err(PluginError::Disabled {
                plugin_id: descriptor.plugin_id.clone(),
                reason: EffectiveDisableReason::Policy,
            });
        }
        Ok(())
    }
}

impl PluginManagementService<crate::SystemAuthorityClock, NoopPluginRuntimeControl> {
    /// Convenience bootstrap for management-only tests that must prove zero spawn behavior.
    pub async fn bootstrap_without_runtime(
        config: PluginManagerConfig,
    ) -> Result<Self, PluginError> {
        Self::bootstrap(
            config,
            crate::SystemAuthorityClock::new(),
            NoopPluginRuntimeControl,
            BTreeMap::new(),
        )
        .await
    }
}

/// Generates an independent mutation identity for removal journals.
fn new_operation_id() -> OperationId {
    OperationId::parse(uuid::Uuid::new_v4().hyphenated().to_string())
        .unwrap_or_else(|error| panic!("generated operation id must be valid: {error}"))
}

/// Generates a Host audit identity for one trusted discovery selection.
fn new_audit_id() -> CandidateAuditId {
    CandidateAuditId::parse(uuid::Uuid::new_v4().hyphenated().to_string())
        .unwrap_or_else(|error| panic!("generated audit id must be valid: {error}"))
}

/// Removes OS details from the stable management error surface.
fn internal_io(error: std::io::Error) -> PluginError {
    PluginError::Internal {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{PluginManagement, PluginManagementService};
    use crate::PluginManagerConfig;
    use ora_plugin_protocol::PluginId;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    /// A valid Workbench remains catalog-visible and cannot persist Enabled intent.
    #[tokio::test]
    async fn management_only_bootstrap_is_fail_closed() {
        let root =
            TempDir::new().unwrap_or_else(|error| panic!("expected data directory: {error}"));
        let service = PluginManagementService::bootstrap_without_runtime(PluginManagerConfig::new(
            root.path(),
        ))
        .await
        .unwrap_or_else(|error| panic!("expected management bootstrap: {error}"));
        assert_eq!(
            service
                .scan_installed()
                .await
                .unwrap_or_else(|error| panic!("expected catalog: {error}"))
                .entries,
            Vec::new()
        );
        let missing = PluginId::parse("ora.missing")
            .unwrap_or_else(|error| panic!("expected plugin id: {error}"));
        assert!(service.enable(missing).await.is_err());
    }
}
