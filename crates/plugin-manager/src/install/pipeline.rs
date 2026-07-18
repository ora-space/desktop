use crate::{
    AuthorizedCandidate, InstallReceipt, InstallSource, InstalledRecord, ManagerLease,
    PackageTreeMode, PackageValidator, PendingInstall, PendingInstallPhase, PendingOperation,
    PluginDiagnostic, PluginDiagnosticCode, PluginError, PluginManagerConfig, StateMutation,
    StateStore, ValidationTarget, audit_no_named_streams, compute_tree_digest,
    current_source_identity,
};
use ora_plugin_protocol::{ContentOwnerId, JsonSafeU64, OperationId, PluginId};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, RwLock};

/// The committed installed+disabled result returned before any separate enable action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPlugin {
    pub plugin_id: PluginId,
    pub record: InstalledRecord,
    pub receipt: InstallReceipt,
    pub location: PathBuf,
}

/// Coordinates stable installed-root read snapshots with commit/maintenance visibility changes.
#[derive(Debug, Default)]
pub struct PackageStoreCoordinator {
    visibility: RwLock<()>,
}

/// Provides one in-process mutation gate for each canonical plugin identity.
#[derive(Debug, Default)]
pub struct PluginMutationCoordinator {
    gates: Mutex<HashMap<PluginId, Arc<Mutex<()>>>>,
}

impl PluginMutationCoordinator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the shared gate used by install, enablement, grants, removal, and data deletion.
    pub async fn gate(&self, plugin_id: &PluginId) -> Arc<Mutex<()>> {
        let mut gates = self.gates.lock().await;
        gates
            .entry(plugin_id.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }
}

impl PackageStoreCoordinator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Prevents final-directory visibility changes while an installed scan builds its snapshot.
    pub(crate) async fn read_permit(&self) -> tokio::sync::RwLockReadGuard<'_, ()> {
        self.visibility.read().await
    }

    /// Serializes final/trash visibility changes with installed catalog snapshots.
    pub(crate) async fn write_permit(&self) -> tokio::sync::RwLockWriteGuard<'_, ()> {
        self.visibility.write().await
    }
}

/// Executes the digest-bound same-volume install journal and atomic final rename.
pub struct PluginInstaller {
    config: PluginManagerConfig,
    validator: PackageValidator,
    state: StateStore,
    lease: Arc<ManagerLease>,
    package_store: Arc<PackageStoreCoordinator>,
    mutations: Arc<PluginMutationCoordinator>,
}

impl PluginInstaller {
    pub fn new(
        config: PluginManagerConfig,
        validator: PackageValidator,
        state: StateStore,
        lease: Arc<ManagerLease>,
        package_store: Arc<PackageStoreCoordinator>,
        mutations: Arc<PluginMutationCoordinator>,
    ) -> Self {
        Self {
            config,
            validator,
            state,
            lease,
            package_store,
            mutations,
        }
    }

    /// Installs reviewed bytes into a new managed final directory and always leaves them disabled.
    pub async fn install_authorized_candidate(
        &self,
        candidate: AuthorizedCandidate,
    ) -> Result<InstalledPlugin, PluginError> {
        self.lease.assert_held()?;
        let gate = self.mutations.gate(&candidate.plugin_id).await;
        let _plugin_guard = gate.lock().await;
        let final_path = self.config.plugins_dir().join(candidate.plugin_id.as_str());
        if final_path.exists() {
            return Err(PluginError::AlreadyInstalled {
                plugin_id: candidate.plugin_id,
            });
        }
        if current_source_identity(&candidate.source_root)? != candidate.source_identity {
            return Err(PluginError::SourceChanged {
                reason: crate::SourceChangeReason::RootIdentityMismatch,
            });
        }
        let source_package = self
            .validator
            .validate(&candidate.source_root, ValidationTarget::Candidate)
            .map_err(source_validation_error)?;
        require_authorized_identity(&source_package, &candidate)?;

        let operation_id = new_operation_id();
        let staging_path = self.config.staging_dir().join(operation_id.as_str());
        std::fs::create_dir(&staging_path).map_err(internal_io)?;
        copy_fresh_package_files(
            &candidate.source_root,
            &staging_path,
            &source_package.digest.files,
        )?;

        if current_source_identity(&candidate.source_root)? != candidate.source_identity {
            return Err(PluginError::SourceChanged {
                reason: crate::SourceChangeReason::RootIdentityMismatch,
            });
        }
        let source_after_copy = self
            .validator
            .validate(&candidate.source_root, ValidationTarget::Candidate)
            .map_err(source_validation_error)?;
        require_authorized_identity(&source_after_copy, &candidate)?;
        let staging_package = self
            .validator
            .validate(
                &staging_path,
                ValidationTarget::Staging {
                    reviewed_id: &candidate.plugin_id,
                    reviewed_version: &candidate.plugin_version,
                    reviewed_digest: &candidate.content_digest,
                },
            )
            .map_err(source_validation_error)?;

        let content_owner = content_owner_from_digest(&candidate.content_digest)?;
        let receipt = build_receipt(&candidate, &staging_package, operation_id.clone())?;
        write_receipt(&staging_path, &receipt)?;
        let post_receipt_digest = compute_tree_digest(
            &staging_path,
            &self.config.limits,
            PackageTreeMode::InstalledContent,
        )
        .map_err(|error| PluginError::UnsupportedPackageLayout {
            diagnostics: vec![PluginDiagnostic::new(
                PluginDiagnosticCode::UnsafeFilesystemObject,
                error.to_string(),
            )],
        })?;
        if post_receipt_digest.digest != candidate.content_digest {
            return Err(PluginError::SourceChanged {
                reason: crate::SourceChangeReason::ContentDigestMismatch,
            });
        }

        let pending = PendingInstall {
            operation_id: operation_id.clone(),
            plugin_id: candidate.plugin_id.clone(),
            expected_version: candidate.plugin_version.clone(),
            expected_digest: candidate.content_digest.clone(),
            candidate_audit_id: candidate.audit_id,
            phase: PendingInstallPhase::Prepared,
        };
        let _visibility_guard = self.package_store.visibility.write().await;
        self.state
            .commit(StateMutation::AddPending {
                operation: PendingOperation::Install(pending.clone()),
            })
            .await?;
        if final_path.exists() {
            return Err(PluginError::InstallConflict {
                plugin_id: candidate.plugin_id,
            });
        }
        std::fs::rename(&staging_path, &final_path).map_err(|error| {
            recovery_error(
                operation_id.clone(),
                format!("final directory rename failed: {error}"),
            )
        })?;

        let mut files_committed = pending;
        files_committed.phase = PendingInstallPhase::FilesCommitted;
        self.state
            .commit(StateMutation::ReplacePending {
                operation: PendingOperation::Install(files_committed),
            })
            .await
            .map_err(|error| {
                recovery_error(
                    operation_id.clone(),
                    format!("files committed but state phase failed: {error}"),
                )
            })?;
        let record = InstalledRecord {
            plugin_version: candidate.plugin_version,
            content_digest: candidate.content_digest,
            content_owner,
            install_operation_id: operation_id.clone(),
        };
        self.state
            .commit(StateMutation::CompleteInstall {
                plugin_id: candidate.plugin_id.clone(),
                record: record.clone(),
                operation_id: operation_id.clone(),
            })
            .await
            .map_err(|error| {
                recovery_error(
                    operation_id,
                    format!("installed files require state recovery: {error}"),
                )
            })?;

        Ok(InstalledPlugin {
            plugin_id: candidate.plugin_id,
            record,
            receipt,
            location: final_path,
        })
    }
}

/// Requires the second source proof to equal every reviewed identity field.
fn require_authorized_identity(
    package: &crate::ValidatedPackage,
    candidate: &AuthorizedCandidate,
) -> Result<(), PluginError> {
    if package.manifest.ora.id() != &candidate.plugin_id
        || package.manifest.version != candidate.plugin_version
        || package.digest.digest != candidate.content_digest
    {
        return Err(PluginError::SourceChanged {
            reason: crate::SourceChangeReason::ContentDigestMismatch,
        });
    }
    Ok(())
}

/// Copies only reviewed regular-file streams into create-new staging files.
fn copy_fresh_package_files(
    source_root: &Path,
    staging_root: &Path,
    files: &[crate::DigestFileProof],
) -> Result<(), PluginError> {
    let mut buffer = [0u8; 64 * 1024];
    for proof in files {
        let relative = Path::new(&proof.relative_path);
        let source_path = source_root.join(relative);
        let destination_path = staging_root.join(relative);
        audit_no_named_streams(&source_path).map_err(|_| PluginError::SourceChanged {
            reason: crate::SourceChangeReason::RootIdentityMismatch,
        })?;
        if let Some(parent) = destination_path.parent() {
            std::fs::create_dir_all(parent).map_err(internal_io)?;
        }
        let mut source = open_source_no_follow(&source_path)?;
        let mut destination = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination_path)
            .map_err(internal_io)?;
        loop {
            let read = source.read(&mut buffer).map_err(internal_io)?;
            if read == 0 {
                break;
            }
            destination
                .write_all(&buffer[..read])
                .map_err(internal_io)?;
        }
        destination.sync_all().map_err(internal_io)?;
        audit_no_named_streams(&source_path).map_err(|_| PluginError::SourceChanged {
            reason: crate::SourceChangeReason::RootIdentityMismatch,
        })?;
        audit_no_named_streams(&destination_path).map_err(|_| PluginError::Internal {
            message: "fresh staging file stream audit failed".to_owned(),
        })?;
    }
    Ok(())
}

#[cfg(windows)]
fn open_source_no_follow(path: &Path) -> Result<File, PluginError> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(internal_io)
}

#[cfg(not(windows))]
fn open_source_no_follow(path: &Path) -> Result<File, PluginError> {
    File::open(path).map_err(internal_io)
}

/// Builds the Host receipt only after staging validation proves digest equality.
fn build_receipt(
    candidate: &AuthorizedCandidate,
    package: &crate::ValidatedPackage,
    operation_id: OperationId,
) -> Result<InstallReceipt, PluginError> {
    let installed_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| PluginError::Internal {
            message: error.to_string(),
        })?
        .as_millis();
    let installed_at = u64::try_from(installed_at).map_err(|_| PluginError::Internal {
        message: "system time exceeds supported JSON integer".to_string(),
    })?;
    Ok(InstallReceipt {
        receipt_version: 1,
        plugin_id: candidate.plugin_id.clone(),
        plugin_version: candidate.plugin_version.clone(),
        source: InstallSource::LocalDirectory,
        installed_at_unix_ms: JsonSafeU64::new(installed_at).map_err(internal_value)?,
        content_digest: candidate.content_digest.clone(),
        file_count: JsonSafeU64::new(package.digest.file_count).map_err(internal_value)?,
        total_bytes: JsonSafeU64::new(package.digest.total_bytes).map_err(internal_value)?,
        operation_id,
    })
}

/// Creates and flushes Host-owned receipt metadata inside staging.
fn write_receipt(staging_root: &Path, receipt: &InstallReceipt) -> Result<(), PluginError> {
    let ora_dir = staging_root.join(".ora");
    std::fs::create_dir(&ora_dir).map_err(internal_io)?;
    let receipt_path = ora_dir.join("receipt.json");
    let bytes = serde_json::to_vec(receipt).map_err(|error| PluginError::Internal {
        message: error.to_string(),
    })?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(receipt_path)
        .map_err(internal_io)?;
    file.write_all(&bytes).map_err(internal_io)?;
    file.sync_all().map_err(internal_io)
}

/// Converts a content receipt digest into its Windows path-safe storage-owner identity.
pub(crate) fn content_owner_from_digest(
    digest: &ora_plugin_protocol::ContentDigest,
) -> Result<ContentOwnerId, PluginError> {
    let hex = digest
        .as_str()
        .strip_prefix("sha256:")
        .ok_or_else(|| PluginError::Internal {
            message: "content digest has invalid prefix".to_string(),
        })?;
    ContentOwnerId::parse(format!("sha256-{hex}")).map_err(internal_value)
}

/// Generates an independent transaction identity unrelated to plugin identity.
fn new_operation_id() -> OperationId {
    OperationId::parse(uuid::Uuid::new_v4().hyphenated().to_string())
        .unwrap_or_else(|error| panic!("generated install operation id must be valid: {error}"))
}

/// Normalizes validation details into a stable install boundary error.
fn source_validation_error(error: crate::PackageValidationError) -> PluginError {
    PluginError::SourceChanged {
        reason: match error {
            crate::PackageValidationError::SourceChanged => {
                crate::SourceChangeReason::ContentDigestMismatch
            }
            _ => crate::SourceChangeReason::StagingValidationFailed,
        },
    }
}

/// Creates a journal-aware recovery result after a commit boundary may have advanced.
fn recovery_error(operation_id: OperationId, message: String) -> PluginError {
    PluginError::RecoveryRequired {
        operation_id,
        diagnostic: PluginDiagnostic::new(PluginDiagnosticCode::RecoveryRequired, message),
    }
}

/// Converts leaf validation errors into bounded internal failures.
fn internal_value(error: impl std::fmt::Display) -> PluginError {
    PluginError::Internal {
        message: error.to_string(),
    }
}

/// Converts local I/O failures without leaking package contents.
fn internal_io(error: std::io::Error) -> PluginError {
    PluginError::Internal {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{PackageStoreCoordinator, PluginInstaller};
    use crate::{
        CandidateAuthority, FileStatePersistence, ManagementSessionId, ManagerLease,
        PackageValidator, PluginManagerConfig, StateRecoverySource, StateStore, UserEnablement,
    };
    use ora_plugin_protocol::CandidateAuditId;
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::sync::Arc;
    use tempfile::TempDir;

    /// Runs the complete reviewed-source to journaled installed+disabled commit path.
    #[tokio::test]
    async fn installs_authorized_candidate_disabled() {
        let data =
            TempDir::new().unwrap_or_else(|error| panic!("expected data directory: {error}"));
        let source =
            TempDir::new().unwrap_or_else(|error| panic!("expected source directory: {error}"));
        fs::create_dir(source.path().join("dist"))
            .unwrap_or_else(|error| panic!("expected dist directory: {error}"));
        fs::write(source.path().join("dist/index.js"), "export default {};")
            .unwrap_or_else(|error| panic!("expected entry write: {error}"));
        fs::write(
            source.path().join("package.json"),
            r#"{"name":"@ora/example","version":"0.1.0","type":"module","ora":{"manifestVersion":1,"id":"ora.example","displayName":"Example","kind":"agent","main":"dist/index.js","engines":{"ora":">=0.1.0 <0.2.0","pluginApi":1,"bun":">=1.0.0 <2.0.0"},"contributes":{"agents":[{"id":"example","displayName":"Example","contractVersion":1}]}}}"#,
        )
        .unwrap_or_else(|error| panic!("expected manifest write: {error}"));

        let config = PluginManagerConfig::new(data.path());
        let lease = Arc::new(
            ManagerLease::acquire(&config)
                .unwrap_or_else(|error| panic!("expected manager lease: {error}")),
        );
        let persistence = FileStatePersistence::new(config.plugin_system_dir());
        let recovery = persistence
            .load_or_recover()
            .await
            .unwrap_or_else(|error| panic!("expected fresh state: {error}"));
        assert_eq!(recovery.source, StateRecoverySource::Fresh);
        let state = StateStore::start(recovery.snapshot, persistence, 16);
        let validator = PackageValidator::new(
            config.limits.clone(),
            config.host_version.clone(),
            config.bun_version.clone(),
        );
        let authority = CandidateAuthority::new(
            crate::SystemAuthorityClock::new(),
            config.selection_ttl,
            config.candidate_ttl,
        );
        let session = ManagementSessionId::new_random()
            .unwrap_or_else(|error| panic!("expected management session: {error}"));
        let selection = authority
            .register_selection(
                session.clone(),
                source.path(),
                CandidateAuditId::parse("550e8400-e29b-41d4-a716-446655440000")
                    .unwrap_or_else(|error| panic!("expected audit id: {error}")),
            )
            .unwrap_or_else(|error| panic!("expected selection: {error}"));
        let identified = authority
            .identify(&session, selection.selection_handle, &validator)
            .unwrap_or_else(|error| panic!("expected identify: {error}"));
        let authorized = authority
            .consume_candidate(&session, identified.candidate_handle)
            .unwrap_or_else(|error| panic!("expected candidate authority: {error}"));
        let installer = PluginInstaller::new(
            config,
            validator,
            state.clone(),
            lease,
            Arc::new(PackageStoreCoordinator::new()),
            Arc::new(crate::PluginMutationCoordinator::new()),
        );
        let installed = installer
            .install_authorized_candidate(authorized)
            .await
            .unwrap_or_else(|error| panic!("expected install: {error}"));
        let snapshot = state
            .snapshot()
            .await
            .unwrap_or_else(|error| panic!("expected state snapshot: {error}"));
        assert!(installed.location.exists());
        assert_eq!(
            snapshot
                .plugins
                .get(&installed.plugin_id)
                .map(|record| record.user_enablement),
            Some(UserEnablement::Disabled)
        );
        assert_eq!(snapshot.pending_operations, Vec::new());
    }
}
