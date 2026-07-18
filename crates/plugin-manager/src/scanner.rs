use crate::{
    CatalogEntry, CompatibilityReason, InstallReceipt, IntegrityStatus, ManifestValidity,
    PackageStoreCoordinator, PackageValidationError, PackageValidator, PluginCatalogSnapshot,
    PluginDiagnostic, PluginDiagnosticCode, PluginManagerConfig, PluginStateSnapshot,
    RuntimeCompatibility, RuntimeSupport, StateStore, ValidatedPackage, ValidationTarget,
    parse_install_receipt,
};
use ora_plugin_protocol::{
    JsonSafeU64, PluginId, PluginKind, PluginManifest, PluginPackageManifest, parse_plugin_manifest,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Installed scan output includes fresh proofs for internal admission but exposes only catalog DTOs.
#[derive(Debug, Clone)]
pub struct InstalledScan {
    pub catalog: PluginCatalogSnapshot,
    pub validated: BTreeMap<PluginId, ValidatedPackage>,
    pub state: PluginStateSnapshot,
}

/// Builds revisioned installed catalog snapshots without mutating or repairing package state.
pub struct InstalledScanner {
    config: PluginManagerConfig,
    validator: PackageValidator,
    state: StateStore,
    package_store: Arc<PackageStoreCoordinator>,
    publication: Mutex<CatalogPublication>,
}

struct CatalogPublication {
    revision: JsonSafeU64,
    entries: Vec<CatalogEntry>,
}

impl InstalledScanner {
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
            publication: Mutex::new(CatalogPublication {
                revision: JsonSafeU64::new(0)
                    .unwrap_or_else(|error| panic!("zero catalog revision must be valid: {error}")),
                entries: Vec::new(),
            }),
        }
    }

    /// Scans one stable final-directory/state revision view and retains all invalid entries.
    pub async fn scan_installed(&self) -> Result<InstalledScan, crate::PluginError> {
        let _permit = self.package_store.read_permit().await;
        let state = self.state.snapshot().await?;
        let plugins_dir = self.config.plugins_dir();
        let validator = self.validator.clone();
        let state_for_worker = state.clone();
        let facts = tokio::task::spawn_blocking(move || {
            scan_installed_blocking(&plugins_dir, &validator, &state_for_worker)
        })
        .await
        .map_err(|error| crate::PluginError::Internal {
            message: format!("installed scan worker failed: {error}"),
        })??;
        let mut publication = self.publication.lock().await;
        if publication.entries != facts.entries {
            publication.revision = publication.revision.checked_increment().map_err(|error| {
                crate::PluginError::Internal {
                    message: error.to_string(),
                }
            })?;
            publication.entries = facts.entries.clone();
        }
        Ok(InstalledScan {
            catalog: PluginCatalogSnapshot {
                revision: publication.revision,
                entries: facts.entries,
            },
            validated: facts.validated,
            state,
        })
    }
}

struct ScanFacts {
    entries: Vec<CatalogEntry>,
    validated: BTreeMap<PluginId, ValidatedPackage>,
}

/// Performs blocking filesystem enumeration inside the scanner's dedicated worker.
fn scan_installed_blocking(
    plugins_dir: &Path,
    validator: &PackageValidator,
    state: &PluginStateSnapshot,
) -> Result<ScanFacts, crate::PluginError> {
    let mut entries = Vec::new();
    let mut validated = BTreeMap::new();
    let mut seen_ids = BTreeSet::new();
    let directory_entries = std::fs::read_dir(plugins_dir).map_err(internal_io)?;
    for directory_entry in directory_entries {
        let directory_entry = directory_entry.map_err(internal_io)?;
        let path = directory_entry.path();
        let name = directory_entry.file_name().to_string_lossy().into_owned();
        if matches!(name.as_str(), ".staging" | ".trash") {
            continue;
        }
        let parsed_id = PluginId::parse(name.clone()).ok();
        if let Some(plugin_id) = &parsed_id {
            seen_ids.insert(plugin_id.clone());
        }
        let manifest = read_manifest_for_catalog(&path);
        let Some(plugin_id) = parsed_id else {
            entries.push(invalid_entry(
                path,
                manifest,
                PluginDiagnosticCode::InvalidManifest,
                "final directory name is not a canonical plugin id",
            ));
            continue;
        };
        let Some(record) = state.plugins.get(&plugin_id) else {
            entries.push(untracked_entry(path, manifest, plugin_id));
            continue;
        };
        let receipt = match read_receipt(&path) {
            Ok(receipt) => receipt,
            Err(diagnostic) => {
                entries.push(CatalogEntry {
                    plugin_id: Some(plugin_id),
                    location: path,
                    manifest,
                    validity: ManifestValidity::Valid,
                    compatibility: RuntimeCompatibility::Compatible,
                    support: RuntimeSupport::Supported,
                    integrity: IntegrityStatus::MissingReceipt,
                    diagnostics: vec![diagnostic],
                });
                continue;
            }
        };
        match validator.validate(
            &path,
            ValidationTarget::Installed {
                receipt: &receipt,
                record: &record.installation,
            },
        ) {
            Ok(package) => {
                entries.push(catalog_from_validated(&package));
                validated.insert(plugin_id, package);
            }
            Err(error) => entries.push(validation_failure_entry(path, manifest, plugin_id, error)),
        }
    }

    for plugin_id in state.plugins.keys() {
        if !seen_ids.contains(plugin_id) {
            entries.push(CatalogEntry {
                plugin_id: Some(plugin_id.clone()),
                location: plugins_dir.join(plugin_id.as_str()),
                manifest: None,
                validity: ManifestValidity::Invalid,
                compatibility: RuntimeCompatibility::Compatible,
                support: RuntimeSupport::Supported,
                integrity: IntegrityStatus::StateMismatch,
                diagnostics: vec![PluginDiagnostic::new(
                    PluginDiagnosticCode::MissingInstallFiles,
                    "state record exists but final package directory is missing",
                )],
            });
        }
    }
    entries.sort_by(|left, right| left.location.cmp(&right.location));
    Ok(ScanFacts { entries, validated })
}

/// Reads a bounded manifest independently so invalid integrity remains diagnosable.
fn read_manifest_for_catalog(path: &Path) -> Option<PluginPackageManifest> {
    let bytes = std::fs::read(path.join("package.json")).ok()?;
    parse_plugin_manifest(&bytes).ok()
}

/// Reads the Host-owned receipt through its strict parser.
fn read_receipt(path: &Path) -> Result<InstallReceipt, PluginDiagnostic> {
    let bytes = std::fs::read(path.join(".ora").join("receipt.json")).map_err(|_| {
        PluginDiagnostic::new(
            PluginDiagnosticCode::MissingReceipt,
            "install receipt is missing",
        )
    })?;
    parse_install_receipt(&bytes).map_err(|error| {
        PluginDiagnostic::new(PluginDiagnosticCode::InvalidReceipt, error.to_string())
    })
}

/// Projects a fresh successful proof into the public diagnostic catalog.
fn catalog_from_validated(package: &ValidatedPackage) -> CatalogEntry {
    CatalogEntry {
        plugin_id: Some(package.manifest.ora.id().clone()),
        location: package.root.clone(),
        manifest: Some(package.manifest.clone()),
        validity: package.validity.clone(),
        compatibility: package.compatibility.clone(),
        support: package.support.clone(),
        integrity: package.integrity.clone(),
        diagnostics: package.diagnostics.clone(),
    }
}

/// Keeps an untracked final visible but never grants receipt-only adoption.
fn untracked_entry(
    path: PathBuf,
    manifest: Option<PluginPackageManifest>,
    plugin_id: PluginId,
) -> CatalogEntry {
    let (compatibility, support) = manifest_statuses(manifest.as_ref());
    CatalogEntry {
        plugin_id: Some(plugin_id),
        location: path,
        validity: if manifest.is_some() {
            ManifestValidity::Valid
        } else {
            ManifestValidity::Invalid
        },
        manifest,
        compatibility,
        support,
        integrity: IntegrityStatus::StateMismatch,
        diagnostics: vec![PluginDiagnostic::new(
            PluginDiagnosticCode::UntrackedInstall,
            "final package has no matching persisted install intent or state record",
        )],
    }
}

/// Maps hard proof failures to a non-running catalog entry instead of hiding the directory.
fn validation_failure_entry(
    path: PathBuf,
    manifest: Option<PluginPackageManifest>,
    plugin_id: PluginId,
    error: PackageValidationError,
) -> CatalogEntry {
    let validity = if manifest.is_some() {
        ManifestValidity::Valid
    } else {
        ManifestValidity::Invalid
    };
    let (compatibility, support) = manifest_statuses(manifest.as_ref());
    let integrity = match error {
        PackageValidationError::InstalledFactsMismatch => IntegrityStatus::StateMismatch,
        _ => IntegrityStatus::DigestMismatch,
    };
    CatalogEntry {
        plugin_id: Some(plugin_id),
        location: path,
        manifest,
        validity,
        compatibility,
        support,
        integrity,
        diagnostics: vec![PluginDiagnostic::new(
            PluginDiagnosticCode::IntegrityMismatch,
            error.to_string(),
        )],
    }
}

/// Builds an invalid entry for names or objects that cannot enter identity matching.
fn invalid_entry(
    path: PathBuf,
    manifest: Option<PluginPackageManifest>,
    code: PluginDiagnosticCode,
    message: &str,
) -> CatalogEntry {
    CatalogEntry {
        plugin_id: manifest.as_ref().map(|value| value.ora.id().clone()),
        location: path,
        manifest,
        validity: ManifestValidity::Invalid,
        compatibility: RuntimeCompatibility::Incompatible(CompatibilityReason::OraVersion),
        support: RuntimeSupport::UnsupportedSchemaVersion {
            manifest_version: 0,
        },
        integrity: IntegrityStatus::StateMismatch,
        diagnostics: vec![PluginDiagnostic::new(code, message)],
    }
}

/// Derives support for diagnostic-only entries whose full installed proof failed.
fn manifest_statuses(
    manifest: Option<&PluginPackageManifest>,
) -> (RuntimeCompatibility, RuntimeSupport) {
    match manifest.map(|value| &value.ora) {
        Some(PluginManifest::Agent { .. }) => {
            (RuntimeCompatibility::Compatible, RuntimeSupport::Supported)
        }
        Some(PluginManifest::Workbench { .. }) => (
            RuntimeCompatibility::Compatible,
            RuntimeSupport::UnsupportedKind {
                kind: PluginKind::Workbench,
            },
        ),
        None => (
            RuntimeCompatibility::Incompatible(CompatibilityReason::OraVersion),
            RuntimeSupport::UnsupportedSchemaVersion {
                manifest_version: 0,
            },
        ),
    }
}

/// Hides platform-specific read_dir details behind the stable management boundary.
fn internal_io(error: std::io::Error) -> crate::PluginError {
    crate::PluginError::Internal {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::InstalledScanner;
    use crate::{
        FileStatePersistence, ManagerLease, PackageStoreCoordinator, PackageValidator,
        PluginManagerConfig, StateStore,
    };
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::sync::Arc;
    use tempfile::TempDir;

    /// An orphan final remains visible as untracked and never enters validated admission proofs.
    #[tokio::test]
    async fn reports_untracked_final_directory() {
        let root =
            TempDir::new().unwrap_or_else(|error| panic!("expected data directory: {error}"));
        let config = PluginManagerConfig::new(root.path());
        let _lease = ManagerLease::acquire(&config)
            .unwrap_or_else(|error| panic!("expected lease: {error}"));
        let final_path = config.plugins_dir().join("ora.orphan");
        fs::create_dir(&final_path)
            .unwrap_or_else(|error| panic!("expected orphan directory: {error}"));
        let persistence = FileStatePersistence::new(config.plugin_system_dir());
        let recovery = persistence
            .load_or_recover()
            .await
            .unwrap_or_else(|error| panic!("expected state recovery: {error}"));
        let state = StateStore::start(recovery.snapshot, persistence, 8);
        let scanner = InstalledScanner::new(
            config.clone(),
            PackageValidator::new(
                config.limits.clone(),
                config.host_version.clone(),
                config.bun_version.clone(),
            ),
            state,
            Arc::new(PackageStoreCoordinator::new()),
        );
        let scan = scanner
            .scan_installed()
            .await
            .unwrap_or_else(|error| panic!("expected installed scan: {error}"));
        let repeated = scanner
            .scan_installed()
            .await
            .unwrap_or_else(|error| panic!("expected repeated installed scan: {error}"));
        assert_eq!(scan.catalog.entries.len(), 1);
        assert!(scan.validated.is_empty());
        assert_eq!(scan.catalog, repeated.catalog);
    }
}
