use crate::{
    PluginError, PluginStateSnapshot, StatePersistence, UserEnablement, audit_regular_file,
};
use ora_plugin_protocol::{OperationId, parse_strict_json};
use std::fs::OpenOptions;
use std::future::Future;
use std::io::Write;
use std::path::{Path, PathBuf};

/// File-backed primary/backup state persistence rooted under the leased plugin-system directory.
#[derive(Debug, Clone)]
pub struct FileStatePersistence {
    plugin_system_dir: PathBuf,
}

impl FileStatePersistence {
    pub fn new(plugin_system_dir: impl Into<PathBuf>) -> Self {
        Self {
            plugin_system_dir: plugin_system_dir.into(),
        }
    }

    pub fn primary_path(&self) -> PathBuf {
        self.plugin_system_dir.join("state.json")
    }

    pub fn backup_path(&self) -> PathBuf {
        self.plugin_system_dir.join("state.previous.json")
    }

    /// Loads a valid primary or performs fail-closed backup recovery before readiness.
    pub async fn load_or_recover(&self) -> Result<StateRecovery, PluginError> {
        let persistence = self.clone();
        tokio::task::spawn_blocking(move || persistence.load_or_recover_blocking())
            .await
            .map_err(|error| PluginError::Internal {
                message: format!("state recovery worker failed: {error}"),
            })?
    }

    /// Performs strict primary/backup arbitration while holding the ManagerLease externally.
    fn load_or_recover_blocking(&self) -> Result<StateRecovery, PluginError> {
        std::fs::create_dir_all(&self.plugin_system_dir).map_err(internal_io)?;
        match read_snapshot(&self.primary_path()) {
            SnapshotRead::Valid(snapshot) => Ok(StateRecovery {
                snapshot,
                source: StateRecoverySource::Primary,
            }),
            SnapshotRead::Unsupported(version) => Err(PluginError::StateVersionUnsupported {
                schema_version: version,
            }),
            SnapshotRead::Missing | SnapshotRead::Corrupt => {
                let primary_was_corrupt = matches!(
                    read_snapshot(&self.primary_path()),
                    SnapshotRead::Corrupt | SnapshotRead::Unsupported(_)
                );
                match read_snapshot(&self.backup_path()) {
                    SnapshotRead::Valid(mut snapshot) => {
                        for record in snapshot.plugins.values_mut() {
                            record.user_enablement = UserEnablement::Disabled;
                        }
                        snapshot.launch_grants.clear();
                        if primary_was_corrupt {
                            quarantine_primary(&self.primary_path())?;
                        }
                        install_new_primary(&self.primary_path(), &snapshot)?;
                        Ok(StateRecovery {
                            snapshot,
                            source: StateRecoverySource::SanitizedBackup,
                        })
                    }
                    SnapshotRead::Unsupported(version) => {
                        Err(PluginError::StateVersionUnsupported {
                            schema_version: version,
                        })
                    }
                    SnapshotRead::Missing if !primary_was_corrupt => {
                        let snapshot = PluginStateSnapshot::empty();
                        install_new_primary(&self.primary_path(), &snapshot)?;
                        Ok(StateRecovery {
                            snapshot,
                            source: StateRecoverySource::Fresh,
                        })
                    }
                    SnapshotRead::Missing | SnapshotRead::Corrupt => Err(PluginError::StateCorrupt),
                }
            }
        }
    }
}

impl StatePersistence for FileStatePersistence {
    fn commit(
        &mut self,
        previous: &PluginStateSnapshot,
        candidate: &PluginStateSnapshot,
    ) -> impl Future<Output = Result<(), PluginError>> + Send {
        let persistence = self.clone();
        let previous = previous.clone();
        let candidate = candidate.clone();
        async move {
            tokio::task::spawn_blocking(move || {
                replace_snapshot(&persistence.backup_path(), &previous)?;
                replace_snapshot(&persistence.primary_path(), &candidate)?;
                let verified = match read_snapshot(&persistence.primary_path()) {
                    SnapshotRead::Valid(snapshot) => snapshot,
                    SnapshotRead::Missing
                    | SnapshotRead::Corrupt
                    | SnapshotRead::Unsupported(_) => return Err(persistence_uncertain()),
                };
                if verified != candidate {
                    return Err(persistence_uncertain());
                }
                Ok(())
            })
            .await
            .map_err(|error| PluginError::Internal {
                message: format!("state persistence worker failed: {error}"),
            })?
        }
    }
}

/// Reports which strict input established the ready state snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateRecoverySource {
    Primary,
    SanitizedBackup,
    Fresh,
}

/// A ready snapshot together with its recovery audit classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateRecovery {
    pub snapshot: PluginStateSnapshot,
    pub source: StateRecoverySource,
}

enum SnapshotRead {
    Missing,
    Corrupt,
    Unsupported(u64),
    Valid(PluginStateSnapshot),
}

/// Parses state with duplicate-key/depth protection and exact schema-version routing.
fn read_snapshot(path: &Path) -> SnapshotRead {
    match std::fs::symlink_metadata(path) {
        Ok(_) => {
            if audit_regular_file(path).is_err() {
                return SnapshotRead::Corrupt;
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return SnapshotRead::Missing;
        }
        Err(_) => return SnapshotRead::Corrupt,
    }
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return SnapshotRead::Missing,
        Err(_) => return SnapshotRead::Corrupt,
    };
    if bytes.len() > 16 * 1024 * 1024 {
        return SnapshotRead::Corrupt;
    }
    let value = match parse_strict_json(&bytes, 64) {
        Ok(value) => value,
        Err(_) => return SnapshotRead::Corrupt,
    };
    let version = match value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
    {
        Some(version) => version,
        None => return SnapshotRead::Corrupt,
    };
    if version != 1 {
        return SnapshotRead::Unsupported(version);
    }
    serde_json::from_value(value)
        .map(SnapshotRead::Valid)
        .unwrap_or(SnapshotRead::Corrupt)
}

/// Writes and flushes a create-new sibling before platform-appropriate atomic replacement.
fn replace_snapshot(path: &Path, snapshot: &PluginStateSnapshot) -> Result<(), PluginError> {
    let temp = write_temp(path, snapshot)?;
    if path.exists() {
        audit_regular_file(path).map_err(|_| persistence_uncertain())?;
    }
    audit_regular_file(&temp).map_err(|_| persistence_uncertain())?;
    atomic_replace(&temp, path).map_err(|_| persistence_uncertain())?;
    audit_regular_file(path).map_err(|_| persistence_uncertain())
}

/// Installs a recovery/fresh primary only while the canonical name is absent.
fn install_new_primary(path: &Path, snapshot: &PluginStateSnapshot) -> Result<(), PluginError> {
    if path.exists() {
        return Err(PluginError::StateCorrupt);
    }
    let temp = write_temp(path, snapshot)?;
    audit_regular_file(&temp).map_err(|_| PluginError::StateCorrupt)?;
    atomic_move_no_replace(&temp, path).map_err(internal_io)?;
    audit_regular_file(path).map_err(|_| PluginError::StateCorrupt)
}

/// Creates an unpredictable sibling temp and flushes bytes before exposing a commit point.
fn write_temp(path: &Path, snapshot: &PluginStateSnapshot) -> Result<PathBuf, PluginError> {
    let parent = path.parent().ok_or_else(|| PluginError::Internal {
        message: "state path has no parent".to_string(),
    })?;
    let temp = parent.join(format!(".state.tmp.{}", uuid::Uuid::new_v4()));
    let bytes = serde_json::to_vec(snapshot).map_err(|error| PluginError::Internal {
        message: error.to_string(),
    })?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(internal_io)?;
    file.write_all(&bytes).map_err(internal_io)?;
    file.sync_all().map_err(internal_io)?;
    drop(file);
    audit_regular_file(&temp).map_err(|_| persistence_uncertain())?;
    Ok(temp)
}

/// Isolates corrupt primary bytes under a non-input name before clean recovery creation.
#[cfg(not(windows))]
fn quarantine_primary(path: &Path) -> Result<(), PluginError> {
    let parent = path.parent().ok_or(PluginError::StateCorrupt)?;
    let quarantine = parent.join(format!("state.corrupt.{}.json", uuid::Uuid::new_v4()));
    atomic_move_no_replace(path, &quarantine).map_err(|_| PluginError::StateCorrupt)
}

/// Pins and renames the invalid primary by handle so path replacement cannot redirect recovery.
#[cfg(windows)]
fn quarantine_primary(path: &Path) -> Result<(), PluginError> {
    use std::mem::{offset_of, size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CreateFileW, DELETE, FILE_ATTRIBUTE_DIRECTORY,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_RENAME_INFO, FILE_SHARE_READ,
        FILE_SHARE_WRITE, FileRenameInfo, GetFileInformationByHandle, OPEN_EXISTING,
        SetFileInformationByHandle,
    };

    let parent_path = path.parent().ok_or(PluginError::StateCorrupt)?;
    let open = |open_path: &Path, access: u32, flags: u32| -> Result<OwnedHandle, PluginError> {
        let wide: Vec<u16> = open_path.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: the path is NUL-terminated and all pointers are call-scoped.
        let raw = unsafe {
            CreateFileW(
                wide.as_ptr(),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                flags,
                std::ptr::null_mut(),
            )
        };
        if raw == INVALID_HANDLE_VALUE || raw.is_null() {
            return Err(PluginError::StateCorrupt);
        }
        // SAFETY: CreateFileW returned one unique live handle.
        Ok(unsafe { OwnedHandle::from_raw_handle(raw) })
    };
    let parent = open(
        parent_path,
        FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES,
        FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
    )?;
    let primary = open(
        path,
        DELETE | FILE_READ_ATTRIBUTES,
        FILE_FLAG_OPEN_REPARSE_POINT,
    )?;
    let information = |handle: &OwnedHandle| -> Result<BY_HANDLE_FILE_INFORMATION, PluginError> {
        let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { zeroed() };
        // SAFETY: the output structure and handle remain valid for this call.
        if unsafe { GetFileInformationByHandle(handle.as_raw_handle() as HANDLE, &mut information) }
            == 0
        {
            return Err(PluginError::StateCorrupt);
        }
        Ok(information)
    };
    let parent_information = information(&parent)?;
    let primary_information = information(&primary)?;
    if parent_information.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
        || parent_information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || primary_information.dwFileAttributes
            & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)
            != 0
        || primary_information.nNumberOfLinks != 1
    {
        return Err(PluginError::StateCorrupt);
    }
    for _ in 0..8 {
        let name = format!("state.corrupt.{}.json", uuid::Uuid::new_v4());
        let quarantine = parent_path.join(&name);
        let name_wide: Vec<u16> = quarantine.as_os_str().encode_wide().collect();
        let byte_len = name_wide
            .len()
            .checked_mul(size_of::<u16>())
            .ok_or(PluginError::StateCorrupt)?;
        let structure_len = offset_of!(FILE_RENAME_INFO, FileName)
            .checked_add(byte_len)
            .ok_or(PluginError::StateCorrupt)?;
        let word_count = structure_len.div_ceil(size_of::<u64>());
        let mut storage = vec![0_u64; word_count];
        let rename = storage.as_mut_ptr().cast::<FILE_RENAME_INFO>();
        // SAFETY: storage is aligned, sized for the fixed fields and exact UTF-16 payload.
        unsafe {
            (*rename).Anonymous.ReplaceIfExists = false;
            // SetFileInformationByHandle requires the fully qualified target and ignores a root
            // handle; the separately pinned parent keeps that absolute path identity stable.
            (*rename).RootDirectory = std::ptr::null_mut();
            (*rename).FileNameLength =
                u32::try_from(byte_len).map_err(|_| PluginError::StateCorrupt)?;
            std::ptr::copy_nonoverlapping(
                name_wide.as_ptr(),
                std::ptr::addr_of_mut!((*rename).FileName).cast(),
                name_wide.len(),
            );
        }
        // SAFETY: the rename buffer matches FileRenameInfo and both handles stay pinned.
        let succeeded = unsafe {
            SetFileInformationByHandle(
                primary.as_raw_handle() as HANDLE,
                FileRenameInfo,
                rename.cast(),
                u32::try_from(structure_len).map_err(|_| PluginError::StateCorrupt)?,
            )
        };
        if succeeded != 0 {
            let renamed_information = information(&primary)?;
            if renamed_information.dwVolumeSerialNumber != primary_information.dwVolumeSerialNumber
                || renamed_information.nFileIndexHigh != primary_information.nFileIndexHigh
                || renamed_information.nFileIndexLow != primary_information.nFileIndexLow
            {
                return Err(PluginError::StateCorrupt);
            }
            drop(primary);
            let quarantine_metadata =
                std::fs::symlink_metadata(&quarantine).map_err(|_| PluginError::StateCorrupt)?;
            use std::os::windows::fs::MetadataExt;
            if std::fs::symlink_metadata(path).is_ok()
                || !quarantine_metadata.is_file()
                || quarantine_metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            {
                return Err(PluginError::StateCorrupt);
            }
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_ALREADY_EXISTS as i32) {
            return Err(PluginError::StateCorrupt);
        }
    }
    Err(PluginError::StateCorrupt)
}

#[cfg(windows)]
fn atomic_replace(source: &Path, target: &Path) -> std::io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_WRITE_THROUGH, MoveFileExW, ReplaceFileW,
    };

    if target.exists() {
        let target_wide = wide_null(target);
        let source_wide = wide_null(source);
        let succeeded = unsafe {
            ReplaceFileW(
                target_wide.as_ptr(),
                source_wide.as_ptr(),
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if succeeded == 0 {
            return Err(std::io::Error::last_os_error());
        }
        return Ok(());
    }
    let source_wide = wide_null(source);
    let target_wide = wide_null(target);
    let succeeded = unsafe {
        MoveFileExW(
            source_wide.as_ptr(),
            target_wide.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    };
    if succeeded == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, target: &Path) -> std::io::Result<()> {
    std::fs::rename(source, target)
}

#[cfg(windows)]
fn atomic_move_no_replace(source: &Path, target: &Path) -> std::io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};

    let source_wide = wide_null(source);
    let target_wide = wide_null(target);
    let succeeded = unsafe {
        MoveFileExW(
            source_wide.as_ptr(),
            target_wide.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    };
    if succeeded == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
fn atomic_move_no_replace(source: &Path, target: &Path) -> std::io::Result<()> {
    if target.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "target exists",
        ));
    }
    std::fs::rename(source, target)
}

#[cfg(windows)]
fn wide_null(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;

    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// Creates a stable uncertain-commit error with an independent operation identity.
fn persistence_uncertain() -> PluginError {
    let operation_id = OperationId::parse(uuid::Uuid::new_v4().hyphenated().to_string())
        .unwrap_or_else(|error| {
            panic!("generated persistence operation id must be valid: {error}")
        });
    PluginError::PersistenceUncertain { operation_id }
}

/// Preserves bounded local I/O context without leaking state bytes.
fn internal_io(error: std::io::Error) -> PluginError {
    PluginError::Internal {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{FileStatePersistence, StateRecoverySource};
    use crate::{
        CrashPolicy, InstalledRecord, PluginError, PluginLaunchGrant, PluginStateRecord,
        PluginStateSnapshot, StatePersistence, UserEnablement,
    };
    use ora_plugin_protocol::{
        ContentDigest, ContentOwnerId, JsonSafeU64, OperationId, PluginId, PluginVersion,
    };
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    /// Commits primary/backup and reloads the exact current revision.
    #[tokio::test]
    async fn commits_and_loads_primary_state() {
        let root =
            TempDir::new().unwrap_or_else(|error| panic!("expected state directory: {error}"));
        let mut persistence = FileStatePersistence::new(root.path());
        let initial = persistence
            .load_or_recover()
            .await
            .unwrap_or_else(|error| panic!("expected fresh recovery: {error}"));
        assert_eq!(initial.source, StateRecoverySource::Fresh);
        let mut candidate = PluginStateSnapshot::empty();
        candidate.revision = ora_plugin_protocol::JsonSafeU64::new(1)
            .unwrap_or_else(|error| panic!("expected revision: {error}"));
        persistence
            .commit(&initial.snapshot, &candidate)
            .await
            .unwrap_or_else(|error| panic!("expected state commit: {error}"));
        assert_eq!(
            persistence
                .load_or_recover()
                .await
                .unwrap_or_else(|error| panic!("expected primary reload: {error}")),
            super::StateRecovery {
                snapshot: candidate,
                source: StateRecoverySource::Primary,
            }
        );
    }

    /// Quarantines a primary carrying ADS and creates a clean disabled recovery primary.
    #[cfg(windows)]
    #[tokio::test]
    async fn recovers_without_inheriting_primary_named_streams() {
        use crate::audit_regular_file;
        use std::ffi::OsString;
        use std::path::PathBuf;

        let root =
            TempDir::new().unwrap_or_else(|error| panic!("expected state directory: {error}"));
        let mut persistence = FileStatePersistence::new(root.path());
        let initial = persistence
            .load_or_recover()
            .await
            .unwrap_or_else(|error| panic!("expected fresh recovery: {error}"));
        let mut candidate = PluginStateSnapshot::empty();
        candidate.revision = ora_plugin_protocol::JsonSafeU64::new(1)
            .unwrap_or_else(|error| panic!("expected revision: {error}"));
        persistence
            .commit(&initial.snapshot, &candidate)
            .await
            .unwrap_or_else(|error| panic!("expected state commit: {error}"));

        let mut stream_name = OsString::from(persistence.primary_path().as_os_str());
        stream_name.push(":hostile");
        std::fs::write(PathBuf::from(stream_name), b"sentinel")
            .unwrap_or_else(|error| panic!("expected ADS write: {error}"));
        assert_eq!(
            audit_regular_file(&persistence.primary_path()).is_err(),
            true
        );

        let recovered = persistence
            .load_or_recover()
            .await
            .unwrap_or_else(|error| panic!("expected clean backup recovery: {error}"));
        assert_eq!(recovered.source, StateRecoverySource::SanitizedBackup);
        assert_eq!(audit_regular_file(&persistence.primary_path()), Ok(()));
    }

    /// Sanitizes enabled intent and launch references before publishing backup recovery.
    #[tokio::test]
    async fn backup_recovery_disables_plugins_and_clears_launch_grants() {
        let root =
            TempDir::new().unwrap_or_else(|error| panic!("expected state directory: {error}"));
        let mut persistence = FileStatePersistence::new(root.path());
        let initial = persistence
            .load_or_recover()
            .await
            .unwrap_or_else(|error| panic!("expected fresh recovery: {error}"));
        let plugin_id = PluginId::parse("ora.recovery")
            .unwrap_or_else(|error| panic!("expected plugin id: {error}"));
        let content_owner = ContentOwnerId::parse(format!("sha256-{}", "1".repeat(64)))
            .unwrap_or_else(|error| panic!("expected content owner: {error}"));
        let mut with_grant = PluginStateSnapshot::empty();
        with_grant.revision = json_u64(1);
        with_grant.plugins.insert(
            plugin_id.clone(),
            PluginStateRecord {
                user_enablement: UserEnablement::Enabled,
                installation: InstalledRecord {
                    plugin_version: PluginVersion::parse("0.1.0")
                        .unwrap_or_else(|error| panic!("expected version: {error}")),
                    content_digest: ContentDigest::parse(format!("sha256:{}", "1".repeat(64)))
                        .unwrap_or_else(|error| panic!("expected digest: {error}")),
                    content_owner: content_owner.clone(),
                    install_operation_id: operation_id("00000000-0000-4000-8000-000000000001"),
                },
                crash_policy: CrashPolicy::normal(),
                enablement_epoch: json_u64(1),
            },
        );
        with_grant.launch_grants.insert(
            plugin_id.clone(),
            PluginLaunchGrant {
                plugin_id: plugin_id.clone(),
                content_owner,
                schema_version: 1,
                revision: json_u64(1),
                environment: Vec::new(),
            },
        );
        persistence
            .commit(&initial.snapshot, &with_grant)
            .await
            .unwrap_or_else(|error| panic!("expected first state commit: {error}"));
        let mut current = with_grant.clone();
        current.revision = json_u64(2);
        persistence
            .commit(&with_grant, &current)
            .await
            .unwrap_or_else(|error| panic!("expected second state commit: {error}"));
        std::fs::write(persistence.primary_path(), b"corrupt")
            .unwrap_or_else(|error| panic!("expected corrupt primary: {error}"));

        let recovered = persistence
            .load_or_recover()
            .await
            .unwrap_or_else(|error| panic!("expected sanitized recovery: {error}"));
        let mut expected = with_grant;
        expected
            .plugins
            .get_mut(&plugin_id)
            .unwrap_or_else(|| panic!("expected plugin record"))
            .user_enablement = UserEnablement::Disabled;
        expected.launch_grants.clear();
        assert_eq!(
            recovered,
            super::StateRecovery {
                snapshot: expected,
                source: StateRecoverySource::SanitizedBackup,
            }
        );
    }

    /// Refuses readiness when neither durable state copy can establish user intent.
    #[tokio::test]
    async fn double_corrupt_state_fails_closed() {
        let root =
            TempDir::new().unwrap_or_else(|error| panic!("expected state directory: {error}"));
        let persistence = FileStatePersistence::new(root.path());
        std::fs::create_dir_all(root.path())
            .unwrap_or_else(|error| panic!("expected state root: {error}"));
        std::fs::write(persistence.primary_path(), b"corrupt")
            .unwrap_or_else(|error| panic!("expected corrupt primary: {error}"));
        std::fs::write(persistence.backup_path(), b"also corrupt")
            .unwrap_or_else(|error| panic!("expected corrupt backup: {error}"));
        assert_eq!(
            persistence.load_or_recover().await,
            Err(PluginError::StateCorrupt)
        );
    }

    /// Rejects a future primary schema instead of silently rolling user state backward.
    #[tokio::test]
    async fn future_primary_schema_does_not_fall_back() {
        let root =
            TempDir::new().unwrap_or_else(|error| panic!("expected state directory: {error}"));
        let persistence = FileStatePersistence::new(root.path());
        std::fs::create_dir_all(root.path())
            .unwrap_or_else(|error| panic!("expected state root: {error}"));
        std::fs::write(persistence.primary_path(), br#"{"schemaVersion":2}"#)
            .unwrap_or_else(|error| panic!("expected future primary: {error}"));
        std::fs::write(
            persistence.backup_path(),
            serde_json::to_vec(&PluginStateSnapshot::empty())
                .unwrap_or_else(|error| panic!("expected backup JSON: {error}")),
        )
        .unwrap_or_else(|error| panic!("expected valid backup: {error}"));
        assert_eq!(
            persistence.load_or_recover().await,
            Err(PluginError::StateVersionUnsupported { schema_version: 2 })
        );
    }

    /// Constructs a JavaScript-safe state counter for recovery fixtures.
    fn json_u64(value: u64) -> JsonSafeU64 {
        JsonSafeU64::new(value).unwrap_or_else(|error| panic!("expected JSON integer: {error}"))
    }

    /// Parses one canonical operation UUID for recovery fixtures.
    fn operation_id(value: &str) -> OperationId {
        OperationId::parse(value).unwrap_or_else(|error| panic!("expected operation id: {error}"))
    }
}
