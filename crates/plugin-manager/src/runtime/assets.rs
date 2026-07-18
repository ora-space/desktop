use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ora_plugin_protocol::{
    ContentDigest, PLUGIN_API_VERSION_V1, PluginRelativePath, PluginVersion, WIRE_VERSION_V1,
    parse_strict_json,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use crate::{PluginError, PluginRuntimeAssets, audit_regular_file};

const RUNTIME_ASSET_SCHEMA_V1: u32 = 1;
const RUNTIME_RECEIPT_SCHEMA_V1: u32 = 1;
const MAX_RUNTIME_MANIFEST_BYTES: usize = 256 * 1024;
const MAX_RUNTIME_FILE_BYTES: usize = 256 * 1024 * 1024;
const RECEIPT_FILE: &str = "runtime-receipt.json";
const ACTIVE_FILE: &str = "active.json";
pub const RUNTIME_ASSET_MANIFEST_FILE: &str = "runtime-manifest.json";

/// The only production platform frozen by the v1 runtime asset contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeTarget {
    #[serde(rename = "x86_64-pc-windows-msvc")]
    X86_64PcWindowsMsvc,
}

/// Immutable upstream Bun archive identity retained for supply-chain evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BunSourceIdentity {
    pub version: PluginVersion,
    pub official_url: String,
    pub archive_digest: ContentDigest,
    pub archive_bytes: u64,
}

/// Closed roles prevent a manifest from omitting or substituting a startup-critical file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeAssetRole {
    BunExecutable,
    BootstrapBundle,
    EmptyBunfig,
}

/// One exact extracted runtime file and its post-deployment digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssetFile {
    pub role: RuntimeAssetRole,
    pub path: PluginRelativePath,
    pub digest: ContentDigest,
    pub bytes: u64,
}

/// Versioned source manifest shared by test resources and future Tauri resources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssetManifest {
    pub schema_version: u32,
    pub target: RuntimeTarget,
    pub runtime_version: PluginVersion,
    pub wire_version: u32,
    pub plugin_api: u32,
    pub bootstrap_schema_version: u32,
    pub config_schema_version: u32,
    pub bun: BunSourceIdentity,
    pub files: Vec<RuntimeAssetFile>,
}

impl RuntimeAssetManifest {
    /// Validates cross-field identities and the exact three startup-critical file roles.
    pub fn validate(&self) -> Result<(), RuntimeAssetValidationError> {
        if self.schema_version != RUNTIME_ASSET_SCHEMA_V1 {
            return Err(RuntimeAssetValidationError::UnsupportedSchema);
        }
        if self.target != RuntimeTarget::X86_64PcWindowsMsvc
            || self.wire_version != WIRE_VERSION_V1
            || self.plugin_api != PLUGIN_API_VERSION_V1
            || self.bootstrap_schema_version != 1
            || self.config_schema_version != 1
            || self.bun.archive_bytes == 0
        {
            return Err(RuntimeAssetValidationError::IdentityMismatch);
        }
        let expected_url = format!(
            "https://github.com/oven-sh/bun/releases/download/bun-v{}/bun-windows-x64-baseline.zip",
            self.bun.version
        );
        if self.bun.official_url != expected_url {
            return Err(RuntimeAssetValidationError::IdentityMismatch);
        }
        let expected_paths = BTreeMap::from([
            (RuntimeAssetRole::BunExecutable, "bun.exe"),
            (
                RuntimeAssetRole::BootstrapBundle,
                "plugin-host-bootstrap.js",
            ),
            (RuntimeAssetRole::EmptyBunfig, "empty-bunfig.toml"),
        ]);
        if self.files.len() != expected_paths.len() {
            return Err(RuntimeAssetValidationError::InvalidFileSet);
        }
        let mut roles = BTreeSet::new();
        for file in &self.files {
            if !roles.insert(file.role)
                || expected_paths.get(&file.role).copied() != Some(file.path.as_str())
                || file.bytes == 0
                || file.bytes as usize > MAX_RUNTIME_FILE_BYTES
            {
                return Err(RuntimeAssetValidationError::InvalidFileSet);
            }
        }
        Ok(())
    }

    fn file(&self, role: RuntimeAssetRole) -> Option<&RuntimeAssetFile> {
        self.files.iter().find(|file| file.role == role)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RuntimeAssetValidationError {
    #[error("runtime asset schema version is unsupported")]
    UnsupportedSchema,
    #[error("runtime asset identity axes do not match v1")]
    IdentityMismatch,
    #[error("runtime asset manifest does not contain the exact required files")]
    InvalidFileSet,
    #[error("runtime asset manifest is not strict JSON")]
    InvalidManifest,
}

/// Read-only trusted resource boundary; implementations never download or inspect PATH.
pub trait RuntimeAssetSource: Send + Sync + 'static {
    /// Reads the bounded source manifest bytes.
    fn read_manifest(
        &self,
    ) -> impl Future<Output = Result<Vec<u8>, RuntimeAssetSourceError>> + Send;

    /// Reads one manifest-authorized extracted file by its validated relative path.
    fn read_file(
        &self,
        path: &PluginRelativePath,
    ) -> impl Future<Output = Result<Vec<u8>, RuntimeAssetSourceError>> + Send;
}

/// Reads application-packaged runtime resources from one immutable resource directory.
#[derive(Debug, Clone)]
pub struct DirectoryRuntimeAssetSource {
    root: PathBuf,
}

impl DirectoryRuntimeAssetSource {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl RuntimeAssetSource for DirectoryRuntimeAssetSource {
    async fn read_manifest(&self) -> Result<Vec<u8>, RuntimeAssetSourceError> {
        read_packaged_file(&self.root.join(RUNTIME_ASSET_MANIFEST_FILE)).await
    }

    async fn read_file(
        &self,
        path: &PluginRelativePath,
    ) -> Result<Vec<u8>, RuntimeAssetSourceError> {
        read_packaged_file(&self.root.join(path.as_str())).await
    }
}

/// Rejects links, hardlinks, ADS, and oversized packaged resources before reading bytes.
async fn read_packaged_file(path: &Path) -> Result<Vec<u8>, RuntimeAssetSourceError> {
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || {
        audit_regular_file(&path).map_err(|_| RuntimeAssetSourceError::Unreadable)?;
        let metadata = std::fs::metadata(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                RuntimeAssetSourceError::Missing
            } else {
                RuntimeAssetSourceError::Unreadable
            }
        })?;
        if metadata.len() as usize > MAX_RUNTIME_FILE_BYTES {
            return Err(RuntimeAssetSourceError::Unreadable);
        }
        std::fs::read(path).map_err(|_| RuntimeAssetSourceError::Unreadable)
    })
    .await
    .map_err(|_| RuntimeAssetSourceError::Unreadable)?
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RuntimeAssetSourceError {
    #[error("trusted runtime asset is missing")]
    Missing,
    #[error("trusted runtime asset cannot be read")]
    Unreadable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuntimeAssetReceipt {
    schema_version: u32,
    manifest_digest: ContentDigest,
    manifest: RuntimeAssetManifest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ActiveRuntimeReference {
    schema_version: u32,
    runtime_version: PluginVersion,
    manifest_digest: ContentDigest,
}

/// Deploys and revalidates one versioned runtime under the ManagerLease-owned data root.
pub struct RuntimeAssetStore<Source> {
    root: PathBuf,
    source: Arc<Source>,
    mutation: Mutex<()>,
    leases: std::sync::Mutex<BTreeMap<PluginVersion, std::sync::Weak<RuntimeAssetLeaseInner>>>,
}

impl<Source> RuntimeAssetStore<Source>
where
    Source: RuntimeAssetSource,
{
    pub fn new(root: impl Into<PathBuf>, source: Arc<Source>) -> Self {
        Self {
            root: root.into(),
            source,
            mutation: Mutex::new(()),
            leases: std::sync::Mutex::new(BTreeMap::new()),
        }
    }

    /// Returns a verified runtime lease, deploying or repairing from the same trusted source.
    pub async fn prepare(&self) -> Result<RuntimeAssetLease, PluginError> {
        let _guard = self.mutation.lock().await;
        let manifest_bytes = self
            .source
            .read_manifest()
            .await
            .map_err(|_| PluginError::PluginRuntimeUnavailable)?;
        let manifest =
            parse_manifest(&manifest_bytes).map_err(|_| PluginError::PluginRuntimeUnavailable)?;
        let manifest_digest =
            digest_bytes(&manifest_bytes).map_err(|_| PluginError::PluginRuntimeUnavailable)?;

        tokio::fs::create_dir_all(self.root.join(".staging"))
            .await
            .map_err(|_| PluginError::PluginRuntimeUnavailable)?;
        tokio::fs::create_dir_all(self.root.join(".trash"))
            .await
            .map_err(|_| PluginError::PluginRuntimeUnavailable)?;
        let final_path = self
            .root
            .join(format!("runtime-{}", manifest.runtime_version));
        let active_inner = {
            let leases = self
                .leases
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            leases
                .get(&manifest.runtime_version)
                .and_then(std::sync::Weak::upgrade)
        };
        if let Some(inner) = active_inner {
            validate_installed_runtime(&inner.root, &manifest, &manifest_digest)
                .await
                .map_err(|_| PluginError::PluginRuntimeUnavailable)?;
            return Ok(RuntimeAssetLease { inner });
        }
        if final_path.exists()
            && validate_installed_runtime(&final_path, &manifest, &manifest_digest)
                .await
                .is_err()
        {
            let quarantine = self
                .root
                .join(".trash")
                .join(format!("corrupt-{}", uuid::Uuid::new_v4()));
            tokio::fs::rename(&final_path, quarantine)
                .await
                .map_err(|_| PluginError::PluginRuntimeUnavailable)?;
        }
        if !final_path.exists() {
            self.deploy(&manifest, &manifest_digest, &final_path)
                .await?;
        }
        validate_installed_runtime(&final_path, &manifest, &manifest_digest)
            .await
            .map_err(|_| PluginError::PluginRuntimeUnavailable)?;
        self.commit_active_reference(&manifest, &manifest_digest)
            .await?;

        let inner = Arc::new(RuntimeAssetLeaseInner {
            root: final_path,
            manifest: manifest.clone(),
            manifest_digest,
        });
        self.leases
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(manifest.runtime_version, Arc::downgrade(&inner));
        Ok(RuntimeAssetLease { inner })
    }

    async fn deploy(
        &self,
        manifest: &RuntimeAssetManifest,
        manifest_digest: &ContentDigest,
        final_path: &Path,
    ) -> Result<(), PluginError> {
        let staging = self
            .root
            .join(".staging")
            .join(uuid::Uuid::new_v4().to_string());
        tokio::fs::create_dir(&staging)
            .await
            .map_err(|_| PluginError::PluginRuntimeUnavailable)?;
        for file in &manifest.files {
            let bytes = self
                .source
                .read_file(&file.path)
                .await
                .map_err(|_| PluginError::PluginRuntimeUnavailable)?;
            if bytes.len() as u64 != file.bytes || digest_bytes(&bytes).as_ref() != Ok(&file.digest)
            {
                return Err(PluginError::PluginRuntimeUnavailable);
            }
            write_create_new(&staging.join(file.path.as_str()), &bytes).await?;
        }
        let receipt = RuntimeAssetReceipt {
            schema_version: RUNTIME_RECEIPT_SCHEMA_V1,
            manifest_digest: manifest_digest.clone(),
            manifest: manifest.clone(),
        };
        let receipt = serde_json::to_vec_pretty(&receipt)
            .map_err(|_| PluginError::PluginRuntimeUnavailable)?;
        write_create_new(&staging.join(RECEIPT_FILE), &receipt).await?;
        tokio::fs::rename(&staging, final_path)
            .await
            .map_err(|_| PluginError::PluginRuntimeUnavailable)
    }

    async fn commit_active_reference(
        &self,
        manifest: &RuntimeAssetManifest,
        manifest_digest: &ContentDigest,
    ) -> Result<(), PluginError> {
        let active = ActiveRuntimeReference {
            schema_version: 1,
            runtime_version: manifest.runtime_version.clone(),
            manifest_digest: manifest_digest.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&active)
            .map_err(|_| PluginError::PluginRuntimeUnavailable)?;
        let temporary = self
            .root
            .join(format!("active-{}.tmp", uuid::Uuid::new_v4()));
        write_create_new(&temporary, &bytes).await?;
        replace_active_file(&temporary, &self.root.join(ACTIVE_FILE))
            .await
            .map_err(|_| PluginError::PluginRuntimeUnavailable)
    }
}

/// Keeps the version directory live and revalidates all critical files before every spawn.
#[derive(Clone)]
pub struct RuntimeAssetLease {
    inner: Arc<RuntimeAssetLeaseInner>,
}

struct RuntimeAssetLeaseInner {
    root: PathBuf,
    manifest: RuntimeAssetManifest,
    manifest_digest: ContentDigest,
}

impl RuntimeAssetLease {
    /// Rechecks receipt, type, size, and digest, then returns launch-only absolute paths.
    pub async fn launch_assets(&self) -> Result<PluginRuntimeAssets, PluginError> {
        validate_installed_runtime(
            &self.inner.root,
            &self.inner.manifest,
            &self.inner.manifest_digest,
        )
        .await
        .map_err(|_| PluginError::PluginRuntimeUnavailable)?;
        Ok(PluginRuntimeAssets::new(
            self.path(RuntimeAssetRole::BunExecutable)?,
            self.path(RuntimeAssetRole::BootstrapBundle)?,
            self.path(RuntimeAssetRole::EmptyBunfig)?,
            self.inner.manifest.runtime_version.clone(),
        ))
    }

    fn path(&self, role: RuntimeAssetRole) -> Result<PathBuf, PluginError> {
        let file = self
            .inner
            .manifest
            .file(role)
            .ok_or(PluginError::PluginRuntimeUnavailable)?;
        Ok(self.inner.root.join(file.path.as_str()))
    }
}

fn parse_manifest(bytes: &[u8]) -> Result<RuntimeAssetManifest, RuntimeAssetValidationError> {
    if bytes.is_empty() || bytes.len() > MAX_RUNTIME_MANIFEST_BYTES {
        return Err(RuntimeAssetValidationError::InvalidManifest);
    }
    let value =
        parse_strict_json(bytes, 64).map_err(|_| RuntimeAssetValidationError::InvalidManifest)?;
    let manifest = serde_json::from_value::<RuntimeAssetManifest>(value)
        .map_err(|_| RuntimeAssetValidationError::InvalidManifest)?;
    manifest.validate()?;
    Ok(manifest)
}

async fn validate_installed_runtime(
    root: &Path,
    manifest: &RuntimeAssetManifest,
    manifest_digest: &ContentDigest,
) -> Result<(), RuntimeAssetValidationError> {
    let root_metadata =
        std::fs::symlink_metadata(root).map_err(|_| RuntimeAssetValidationError::InvalidFileSet)?;
    if !root_metadata.is_dir()
        || root_metadata.file_type().is_symlink()
        || is_reparse(&root_metadata)
    {
        return Err(RuntimeAssetValidationError::InvalidFileSet);
    }
    validate_regular_file(&root.join(RECEIPT_FILE))?;
    let receipt_bytes = tokio::fs::read(root.join(RECEIPT_FILE))
        .await
        .map_err(|_| RuntimeAssetValidationError::InvalidManifest)?;
    let receipt_value = parse_strict_json(&receipt_bytes, 64)
        .map_err(|_| RuntimeAssetValidationError::InvalidManifest)?;
    let receipt: RuntimeAssetReceipt = serde_json::from_value(receipt_value)
        .map_err(|_| RuntimeAssetValidationError::InvalidManifest)?;
    if receipt.schema_version != RUNTIME_RECEIPT_SCHEMA_V1
        || &receipt.manifest_digest != manifest_digest
        || &receipt.manifest != manifest
    {
        return Err(RuntimeAssetValidationError::IdentityMismatch);
    }
    for file in &manifest.files {
        let path = root.join(file.path.as_str());
        validate_regular_file(&path)?;
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|_| RuntimeAssetValidationError::InvalidFileSet)?;
        if bytes.len() as u64 != file.bytes || digest_bytes(&bytes).as_ref() != Ok(&file.digest) {
            return Err(RuntimeAssetValidationError::InvalidFileSet);
        }
    }
    Ok(())
}

async fn write_create_new(path: &Path, bytes: &[u8]) -> Result<(), PluginError> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|_| PluginError::PluginRuntimeUnavailable)?;
    }
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await
        .map_err(|_| PluginError::PluginRuntimeUnavailable)?;
    file.write_all(bytes)
        .await
        .map_err(|_| PluginError::PluginRuntimeUnavailable)?;
    file.sync_all()
        .await
        .map_err(|_| PluginError::PluginRuntimeUnavailable)?;
    drop(file);
    audit_regular_file(path).map_err(|_| PluginError::PluginRuntimeUnavailable)
}

fn digest_bytes(bytes: &[u8]) -> Result<ContentDigest, ora_plugin_protocol::IdentityError> {
    ContentDigest::parse(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn validate_regular_file(path: &Path) -> Result<(), RuntimeAssetValidationError> {
    audit_regular_file(path).map_err(|_| RuntimeAssetValidationError::InvalidFileSet)?;
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| RuntimeAssetValidationError::InvalidFileSet)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || is_reparse(&metadata) {
        return Err(RuntimeAssetValidationError::InvalidFileSet);
    }
    if hard_link_count(path).map_err(|_| RuntimeAssetValidationError::InvalidFileSet)? != 1 {
        return Err(RuntimeAssetValidationError::InvalidFileSet);
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse(_metadata: &std::fs::Metadata) -> bool {
    false
}

#[cfg(windows)]
fn hard_link_count(path: &Path) -> std::io::Result<u32> {
    use std::mem::zeroed;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };
    let file = std::fs::File::open(path)?;
    let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut information) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(information.nNumberOfLinks)
}

#[cfg(not(windows))]
fn hard_link_count(path: &Path) -> std::io::Result<u32> {
    use std::os::unix::fs::MetadataExt;
    u32::try_from(std::fs::metadata(path)?.nlink()).map_err(std::io::Error::other)
}

async fn replace_active_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    let source = source.to_owned();
    let destination = destination.to_owned();
    tokio::task::spawn_blocking(move || {
        audit_regular_file(&source).map_err(|_| std::io::Error::other("invalid active temp"))?;
        if destination.exists() {
            audit_regular_file(&destination)
                .map_err(|_| std::io::Error::other("invalid active reference"))?;
        }
        replace_active_file_sync(&source, &destination)?;
        audit_regular_file(&destination)
            .map_err(|_| std::io::Error::other("invalid active replacement"))
    })
    .await
    .map_err(std::io::Error::other)?
}

#[cfg(windows)]
fn replace_active_file_sync(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_active_file_sync(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(source, destination)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex, PoisonError};

    use ora_plugin_protocol::{ContentDigest, PluginRelativePath, PluginVersion};
    use pretty_assertions::assert_eq;
    use sha2::{Digest, Sha256};
    use tempfile::TempDir;

    use super::{
        BunSourceIdentity, RuntimeAssetFile, RuntimeAssetManifest, RuntimeAssetRole,
        RuntimeAssetSource, RuntimeAssetSourceError, RuntimeAssetStore, RuntimeTarget,
    };

    struct FakeSource {
        manifest: Vec<u8>,
        files: Mutex<BTreeMap<String, Vec<u8>>>,
    }

    impl RuntimeAssetSource for FakeSource {
        async fn read_manifest(&self) -> Result<Vec<u8>, RuntimeAssetSourceError> {
            Ok(self.manifest.clone())
        }

        async fn read_file(
            &self,
            path: &PluginRelativePath,
        ) -> Result<Vec<u8>, RuntimeAssetSourceError> {
            self.files
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(path.as_str())
                .cloned()
                .ok_or(RuntimeAssetSourceError::Missing)
        }
    }

    /// Deploys from an injected source and repairs a corrupted active bootstrap before launch.
    #[tokio::test]
    async fn deploys_validates_and_repairs_runtime_assets() {
        let temp = TempDir::new().unwrap_or_else(|error| panic!("temp runtime dir: {error}"));
        let source = Arc::new(fake_source());
        let store = RuntimeAssetStore::new(temp.path().join("runtime"), source);
        let lease = store
            .prepare()
            .await
            .unwrap_or_else(|error| panic!("prepare runtime: {error}"));
        let assets = lease
            .launch_assets()
            .await
            .unwrap_or_else(|error| panic!("launch assets: {error}"));
        assert_eq!(
            std::fs::read(assets.bootstrap_entry())
                .unwrap_or_else(|error| panic!("read bootstrap: {error}")),
            b"bootstrap".to_vec()
        );

        std::fs::write(assets.bootstrap_entry(), b"tampered")
            .unwrap_or_else(|error| panic!("tamper bootstrap: {error}"));
        assert!(lease.launch_assets().await.is_err());
        assert!(store.prepare().await.is_err());
        drop(lease);
        let repaired = store
            .prepare()
            .await
            .unwrap_or_else(|error| panic!("repair runtime: {error}"));
        assert!(repaired.launch_assets().await.is_ok());
    }

    fn fake_source() -> FakeSource {
        let files = BTreeMap::from([
            ("bun.exe".to_owned(), b"fake-bun".to_vec()),
            ("plugin-host-bootstrap.js".to_owned(), b"bootstrap".to_vec()),
            ("empty-bunfig.toml".to_owned(), b"[install]\n".to_vec()),
        ]);
        let entries = [
            (RuntimeAssetRole::BunExecutable, "bun.exe"),
            (
                RuntimeAssetRole::BootstrapBundle,
                "plugin-host-bootstrap.js",
            ),
            (RuntimeAssetRole::EmptyBunfig, "empty-bunfig.toml"),
        ]
        .map(|(role, path)| {
            let bytes = &files[path];
            RuntimeAssetFile {
                role,
                path: PluginRelativePath::parse(path)
                    .unwrap_or_else(|error| panic!("asset path: {error}")),
                digest: ContentDigest::parse(format!("sha256:{:x}", Sha256::digest(bytes)))
                    .unwrap_or_else(|error| panic!("asset digest: {error}")),
                bytes: bytes.len() as u64,
            }
        });
        let manifest = RuntimeAssetManifest {
            schema_version: 1,
            target: RuntimeTarget::X86_64PcWindowsMsvc,
            runtime_version: PluginVersion::parse("1.0.0")
                .unwrap_or_else(|error| panic!("runtime version: {error}")),
            wire_version: 1,
            plugin_api: 1,
            bootstrap_schema_version: 1,
            config_schema_version: 1,
            bun: BunSourceIdentity {
                version: PluginVersion::parse("1.3.14")
                    .unwrap_or_else(|error| panic!("Bun version: {error}")),
                official_url: "https://github.com/oven-sh/bun/releases/download/bun-v1.3.14/bun-windows-x64-baseline.zip".to_owned(),
                archive_digest: ContentDigest::parse(format!("sha256:{}", "5".repeat(64)))
                    .unwrap_or_else(|error| panic!("archive digest: {error}")),
                archive_bytes: 38_023_440,
            },
            files: entries.to_vec(),
        };
        FakeSource {
            manifest: serde_json::to_vec_pretty(&manifest)
                .unwrap_or_else(|error| panic!("manifest JSON: {error}")),
            files: Mutex::new(files),
        }
    }
}
