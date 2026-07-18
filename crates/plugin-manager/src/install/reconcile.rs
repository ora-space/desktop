use crate::{
    InstallReceipt, InstalledRecord, PackageStoreCoordinator, PackageValidator, PendingInstall,
    PendingInstallPhase, PendingOperation, PendingRemoval, PendingRemovalPhase, PluginDiagnostic,
    PluginDiagnosticCode, PluginError, PluginManagerConfig, RemovalMarker, SafeTreeDeleter,
    StateMutation, StateStore, ValidationTarget, content_owner_from_digest, parse_install_receipt,
    parse_removal_marker,
};
use ora_plugin_protocol::OperationId;
use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Reconciles install and removal journals against managed disk facts before readiness.
pub struct InstallReconciler {
    config: PluginManagerConfig,
    validator: PackageValidator,
    state: StateStore,
    package_store: Arc<PackageStoreCoordinator>,
}

impl InstallReconciler {
    pub fn new(
        config: PluginManagerConfig,
        validator: PackageValidator,
        state: StateStore,
        package_store: Arc<PackageStoreCoordinator>,
    ) -> Self {
        Self {
            config,
            validator,
            state,
            package_store,
        }
    }

    /// Makes every known journal converge from actual final/staging/trash visibility.
    pub async fn reconcile(&self) -> Result<(), PluginError> {
        let _visibility_guard = self.package_store.write_permit().await;
        let initial = self.state.snapshot().await?;
        for operation in initial.pending_operations {
            match operation {
                PendingOperation::Install(install) => self.reconcile_install(install).await?,
                PendingOperation::Remove(removal) => self.reconcile_removal(removal).await?,
            }
        }
        self.cleanup_orphan_staging().await?;
        self.cleanup_orphan_trash().await?;
        Ok(())
    }

    /// Adopts only a final or staging tree whose fresh proof matches the pending install.
    async fn reconcile_install(&self, pending: PendingInstall) -> Result<(), PluginError> {
        let staging = self
            .config
            .staging_dir()
            .join(pending.operation_id.as_str());
        let final_path = self.config.plugins_dir().join(pending.plugin_id.as_str());
        let staging_exists = staging.exists();
        let final_exists = final_path.exists();
        if staging_exists && final_exists {
            return Err(recovery_required(
                pending.operation_id,
                "install staging and final directory both exist",
            ));
        }
        if !staging_exists && !final_exists {
            self.state
                .commit(StateMutation::AbortPending {
                    operation_id: pending.operation_id,
                })
                .await?;
            return Ok(());
        }

        let managed_path = if final_exists { &final_path } else { &staging };
        let receipt = read_install_receipt(managed_path, &pending.operation_id)?;
        let record = install_record_from_pending(&pending)?;
        self.validator
            .validate(
                managed_path,
                ValidationTarget::RecoveryManaged {
                    expected_id: &pending.plugin_id,
                    receipt: &receipt,
                    record: &record,
                },
            )
            .map_err(|_| {
                recovery_required(
                    pending.operation_id.clone(),
                    "pending install bytes do not match journal and receipt",
                )
            })?;

        if staging_exists {
            std::fs::rename(&staging, &final_path).map_err(|_| {
                recovery_required(
                    pending.operation_id.clone(),
                    "pending install could not commit staging to final",
                )
            })?;
            let mut committed = pending.clone();
            committed.phase = PendingInstallPhase::FilesCommitted;
            self.state
                .commit(StateMutation::ReplacePending {
                    operation: PendingOperation::Install(committed),
                })
                .await?;
        }
        self.state
            .commit(StateMutation::CompleteInstall {
                plugin_id: pending.plugin_id,
                record,
                operation_id: pending.operation_id,
            })
            .await?;
        Ok(())
    }

    /// Completes a tombstoned removal without ever restoring admission or installed state.
    async fn reconcile_removal(&self, mut pending: PendingRemoval) -> Result<(), PluginError> {
        validate_trash_location(&pending)?;
        let final_path = self.config.plugins_dir().join(pending.plugin_id.as_str());
        let trash_path = self.config.trash_dir().join(&pending.trash_location);
        let final_exists = final_path.exists();
        let trash_exists = trash_path.exists();
        if final_exists && trash_exists {
            return Err(recovery_required(
                pending.operation_id,
                "removal final and matching trash directory both exist",
            ));
        }
        if final_exists {
            verify_removal_tree(&self.validator, &final_path, &pending)?;
            std::fs::rename(&final_path, &trash_path).map_err(|_| PluginError::RemovalPending {
                plugin_id: pending.plugin_id.clone(),
            })?;
        }
        if final_exists || trash_exists {
            verify_removal_tree(&self.validator, &trash_path, &pending)?;
            ensure_removal_marker(&trash_path, &removal_marker(&pending))?;
            pending.phase = PendingRemovalPhase::FilesMoved;
            self.state
                .commit(StateMutation::ReplacePending {
                    operation: PendingOperation::Remove(pending.clone()),
                })
                .await?;
        }
        self.state
            .commit(StateMutation::CompleteRemoval {
                plugin_id: pending.plugin_id,
                operation_id: pending.operation_id,
            })
            .await?;
        if trash_path.exists() {
            let _ = SafeTreeDeleter::new(self.config.trash_dir()).delete(&trash_path);
        }
        Ok(())
    }

    /// Deletes only exact operation-named staging objects that have no active journal.
    async fn cleanup_orphan_staging(&self) -> Result<(), PluginError> {
        let active = active_operation_ids(&self.state.snapshot().await?, OperationKind::Install);
        for entry in read_managed_directory(&self.config.staging_dir())? {
            let Some(operation_id) = parse_operation_entry(&entry) else {
                continue;
            };
            if active.contains(&operation_id) {
                continue;
            }
            SafeTreeDeleter::new(self.config.staging_dir())
                .delete(&entry)
                .map_err(|_| {
                    recovery_required(operation_id, "orphan staging cleanup was unsafe")
                })?;
        }
        Ok(())
    }

    /// Deletes orphan trash only when its receipt and Host marker prove the same operation.
    async fn cleanup_orphan_trash(&self) -> Result<(), PluginError> {
        let active = active_operation_ids(&self.state.snapshot().await?, OperationKind::Remove);
        for entry in read_managed_directory(&self.config.trash_dir())? {
            let Some(operation_id) = parse_operation_entry(&entry) else {
                continue;
            };
            if active.contains(&operation_id) {
                continue;
            }
            if !orphan_trash_is_authorized(&self.validator, &entry, &operation_id) {
                continue;
            }
            SafeTreeDeleter::new(self.config.trash_dir())
                .delete(&entry)
                .map_err(|_| recovery_required(operation_id, "orphan trash cleanup was unsafe"))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
enum OperationKind {
    Install,
    Remove,
}

/// Returns current operation ids of one closed journal variant.
fn active_operation_ids(
    snapshot: &crate::PluginStateSnapshot,
    kind: OperationKind,
) -> BTreeSet<OperationId> {
    snapshot
        .pending_operations
        .iter()
        .filter_map(|operation| match (kind, operation) {
            (OperationKind::Install, PendingOperation::Install(install)) => {
                Some(install.operation_id.clone())
            }
            (OperationKind::Remove, PendingOperation::Remove(removal)) => {
                Some(removal.operation_id.clone())
            }
            (OperationKind::Install, PendingOperation::Remove(_))
            | (OperationKind::Remove, PendingOperation::Install(_)) => None,
        })
        .collect()
}

/// Lists one managed maintenance directory without treating absence as corruption.
fn read_managed_directory(path: &Path) -> Result<Vec<PathBuf>, PluginError> {
    match std::fs::read_dir(path) {
        Ok(entries) => entries
            .map(|entry| entry.map(|entry| entry.path()).map_err(internal_io))
            .collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(internal_io(error)),
    }
}

/// Accepts only a canonical OperationId used verbatim as the direct-child name.
fn parse_operation_entry(path: &Path) -> Option<OperationId> {
    let name = path.file_name()?.to_str()?;
    let operation_id = OperationId::parse(name).ok()?;
    (operation_id.as_str() == name).then_some(operation_id)
}

/// Prevents a persisted trash location from becoming an arbitrary path join.
fn validate_trash_location(pending: &PendingRemoval) -> Result<(), PluginError> {
    let parsed = OperationId::parse(&pending.trash_location).ok();
    if parsed.as_ref() != Some(&pending.operation_id)
        || pending.trash_location != pending.operation_id.as_str()
    {
        return Err(recovery_required(
            pending.operation_id.clone(),
            "pending removal has an invalid managed trash location",
        ));
    }
    Ok(())
}

/// Derives the only state record that can be adopted from a pending install.
fn install_record_from_pending(pending: &PendingInstall) -> Result<InstalledRecord, PluginError> {
    Ok(InstalledRecord {
        plugin_version: pending.expected_version.clone(),
        content_digest: pending.expected_digest.clone(),
        content_owner: content_owner_from_digest(&pending.expected_digest)?,
        install_operation_id: pending.operation_id.clone(),
    })
}

/// Reads a strict receipt and binds its operation to the journal before any adoption.
fn read_install_receipt(
    root: &Path,
    operation_id: &OperationId,
) -> Result<InstallReceipt, PluginError> {
    let bytes = std::fs::read(root.join(".ora").join("receipt.json"))
        .map_err(|_| recovery_required(operation_id.clone(), "install receipt is missing"))?;
    parse_install_receipt(&bytes)
        .map_err(|_| recovery_required(operation_id.clone(), "install receipt is invalid"))
}

/// Rebuilds the installed record from receipt fields constrained by a removal tombstone.
fn removal_record(
    pending: &PendingRemoval,
    receipt: &InstallReceipt,
) -> Result<InstalledRecord, PluginError> {
    if receipt.plugin_id != pending.plugin_id
        || receipt.content_digest != pending.expected_digest
        || receipt.operation_id != pending.install_operation_id
    {
        return Err(recovery_required(
            pending.operation_id.clone(),
            "removal receipt does not match tombstone",
        ));
    }
    Ok(InstalledRecord {
        plugin_version: receipt.plugin_version.clone(),
        content_digest: pending.expected_digest.clone(),
        content_owner: content_owner_from_digest(&pending.expected_digest)?,
        install_operation_id: pending.install_operation_id.clone(),
    })
}

/// Proves code in final or trash still matches the exact removal tombstone.
fn verify_removal_tree(
    validator: &PackageValidator,
    root: &Path,
    pending: &PendingRemoval,
) -> Result<(), PluginError> {
    let receipt = read_install_receipt(root, &pending.operation_id)?;
    let record = removal_record(pending, &receipt)?;
    validator
        .validate(
            root,
            ValidationTarget::RecoveryManaged {
                expected_id: &pending.plugin_id,
                receipt: &receipt,
                record: &record,
            },
        )
        .map(|_| ())
        .map_err(|_| {
            recovery_required(
                pending.operation_id.clone(),
                "removal package no longer matches its receipt",
            )
        })
}

/// Constructs the exact Host marker bound to one pending removal.
fn removal_marker(pending: &PendingRemoval) -> RemovalMarker {
    RemovalMarker {
        marker_version: 1,
        removal_operation_id: pending.operation_id.clone(),
        plugin_id: pending.plugin_id.clone(),
        expected_digest: pending.expected_digest.clone(),
        install_operation_id: pending.install_operation_id.clone(),
    }
}

/// Creates a flushed marker or accepts an already flushed byte-equivalent marker.
pub(crate) fn ensure_removal_marker(
    root: &Path,
    expected: &RemovalMarker,
) -> Result<(), PluginError> {
    let marker_path = root.join(".ora").join("removal.json");
    if marker_path.exists() {
        let bytes = std::fs::read(&marker_path).map_err(internal_io)?;
        let actual = parse_removal_marker(&bytes).map_err(|_| {
            recovery_required(
                expected.removal_operation_id.clone(),
                "existing removal marker is invalid",
            )
        })?;
        if actual != *expected {
            return Err(recovery_required(
                expected.removal_operation_id.clone(),
                "existing removal marker does not match tombstone",
            ));
        }
        return Ok(());
    }
    let bytes = serde_json::to_vec(expected).map_err(|error| PluginError::Internal {
        message: error.to_string(),
    })?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(marker_path)
        .map_err(internal_io)?;
    file.write_all(&bytes).map_err(internal_io)?;
    file.sync_all().map_err(internal_io)
}

/// Validates a marker-only trash object before granting recursive-delete authority.
fn orphan_trash_is_authorized(
    validator: &PackageValidator,
    root: &Path,
    operation_id: &OperationId,
) -> bool {
    let marker_bytes = match std::fs::read(root.join(".ora").join("removal.json")) {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };
    let marker = match parse_removal_marker(&marker_bytes) {
        Ok(marker) if marker.removal_operation_id == *operation_id => marker,
        Ok(_) | Err(_) => return false,
    };
    let receipt = match read_install_receipt(root, operation_id) {
        Ok(receipt) => receipt,
        Err(_) => return false,
    };
    let pending = PendingRemoval {
        operation_id: operation_id.clone(),
        plugin_id: marker.plugin_id.clone(),
        expected_digest: marker.expected_digest.clone(),
        install_operation_id: marker.install_operation_id.clone(),
        trash_location: operation_id.as_str().to_owned(),
        phase: PendingRemovalPhase::FilesMoved,
    };
    let record = match removal_record(&pending, &receipt) {
        Ok(record) => record,
        Err(_) => return false,
    };
    validator
        .validate(
            root,
            ValidationTarget::RecoveryManaged {
                expected_id: &marker.plugin_id,
                receipt: &receipt,
                record: &record,
            },
        )
        .is_ok()
}

/// Produces a bounded fail-closed recovery diagnostic for one exact operation.
fn recovery_required(operation_id: OperationId, message: &str) -> PluginError {
    PluginError::RecoveryRequired {
        operation_id,
        diagnostic: PluginDiagnostic::new(PluginDiagnosticCode::RecoveryRequired, message),
    }
}

/// Converts local maintenance I/O failures without exposing package contents.
fn internal_io(error: std::io::Error) -> PluginError {
    PluginError::Internal {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::InstallReconciler;
    use crate::{
        CrashPolicy, InstallReceipt, InstallSource, InstalledRecord, PackageStoreCoordinator,
        PackageTreeMode, PackageValidator, PendingInstall, PendingInstallPhase, PendingOperation,
        PendingRemoval, PendingRemovalPhase, PluginManagerConfig, PluginStateRecord,
        PluginStateSnapshot, StatePersistence, StateStore, UserEnablement, compute_tree_digest,
        content_owner_from_digest,
    };
    use ora_plugin_protocol::{
        CandidateAuditId, JsonSafeU64, OperationId, PluginId, PluginVersion,
    };
    use pretty_assertions::assert_eq;
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::sync::Arc;
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

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum InstallRestartCase {
        NoBytes,
        StagingOnly,
        FinalOnly,
        StagingAndFinal,
        CorruptFinal,
    }

    /// Exercises every install visibility combination from a durable Prepared journal.
    #[tokio::test]
    async fn install_restart_matrix_converges_or_fails_closed() {
        for case in [
            InstallRestartCase::NoBytes,
            InstallRestartCase::StagingOnly,
            InstallRestartCase::FinalOnly,
            InstallRestartCase::StagingAndFinal,
            InstallRestartCase::CorruptFinal,
        ] {
            let data = TempDir::new()
                .unwrap_or_else(|error| panic!("expected {case:?} data root: {error}"));
            let config = create_layout(data.path());
            let plugin_id = plugin_id();
            let operation_id = operation_id("550e8400-e29b-41d4-a716-446655440000");
            let staging_path = config.staging_dir().join(operation_id.as_str());
            let final_path = config.plugins_dir().join(plugin_id.as_str());
            let receipt = write_managed_package(&config, &staging_path, &plugin_id, &operation_id);
            match case {
                InstallRestartCase::NoBytes => std::fs::remove_dir_all(&staging_path)
                    .unwrap_or_else(|error| panic!("expected staging removal: {error}")),
                InstallRestartCase::StagingOnly => {}
                InstallRestartCase::FinalOnly | InstallRestartCase::CorruptFinal => {
                    std::fs::rename(&staging_path, &final_path)
                        .unwrap_or_else(|error| panic!("expected final fixture rename: {error}"));
                    if case == InstallRestartCase::CorruptFinal {
                        std::fs::write(
                            final_path.join("dist").join("index.js"),
                            "export default { corrupt: true };",
                        )
                        .unwrap_or_else(|error| panic!("expected final corruption: {error}"));
                    }
                }
                InstallRestartCase::StagingAndFinal => {
                    write_managed_package(&config, &final_path, &plugin_id, &operation_id);
                }
            }
            let pending = PendingInstall {
                operation_id: operation_id.clone(),
                plugin_id: plugin_id.clone(),
                expected_version: receipt.plugin_version,
                expected_digest: receipt.content_digest,
                candidate_audit_id: CandidateAuditId::parse("b982ef1d-ea31-44c4-983d-4ed47a35e1a4")
                    .unwrap_or_else(|error| panic!("expected audit id: {error}")),
                phase: PendingInstallPhase::Prepared,
            };
            let mut initial = PluginStateSnapshot::empty();
            initial
                .pending_operations
                .push(PendingOperation::Install(pending));
            let state = StateStore::start(initial, MemoryPersistence, 8);
            let result = reconciler(&config, state.clone()).reconcile().await;

            match case {
                InstallRestartCase::NoBytes
                | InstallRestartCase::StagingOnly
                | InstallRestartCase::FinalOnly => {
                    result.unwrap_or_else(|error| {
                        panic!("expected {case:?} install convergence: {error}")
                    });
                    let snapshot = state
                        .snapshot()
                        .await
                        .unwrap_or_else(|error| panic!("expected {case:?} snapshot: {error}"));
                    assert_eq!(snapshot.pending_operations, Vec::new());
                    assert_eq!(
                        snapshot
                            .plugins
                            .get(&plugin_id)
                            .map(|record| record.user_enablement),
                        (case != InstallRestartCase::NoBytes).then_some(UserEnablement::Disabled)
                    );
                    if case == InstallRestartCase::NoBytes {
                        assert!(!final_path.exists());
                    } else {
                        assert!(final_path.exists());
                    }
                }
                InstallRestartCase::StagingAndFinal | InstallRestartCase::CorruptFinal => {
                    assert_recovery_operation(result, &operation_id);
                    let snapshot = state
                        .snapshot()
                        .await
                        .unwrap_or_else(|error| panic!("expected {case:?} snapshot: {error}"));
                    assert_eq!(snapshot.pending_operations.len(), 1);
                    assert_eq!(snapshot.plugins, BTreeMap::new());
                }
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum RemovalRestartCase {
        FinalOnly,
        TrashOnly,
        NoBytes,
        FinalAndTrash,
        CorruptTrash,
    }

    /// Exercises all final/trash combinations without ever reviving a tombstoned plugin.
    #[tokio::test]
    async fn removal_restart_matrix_converges_or_fails_closed() {
        for case in [
            RemovalRestartCase::FinalOnly,
            RemovalRestartCase::TrashOnly,
            RemovalRestartCase::NoBytes,
            RemovalRestartCase::FinalAndTrash,
            RemovalRestartCase::CorruptTrash,
        ] {
            let data = TempDir::new()
                .unwrap_or_else(|error| panic!("expected {case:?} data root: {error}"));
            let config = create_layout(data.path());
            let plugin_id = plugin_id();
            let install_id = operation_id("550e8400-e29b-41d4-a716-446655440000");
            let removal_id = operation_id("6d432fa4-2c47-49d9-8028-e8712c32cc04");
            let final_path = config.plugins_dir().join(plugin_id.as_str());
            let trash_path = config.trash_dir().join(removal_id.as_str());
            let receipt = write_managed_package(&config, &final_path, &plugin_id, &install_id);
            let record = InstalledRecord {
                plugin_version: receipt.plugin_version,
                content_digest: receipt.content_digest.clone(),
                content_owner: content_owner_from_digest(&receipt.content_digest)
                    .unwrap_or_else(|error| panic!("expected content owner: {error}")),
                install_operation_id: install_id.clone(),
            };
            let pending = PendingRemoval {
                operation_id: removal_id.clone(),
                plugin_id: plugin_id.clone(),
                expected_digest: receipt.content_digest,
                install_operation_id: install_id.clone(),
                trash_location: removal_id.as_str().to_owned(),
                phase: PendingRemovalPhase::Prepared,
            };
            let mut initial = PluginStateSnapshot::empty();
            initial.plugins.insert(
                plugin_id.clone(),
                PluginStateRecord {
                    user_enablement: UserEnablement::Enabled,
                    installation: record,
                    crash_policy: CrashPolicy::normal(),
                    enablement_epoch: JsonSafeU64::new(1)
                        .unwrap_or_else(|error| panic!("expected epoch: {error}")),
                },
            );
            initial
                .pending_operations
                .push(PendingOperation::Remove(pending));
            match case {
                RemovalRestartCase::FinalOnly => {}
                RemovalRestartCase::TrashOnly | RemovalRestartCase::CorruptTrash => {
                    std::fs::rename(&final_path, &trash_path)
                        .unwrap_or_else(|error| panic!("expected trash fixture rename: {error}"));
                    if case == RemovalRestartCase::CorruptTrash {
                        std::fs::write(
                            trash_path.join("dist").join("index.js"),
                            "export default { corrupt: true };",
                        )
                        .unwrap_or_else(|error| panic!("expected trash corruption: {error}"));
                    }
                }
                RemovalRestartCase::NoBytes => std::fs::remove_dir_all(&final_path)
                    .unwrap_or_else(|error| panic!("expected final fixture removal: {error}")),
                RemovalRestartCase::FinalAndTrash => {
                    write_managed_package(&config, &trash_path, &plugin_id, &install_id);
                }
            }
            let state = StateStore::start(initial, MemoryPersistence, 8);
            let result = reconciler(&config, state.clone()).reconcile().await;

            match case {
                RemovalRestartCase::FinalOnly
                | RemovalRestartCase::TrashOnly
                | RemovalRestartCase::NoBytes => {
                    result.unwrap_or_else(|error| {
                        panic!("expected {case:?} removal convergence: {error}")
                    });
                    let snapshot = state
                        .snapshot()
                        .await
                        .unwrap_or_else(|error| panic!("expected {case:?} snapshot: {error}"));
                    assert_eq!(snapshot.pending_operations, Vec::new());
                    assert_eq!(snapshot.plugins, BTreeMap::new());
                    assert!(!final_path.exists());
                    assert!(!trash_path.exists());
                }
                RemovalRestartCase::FinalAndTrash | RemovalRestartCase::CorruptTrash => {
                    assert_recovery_operation(result, &removal_id);
                    let snapshot = state
                        .snapshot()
                        .await
                        .unwrap_or_else(|error| panic!("expected {case:?} snapshot: {error}"));
                    assert_eq!(snapshot.pending_operations.len(), 1);
                    assert!(snapshot.plugins.contains_key(&plugin_id));
                }
            }
        }
    }

    /// Requires a fail-closed reconciliation result to retain its exact operation identity.
    fn assert_recovery_operation(result: Result<(), crate::PluginError>, expected: &OperationId) {
        match result {
            Err(crate::PluginError::RecoveryRequired { operation_id, .. }) => {
                assert_eq!(&operation_id, expected);
            }
            other => panic!("expected recovery-required result, got {other:?}"),
        }
    }

    /// A crash after final rename adopts only matching bytes and keeps the plugin disabled.
    #[tokio::test]
    async fn adopts_matching_final_from_prepared_install() {
        let data = TempDir::new().unwrap_or_else(|error| panic!("expected data root: {error}"));
        let config = create_layout(data.path());
        let plugin_id = plugin_id();
        let operation_id = operation_id("550e8400-e29b-41d4-a716-446655440000");
        let final_path = config.plugins_dir().join(plugin_id.as_str());
        let receipt = write_managed_package(&config, &final_path, &plugin_id, &operation_id);
        let pending = PendingInstall {
            operation_id: operation_id.clone(),
            plugin_id: plugin_id.clone(),
            expected_version: receipt.plugin_version.clone(),
            expected_digest: receipt.content_digest.clone(),
            candidate_audit_id: CandidateAuditId::parse("b982ef1d-ea31-44c4-983d-4ed47a35e1a4")
                .unwrap_or_else(|error| panic!("expected audit id: {error}")),
            phase: PendingInstallPhase::Prepared,
        };
        let mut initial = PluginStateSnapshot::empty();
        initial
            .pending_operations
            .push(PendingOperation::Install(pending));
        let state = StateStore::start(initial, MemoryPersistence, 8);
        reconciler(&config, state.clone())
            .reconcile()
            .await
            .unwrap_or_else(|error| panic!("expected install recovery: {error}"));

        let snapshot = state
            .snapshot()
            .await
            .unwrap_or_else(|error| panic!("expected recovered state: {error}"));
        assert_eq!(snapshot.pending_operations, Vec::new());
        assert_eq!(
            snapshot
                .plugins
                .get(&plugin_id)
                .map(|record| record.user_enablement),
            Some(UserEnablement::Disabled)
        );
    }

    /// A crash after final-to-trash rename completes logical removal and marker cleanup.
    #[tokio::test]
    async fn completes_removal_from_matching_trash() {
        let data = TempDir::new().unwrap_or_else(|error| panic!("expected data root: {error}"));
        let config = create_layout(data.path());
        let plugin_id = plugin_id();
        let install_id = operation_id("550e8400-e29b-41d4-a716-446655440000");
        let removal_id = operation_id("6d432fa4-2c47-49d9-8028-e8712c32cc04");
        let trash_path = config.trash_dir().join(removal_id.as_str());
        let receipt = write_managed_package(&config, &trash_path, &plugin_id, &install_id);
        let record = InstalledRecord {
            plugin_version: receipt.plugin_version,
            content_digest: receipt.content_digest.clone(),
            content_owner: content_owner_from_digest(&receipt.content_digest)
                .unwrap_or_else(|error| panic!("expected content owner: {error}")),
            install_operation_id: install_id.clone(),
        };
        let pending = PendingRemoval {
            operation_id: removal_id.clone(),
            plugin_id: plugin_id.clone(),
            expected_digest: receipt.content_digest,
            install_operation_id: install_id,
            trash_location: removal_id.as_str().to_owned(),
            phase: PendingRemovalPhase::Prepared,
        };
        let mut initial = PluginStateSnapshot::empty();
        initial.plugins.insert(
            plugin_id.clone(),
            PluginStateRecord {
                user_enablement: UserEnablement::Enabled,
                installation: record,
                crash_policy: CrashPolicy::normal(),
                enablement_epoch: JsonSafeU64::new(1)
                    .unwrap_or_else(|error| panic!("expected epoch: {error}")),
            },
        );
        initial
            .pending_operations
            .push(PendingOperation::Remove(pending));
        let state = StateStore::start(initial, MemoryPersistence, 8);
        reconciler(&config, state.clone())
            .reconcile()
            .await
            .unwrap_or_else(|error| panic!("expected removal recovery: {error}"));

        let snapshot = state
            .snapshot()
            .await
            .unwrap_or_else(|error| panic!("expected recovered state: {error}"));
        assert_eq!(snapshot.pending_operations, Vec::new());
        assert_eq!(snapshot.plugins, BTreeMap::new());
        assert!(!trash_path.exists());
    }

    /// Creates every root normally pinned by ManagerLease for isolated recovery tests.
    fn create_layout(root: &Path) -> PluginManagerConfig {
        let config = PluginManagerConfig::new(root);
        for path in [
            config.plugins_dir(),
            config.staging_dir(),
            config.trash_dir(),
        ] {
            std::fs::create_dir_all(path)
                .unwrap_or_else(|error| panic!("expected managed directory: {error}"));
        }
        config
    }

    /// Builds the production validator used by the reconciler.
    fn reconciler(config: &PluginManagerConfig, state: StateStore) -> InstallReconciler {
        InstallReconciler::new(
            config.clone(),
            PackageValidator::new(
                config.limits.clone(),
                config.host_version.clone(),
                config.bun_version.clone(),
            ),
            state,
            Arc::new(PackageStoreCoordinator::new()),
        )
    }

    /// Writes immutable package bytes and the exact Host receipt used by recovery.
    fn write_managed_package(
        config: &PluginManagerConfig,
        root: &Path,
        plugin_id: &PluginId,
        operation_id: &OperationId,
    ) -> InstallReceipt {
        std::fs::create_dir_all(root.join("dist"))
            .unwrap_or_else(|error| panic!("expected package directory: {error}"));
        std::fs::write(root.join("dist").join("index.js"), "export default {};")
            .unwrap_or_else(|error| panic!("expected entry write: {error}"));
        std::fs::write(
            root.join("package.json"),
            format!(
                r#"{{"name":"@ora/example","version":"0.1.0","type":"module","ora":{{"manifestVersion":1,"id":"{}","displayName":"Example","kind":"agent","main":"dist/index.js","engines":{{"ora":">=0.1.0 <0.2.0","pluginApi":1,"bun":">=1.0.0 <2.0.0"}},"contributes":{{"agents":[{{"id":"example","displayName":"Example","contractVersion":1}}]}}}}}}"#,
                plugin_id.as_str()
            ),
        )
        .unwrap_or_else(|error| panic!("expected manifest write: {error}"));
        let proof = compute_tree_digest(root, &config.limits, PackageTreeMode::Candidate)
            .unwrap_or_else(|error| panic!("expected tree proof: {error}"));
        let receipt = InstallReceipt {
            receipt_version: 1,
            plugin_id: plugin_id.clone(),
            plugin_version: PluginVersion::parse("0.1.0")
                .unwrap_or_else(|error| panic!("expected version: {error}")),
            source: InstallSource::LocalDirectory,
            installed_at_unix_ms: JsonSafeU64::new(1)
                .unwrap_or_else(|error| panic!("expected timestamp: {error}")),
            content_digest: proof.digest,
            file_count: JsonSafeU64::new(proof.file_count)
                .unwrap_or_else(|error| panic!("expected file count: {error}")),
            total_bytes: JsonSafeU64::new(proof.total_bytes)
                .unwrap_or_else(|error| panic!("expected byte count: {error}")),
            operation_id: operation_id.clone(),
        };
        std::fs::create_dir(root.join(".ora"))
            .unwrap_or_else(|error| panic!("expected metadata directory: {error}"));
        std::fs::write(
            root.join(".ora").join("receipt.json"),
            serde_json::to_vec(&receipt)
                .unwrap_or_else(|error| panic!("expected receipt bytes: {error}")),
        )
        .unwrap_or_else(|error| panic!("expected receipt write: {error}"));
        receipt
    }

    fn plugin_id() -> PluginId {
        PluginId::parse("ora.example").unwrap_or_else(|error| panic!("expected plugin id: {error}"))
    }

    fn operation_id(value: &str) -> OperationId {
        OperationId::parse(value).unwrap_or_else(|error| panic!("expected operation id: {error}"))
    }
}
