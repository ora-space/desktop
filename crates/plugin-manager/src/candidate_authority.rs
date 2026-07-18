use crate::{
    AuthorizationHandleFailure, PackageValidator, PluginError, SourceChangeReason,
    ValidatedPackage, ValidationTarget,
};
use ora_plugin_protocol::{CandidateAuditId, ContentDigest, PluginId, PluginVersion};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// An opaque, session-bound, single-use authorization for identify.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SelectionHandle(String);

impl SelectionHandle {
    /// Reconstructs an opaque transport token without granting authority by itself.
    pub fn from_opaque(value: impl Into<String>) -> Result<Self, PluginError> {
        parse_opaque_handle(value.into()).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An opaque, digest-bound, session-bound, single-use authorization for install.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CandidateHandle(String);

impl CandidateHandle {
    /// Reconstructs an opaque transport token without granting authority by itself.
    pub fn from_opaque(value: impl Into<String>) -> Result<Self, PluginError> {
        parse_opaque_handle(value.into()).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A Host-issued management session identity associated with the in-memory bearer.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ManagementSessionId(String);

impl ManagementSessionId {
    pub fn new_random() -> Result<Self, PluginError> {
        Ok(Self(random_token()?))
    }
}

/// The root identity checked both before and after candidate review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRootIdentity {
    pub canonical_path: PathBuf,
    pub volume_identity: u64,
    pub file_identity: u128,
}

/// Safe display data returned by discovery without exposing an authoritative path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateSelection {
    pub selection_handle: SelectionHandle,
    pub display_name: String,
}

/// The reviewed identity and digest shown before minting install authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifiedPlugin {
    pub plugin_id: PluginId,
    pub plugin_version: PluginVersion,
    pub content_digest: ContentDigest,
    pub candidate_handle: CandidateHandle,
    pub package: ValidatedPackage,
}

/// A consumed candidate containing only server-held authority and reviewed facts.
#[derive(Debug, Clone)]
pub struct AuthorizedCandidate {
    pub source_root: PathBuf,
    pub source_identity: SourceRootIdentity,
    pub plugin_id: PluginId,
    pub plugin_version: PluginVersion,
    pub content_digest: ContentDigest,
    pub audit_id: CandidateAuditId,
}

/// Provides an injectable monotonic clock for TTL and single-use race tests.
pub trait AuthorityClock: Clone + Send + Sync + 'static {
    /// Returns a monotonic duration from this clock's private origin.
    fn now(&self) -> Duration;
}

/// Process-local monotonic clock used by production handle stores.
#[derive(Debug, Clone)]
pub struct SystemAuthorityClock {
    origin: Instant,
}

impl SystemAuthorityClock {
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Default for SystemAuthorityClock {
    fn default() -> Self {
        Self::new()
    }
}

impl AuthorityClock for SystemAuthorityClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}

/// Owns both authorization stages and consumes each token atomically under one lock.
pub struct CandidateAuthority<Clock> {
    clock: Clock,
    selection_ttl: Duration,
    candidate_ttl: Duration,
    entries: Mutex<AuthorityEntries>,
}

struct AuthorityEntries {
    selections: HashMap<String, SelectionRecord>,
    candidates: HashMap<String, CandidateRecord>,
}

struct SelectionRecord {
    session: ManagementSessionId,
    source_root: PathBuf,
    source_identity: SourceRootIdentity,
    audit_id: CandidateAuditId,
    expires_at: Duration,
}

struct CandidateRecord {
    session: ManagementSessionId,
    authorized: AuthorizedCandidate,
    expires_at: Duration,
}

impl<Clock> CandidateAuthority<Clock>
where
    Clock: AuthorityClock,
{
    pub fn new(clock: Clock, selection_ttl: Duration, candidate_ttl: Duration) -> Self {
        Self {
            clock,
            selection_ttl,
            candidate_ttl,
            entries: Mutex::new(AuthorityEntries {
                selections: HashMap::new(),
                candidates: HashMap::new(),
            }),
        }
    }

    /// Registers a trusted picker/discovery path and returns only display data plus an opaque token.
    pub fn register_selection(
        &self,
        session: ManagementSessionId,
        source_root: &Path,
        audit_id: CandidateAuditId,
    ) -> Result<CandidateSelection, PluginError> {
        let source_identity = identify_source_root(source_root)?;
        let token = random_token()?;
        let expires_at = self.clock.now().saturating_add(self.selection_ttl);
        let display_name = source_identity
            .canonical_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("local plugin")
            .to_string();
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .selections
            .insert(
                token.clone(),
                SelectionRecord {
                    session,
                    source_root: source_identity.canonical_path.clone(),
                    source_identity,
                    audit_id,
                    expires_at,
                },
            );
        Ok(CandidateSelection {
            selection_handle: SelectionHandle(token),
            display_name,
        })
    }

    /// Atomically consumes selection authority, validates source, and mints digest-bound authority.
    pub fn identify(
        &self,
        session: &ManagementSessionId,
        selection: SelectionHandle,
        validator: &PackageValidator,
    ) -> Result<IdentifiedPlugin, PluginError> {
        let record = self.consume_selection(session, selection)?;
        let current_identity = identify_source_root(&record.source_root)?;
        if current_identity != record.source_identity {
            return Err(PluginError::SourceChanged {
                reason: SourceChangeReason::RootIdentityMismatch,
            });
        }
        let package = validator
            .validate(&record.source_root, ValidationTarget::Candidate)
            .map_err(|error| PluginError::InvalidManifest {
                diagnostics: vec![crate::PluginDiagnostic::new(
                    crate::PluginDiagnosticCode::InvalidManifest,
                    error.to_string(),
                )],
            })?;
        let authorized = AuthorizedCandidate {
            source_root: record.source_root,
            source_identity: record.source_identity,
            plugin_id: package.manifest.ora.id().clone(),
            plugin_version: package.manifest.version.clone(),
            content_digest: package.digest.digest.clone(),
            audit_id: record.audit_id,
        };
        let token = random_token()?;
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .candidates
            .insert(
                token.clone(),
                CandidateRecord {
                    session: session.clone(),
                    authorized: authorized.clone(),
                    expires_at: self.clock.now().saturating_add(self.candidate_ttl),
                },
            );
        Ok(IdentifiedPlugin {
            plugin_id: authorized.plugin_id,
            plugin_version: authorized.plugin_version,
            content_digest: authorized.content_digest,
            candidate_handle: CandidateHandle(token),
            package,
        })
    }

    /// Consumes install authority before any copy attempt so failures cannot replay a token.
    pub fn consume_candidate(
        &self,
        session: &ManagementSessionId,
        handle: CandidateHandle,
    ) -> Result<AuthorizedCandidate, PluginError> {
        let record = self
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .candidates
            .remove(handle.as_str())
            .ok_or(PluginError::CandidateHandleInvalid {
                reason: AuthorizationHandleFailure::Unknown,
            })?;
        if &record.session != session {
            return Err(PluginError::CandidateHandleInvalid {
                reason: AuthorizationHandleFailure::WrongSession,
            });
        }
        if self.clock.now() > record.expires_at {
            return Err(PluginError::CandidateHandleInvalid {
                reason: AuthorizationHandleFailure::Expired,
            });
        }
        Ok(record.authorized)
    }

    /// Removes the selection under lock before validation to guarantee single use.
    fn consume_selection(
        &self,
        session: &ManagementSessionId,
        handle: SelectionHandle,
    ) -> Result<SelectionRecord, PluginError> {
        let record = self
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .selections
            .remove(handle.as_str())
            .ok_or(PluginError::SelectionHandleInvalid {
                reason: AuthorizationHandleFailure::Unknown,
            })?;
        if &record.session != session {
            return Err(PluginError::SelectionHandleInvalid {
                reason: AuthorizationHandleFailure::WrongSession,
            });
        }
        if self.clock.now() > record.expires_at {
            return Err(PluginError::SelectionHandleInvalid {
                reason: AuthorizationHandleFailure::Expired,
            });
        }
        Ok(record)
    }
}

/// Creates a 256-bit CSPRNG bearer encoded as canonical lowercase hex.
fn random_token() -> Result<String, PluginError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| PluginError::Internal {
        message: format!("failed to generate authorization token: {error}"),
    })?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Accepts only the exact lowercase 256-bit token encoding emitted by this authority.
fn parse_opaque_handle(value: String) -> Result<String, PluginError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Ok(value);
    }
    Err(PluginError::Internal {
        message: "opaque authorization handle is malformed".to_owned(),
    })
}

/// Captures canonical path and platform object identity for source-swap detection.
fn identify_source_root(path: &Path) -> Result<SourceRootIdentity, PluginError> {
    let canonical_path = std::fs::canonicalize(path).map_err(|_| PluginError::SourceChanged {
        reason: SourceChangeReason::RootMissing,
    })?;
    let metadata =
        std::fs::symlink_metadata(&canonical_path).map_err(|_| PluginError::SourceChanged {
            reason: SourceChangeReason::RootMissing,
        })?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(PluginError::SourceChanged {
            reason: SourceChangeReason::RootIdentityMismatch,
        });
    }
    let (volume_identity, file_identity) = platform_identity(&canonical_path, &metadata)?;
    Ok(SourceRootIdentity {
        canonical_path,
        volume_identity,
        file_identity,
    })
}

/// Rebuilds source identity for install-time authority revalidation.
pub(crate) fn current_source_identity(path: &Path) -> Result<SourceRootIdentity, PluginError> {
    identify_source_root(path)
}

#[cfg(windows)]
fn platform_identity(
    path: &Path,
    _metadata: &std::fs::Metadata,
) -> Result<(u64, u128), PluginError> {
    use std::mem::zeroed;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle,
    };

    let directory = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|_| PluginError::SourceChanged {
            reason: SourceChangeReason::RootIdentityMismatch,
        })?;
    let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { zeroed() };
    let succeeded =
        unsafe { GetFileInformationByHandle(directory.as_raw_handle() as _, &mut information) };
    if succeeded == 0 {
        return Err(PluginError::SourceChanged {
            reason: SourceChangeReason::RootIdentityMismatch,
        });
    }
    let file_identity =
        (u128::from(information.nFileIndexHigh) << 32) | u128::from(information.nFileIndexLow);
    Ok((u64::from(information.dwVolumeSerialNumber), file_identity))
}

#[cfg(unix)]
fn platform_identity(
    _path: &Path,
    metadata: &std::fs::Metadata,
) -> Result<(u64, u128), PluginError> {
    use std::os::unix::fs::MetadataExt;

    Ok((metadata.dev(), metadata.ino() as u128))
}

#[cfg(test)]
mod tests {
    use super::{AuthorityClock, CandidateAuthority, ManagementSessionId};
    use crate::{PackageValidator, PluginLimits};
    use ora_plugin_protocol::{CandidateAuditId, PluginVersion};
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::Duration;
    use tempfile::TempDir;

    #[derive(Clone)]
    struct FakeClock(Arc<Mutex<Duration>>);

    impl AuthorityClock for FakeClock {
        fn now(&self) -> Duration {
            *self.0.lock().unwrap_or_else(PoisonError::into_inner)
        }
    }

    /// Selection and candidate tokens are both atomically single-use.
    #[test]
    fn consumes_each_authorization_stage_once() {
        let root =
            TempDir::new().unwrap_or_else(|error| panic!("expected package directory: {error}"));
        fs::create_dir(root.path().join("dist"))
            .unwrap_or_else(|error| panic!("expected dist directory: {error}"));
        fs::write(root.path().join("dist/index.js"), "export default {};")
            .unwrap_or_else(|error| panic!("expected entry: {error}"));
        fs::write(
            root.path().join("package.json"),
            r#"{"name":"@ora/example","version":"0.1.0","type":"module","ora":{"manifestVersion":1,"id":"ora.example","displayName":"Example","kind":"agent","main":"dist/index.js","engines":{"ora":">=0.1.0 <0.2.0","pluginApi":1,"bun":">=1.0.0 <2.0.0"},"contributes":{"agents":[{"id":"example","displayName":"Example","contractVersion":1}]}}}"#,
        )
        .unwrap_or_else(|error| panic!("expected manifest: {error}"));
        let clock = FakeClock(Arc::new(Mutex::new(Duration::ZERO)));
        let authority =
            CandidateAuthority::new(clock, Duration::from_secs(10), Duration::from_secs(10));
        let session = ManagementSessionId::new_random()
            .unwrap_or_else(|error| panic!("expected session: {error}"));
        let audit = CandidateAuditId::parse("550e8400-e29b-41d4-a716-446655440000")
            .unwrap_or_else(|error| panic!("expected audit id: {error}"));
        let selection = authority
            .register_selection(session.clone(), root.path(), audit)
            .unwrap_or_else(|error| panic!("expected selection: {error}"));
        let selection_replay = selection.selection_handle.clone();
        let validator = PackageValidator::new(
            PluginLimits::default(),
            PluginVersion::parse("0.1.0")
                .unwrap_or_else(|error| panic!("expected Host version: {error}")),
            PluginVersion::parse("1.3.14")
                .unwrap_or_else(|error| panic!("expected Bun version: {error}")),
        );
        let identified = authority
            .identify(&session, selection.selection_handle, &validator)
            .unwrap_or_else(|error| panic!("expected identify: {error}"));
        assert_eq!(
            authority
                .identify(&session, selection_replay, &validator)
                .is_err(),
            true
        );
        let candidate_replay = identified.candidate_handle.clone();
        authority
            .consume_candidate(&session, identified.candidate_handle)
            .unwrap_or_else(|error| panic!("expected candidate consume: {error}"));
        assert_eq!(
            authority
                .consume_candidate(&session, candidate_replay)
                .is_err(),
            true
        );
    }
}
