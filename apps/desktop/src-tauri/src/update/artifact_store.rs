//! Recoverable storage for signed Desktop update artifacts.
//!
//! An entry directory is the commit marker. Downloads are prepared under `staging` and moved into
//! the identity-addressed `entries` directory only after the package and record are complete.

use super::UpdateError;
use ora_logging::ora_warn;
use ora_utils::hash::sha256_reader;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;
use url::Url;

const STORE_DIRECTORY: &str = "desktop-updates";
const STORE_VERSION: &str = "v2";
const ENTRIES_DIRECTORY: &str = "entries";
const STAGING_DIRECTORY: &str = "staging";
const RECORD_FILE: &str = "record.json";
const RECORD_SCHEMA_VERSION: u32 = 2;

/// Verifies artifact bytes and identifies the trust root used for that decision.
///
/// Production implementations must perform cryptographic signature verification. Test
/// implementations may use deterministic local evidence to exercise recovery behavior.
pub(super) trait ArtifactVerifier {
    /// Returns a stable fingerprint that prevents reuse after a trust-root rotation.
    fn trust_root_fingerprint(&self) -> &str;

    /// Returns whether `bytes` carry the supplied release signature under the trust root.
    fn verify(&self, bytes: &[u8], encoded_signature: &str) -> bool;
}

/// Names the installer format represented by an update artifact.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum UpdateBundleKind {
    Nsis,
    AppImage,
}

impl UpdateBundleKind {
    /// Returns the only self-update artifact format valid for the current target.
    pub(super) fn current() -> Self {
        #[cfg(target_os = "windows")]
        {
            Self::Nsis
        }
        #[cfg(target_os = "linux")]
        {
            Self::AppImage
        }
    }

    /// Returns the trusted local payload name for this installer format.
    fn payload_file_name(self) -> &'static str {
        match self {
            Self::Nsis => "payload.exe",
            Self::AppImage => "payload.AppImage",
        }
    }
}

/// Identifies one artifact independently of any mutable local path or source URL.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct ArtifactIdentity {
    pub(super) release_version: String,
    pub(super) target: String,
    pub(super) bundle_kind: UpdateBundleKind,
    pub(super) signature_fingerprint: String,
    pub(super) trust_root_fingerprint: String,
}

impl ArtifactIdentity {
    /// Derives a cache identity from the fresh manifest and the active updater trust root.
    pub(super) fn new(
        release_version: String,
        target: String,
        bundle_kind: UpdateBundleKind,
        encoded_signature: &str,
        verifier: &impl ArtifactVerifier,
    ) -> Result<Self, UpdateError> {
        Ok(Self {
            release_version,
            target,
            bundle_kind,
            signature_fingerprint: sha256(encoded_signature.as_bytes())?,
            trust_root_fingerprint: verifier.trust_root_fingerprint().to_owned(),
        })
    }

    /// Returns the path-safe content address used for the committed entry directory.
    fn artifact_id(&self) -> Result<String, UpdateError> {
        let encoded = serde_json::to_vec(self).map_err(UpdateError::EncodeMetadata)?;
        sha256(&encoded)
    }
}

/// Carries fresh manifest evidence needed to validate or commit one artifact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ArtifactDescriptor {
    pub(super) identity: ArtifactIdentity,
    pub(super) source_url: Url,
    pub(super) original_file_name: String,
    pub(super) encoded_signature: String,
}

impl ArtifactDescriptor {
    /// Builds a descriptor from fields announced by the current updater manifest.
    pub(super) fn new(
        release_version: String,
        target: String,
        source_url: Url,
        encoded_signature: String,
        verifier: &impl ArtifactVerifier,
    ) -> Result<Self, UpdateError> {
        let original_file_name = source_url
            .path_segments()
            .and_then(Iterator::last)
            .filter(|name| !name.is_empty())
            .unwrap_or("update-package")
            .to_owned();
        Ok(Self {
            identity: ArtifactIdentity::new(
                release_version,
                target,
                UpdateBundleKind::current(),
                &encoded_signature,
                verifier,
            )?,
            source_url,
            original_file_name,
            encoded_signature,
        })
    }
}

/// Describes the package bytes committed beside an artifact identity record.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ArtifactPayloadRecord {
    file_name: String,
    byte_length: u64,
    sha256: String,
}

/// Persists release identity, provenance, and package integrity for diagnostics and recovery.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ArtifactRecord {
    schema_version: u32,
    identity: ArtifactIdentity,
    source_url: Url,
    original_file_name: String,
    payload: ArtifactPayloadRecord,
}

/// References one committed artifact whose identity and signature have been validated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StoredArtifact {
    pub(super) entry_path: PathBuf,
    pub(super) payload_path: PathBuf,
    pub(super) identity: ArtifactIdentity,
}

/// Owns the versioned entry layout, recovery validation, commits, and garbage collection.
pub(super) struct UpdateArtifactStore {
    entries: PathBuf,
    staging: PathBuf,
}

impl UpdateArtifactStore {
    /// Opens the versioned store, removes interrupted writes, and drops superseded entries.
    pub(super) fn open(home_directory: &Path, current: &Version) -> Result<Self, UpdateError> {
        let cache_directory = home_directory.join("cache");
        let root = cache_directory.join(STORE_DIRECTORY).join(STORE_VERSION);
        let entries = root.join(ENTRIES_DIRECTORY);
        let staging = root.join(STAGING_DIRECTORY);
        std::fs::create_dir_all(&entries).map_err(UpdateError::CacheDirectory)?;
        std::fs::create_dir_all(&staging).map_err(UpdateError::CacheDirectory)?;
        let store = Self { entries, staging };
        store.clear_directory(&store.staging);
        store.discard_superseded(current)?;
        Ok(store)
    }

    /// Returns a matching committed artifact only after checking its record, digest, and signature.
    pub(super) async fn find_verified(
        &self,
        descriptor: &ArtifactDescriptor,
        verifier: &impl ArtifactVerifier,
    ) -> Result<Option<StoredArtifact>, UpdateError> {
        let validated = self.validate_entry(descriptor, verifier).await?;
        if validated.is_some() {
            self.prune_except(&descriptor.identity.artifact_id()?);
        }
        Ok(validated.map(|(artifact, _)| artifact))
    }

    /// Atomically publishes verified bytes and removes entries for older release identities.
    pub(super) async fn commit(
        &self,
        descriptor: &ArtifactDescriptor,
        bytes: &[u8],
        verifier: &impl ArtifactVerifier,
    ) -> Result<StoredArtifact, UpdateError> {
        if !verifier.verify(bytes, &descriptor.encoded_signature) {
            return Err(UpdateError::CachedArtifactUntrusted);
        }
        let artifact_id = descriptor.identity.artifact_id()?;
        let final_path = self.entries.join(&artifact_id);
        let temporary = tempfile::Builder::new()
            .prefix("download-")
            .tempdir_in(&self.staging)
            .map_err(UpdateError::CacheWrite)?;
        let payload_file_name = descriptor.identity.bundle_kind.payload_file_name();
        let payload_path = temporary.path().join(payload_file_name);
        write_synced(&payload_path, bytes).await?;
        let record = ArtifactRecord {
            schema_version: RECORD_SCHEMA_VERSION,
            identity: descriptor.identity.clone(),
            source_url: descriptor.source_url.clone(),
            original_file_name: descriptor.original_file_name.clone(),
            payload: ArtifactPayloadRecord {
                file_name: payload_file_name.to_owned(),
                byte_length: bytes.len() as u64,
                sha256: sha256(bytes)?,
            },
        };
        let record_bytes =
            serde_json::to_vec_pretty(&record).map_err(UpdateError::EncodeMetadata)?;
        write_synced(&temporary.path().join(RECORD_FILE), &record_bytes).await?;

        if final_path.exists() {
            remove_path(&final_path).map_err(UpdateError::CacheCommit)?;
        }
        let temporary_path = temporary.keep();
        tokio::fs::rename(&temporary_path, &final_path)
            .await
            .map_err(UpdateError::CacheCommit)?;
        self.prune_except(&artifact_id);
        Ok(StoredArtifact {
            payload_path: final_path.join(payload_file_name),
            entry_path: final_path,
            identity: descriptor.identity.clone(),
        })
    }

    /// Re-reads and verifies a ready artifact immediately before installation.
    pub(super) async fn read_verified(
        &self,
        stored: &StoredArtifact,
        descriptor: &ArtifactDescriptor,
        verifier: &impl ArtifactVerifier,
    ) -> Result<Vec<u8>, UpdateError> {
        if stored.identity != descriptor.identity
            || stored.entry_path != self.entries.join(descriptor.identity.artifact_id()?)
            || stored.payload_path
                != stored
                    .entry_path
                    .join(descriptor.identity.bundle_kind.payload_file_name())
        {
            return Err(UpdateError::CachedArtifactChanged);
        }
        self.validate_entry(descriptor, verifier)
            .await?
            .map(|(_, bytes)| bytes)
            .ok_or(UpdateError::CachedArtifactChanged)
    }

    /// Removes every committed and interrupted artifact after no update remains installable.
    pub(super) fn clear(&self) {
        self.clear_directory(&self.entries);
        self.clear_directory(&self.staging);
    }

    /// Validates the exact identity-addressed entry and removes it when local evidence is invalid.
    async fn validate_entry(
        &self,
        descriptor: &ArtifactDescriptor,
        verifier: &impl ArtifactVerifier,
    ) -> Result<Option<(StoredArtifact, Vec<u8>)>, UpdateError> {
        let entry_path = self.entries.join(descriptor.identity.artifact_id()?);
        let record_bytes = match tokio::fs::read(entry_path.join(RECORD_FILE)).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(UpdateError::ReadMetadata(error)),
        };
        let record = match serde_json::from_slice::<ArtifactRecord>(&record_bytes) {
            Ok(record) => record,
            Err(_) => {
                self.remove_invalid_entry(&entry_path);
                return Ok(None);
            }
        };
        let payload_file_name = descriptor.identity.bundle_kind.payload_file_name();
        let payload_path = entry_path.join(payload_file_name);
        let bytes = match tokio::fs::read(&payload_path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.remove_invalid_entry(&entry_path);
                return Ok(None);
            }
            Err(error) => return Err(UpdateError::CacheRead(error)),
        };
        let valid = record.schema_version == RECORD_SCHEMA_VERSION
            && record.identity == descriptor.identity
            && record.payload.file_name == payload_file_name
            && record.payload.byte_length == bytes.len() as u64
            && record.payload.sha256 == sha256(&bytes)?
            && verifier.verify(&bytes, &descriptor.encoded_signature);
        if !valid {
            self.remove_invalid_entry(&entry_path);
            return Ok(None);
        }
        Ok(Some((
            StoredArtifact {
                entry_path,
                payload_path,
                identity: descriptor.identity.clone(),
            },
            bytes,
        )))
    }

    /// Drops entries whose identity is malformed or no newer than the running build.
    fn discard_superseded(&self, current: &Version) -> Result<(), UpdateError> {
        for entry in std::fs::read_dir(&self.entries).map_err(UpdateError::CacheInspect)? {
            let entry = entry.map_err(UpdateError::CacheInspect)?;
            let path = entry.path();
            let entry_name = entry.file_name();
            let record = std::fs::read(path.join(RECORD_FILE))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<ArtifactRecord>(&bytes).ok());
            let keep = record.as_ref().is_some_and(|record| {
                record.schema_version == RECORD_SCHEMA_VERSION
                    && record.identity.artifact_id().ok().as_deref() == entry_name.to_str()
                    && Version::parse(&record.identity.release_version)
                        .is_ok_and(|release| release > *current)
            });
            if !keep {
                self.remove_invalid_entry(&path);
            }
        }
        Ok(())
    }

    /// Keeps the newly committed identity and removes every older candidate only after commit.
    fn prune_except(&self, retained_id: &str) {
        let Ok(entries) = std::fs::read_dir(&self.entries) else {
            return;
        };
        for entry in entries.flatten() {
            if entry.file_name() != retained_id {
                self.remove_invalid_entry(&entry.path());
            }
        }
    }

    /// Removes all children while leaving the versioned store directories ready for later use.
    fn clear_directory(&self, directory: &Path) {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            self.remove_invalid_entry(&entry.path());
        }
    }

    /// Treats cleanup as recoverable maintenance so a valid release check is not hidden by it.
    fn remove_invalid_entry(&self, path: &Path) {
        if let Err(error) = remove_path(path) {
            ora_warn!(message = "failed to remove Desktop update cache entry", path = ?path, error = %error);
        }
    }
}

/// Writes and syncs one staging file before its containing directory can be committed.
async fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), UpdateError> {
    let mut file = tokio::fs::File::create(path)
        .await
        .map_err(UpdateError::CacheWrite)?;
    file.write_all(bytes)
        .await
        .map_err(UpdateError::CacheWrite)?;
    file.flush().await.map_err(UpdateError::CacheWrite)?;
    file.sync_all().await.map_err(UpdateError::CacheWrite)
}

/// Computes the lowercase SHA-256 identity used for records and path-safe entry names.
fn sha256(bytes: &[u8]) -> Result<String, UpdateError> {
    sha256_reader(Cursor::new(bytes)).map_err(UpdateError::CacheRead)
}

/// Removes one known store child without following a symlink into another directory tree.
fn remove_path(path: &Path) -> Result<(), std::io::Error> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() || metadata.is_file() {
        std::fs::remove_file(path)
    } else {
        std::fs::remove_dir_all(path)
    }
}
