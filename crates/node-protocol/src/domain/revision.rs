//! Revision delivery execution: saves an ended session's Workspace changes and history to the
//! object store with Cloud-issued upload grants, and reports what was uploaded.

use crate::{
    CommitId, ExecutionId, MessageValidationError, NodeId, NodeRuntimeIdentity, Sha256Digest,
};
use ora_utils::GitBranchName;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use time::OffsetDateTime;

/// Namespace every Revision ref must live under, so a delivery can never move a branch or tag.
pub const REVISION_REF_PREFIX: &str = "refs/ora/revisions/";

/// Internal ref that points at a Revision's final commit.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct RevisionRef(String);

impl RevisionRef {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the full ref exactly as carried on the wire.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Requires the Revision namespace and a suffix Git accepts as a ref component path.
    fn validate(&self) -> Result<(), MessageValidationError> {
        let valid = self
            .0
            .strip_prefix(REVISION_REF_PREFIX)
            .is_some_and(|suffix| GitBranchName::parse(suffix).is_ok());
        if valid {
            return Ok(());
        }
        Err(MessageValidationError::InvalidRevisionRef)
    }
}

/// Object-store key chosen by Cloud; the Node uploads to exactly this key.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ObjectKey(String);

impl ObjectKey {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the key exactly as carried on the wire.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Rejects keys that are empty, absolute-looking, or contain dot segments or control
    /// characters, which some stores and proxies normalize into a different object.
    pub(crate) fn validate(&self) -> Result<(), MessageValidationError> {
        let valid = !self.0.is_empty()
            && self.0.len() <= 1024
            && !self.0.starts_with('/')
            && !self
                .0
                .split('/')
                .any(|segment| matches!(segment, "" | "." | ".."))
            && !self.0.chars().any(char::is_control);
        if valid {
            return Ok(());
        }
        Err(MessageValidationError::InvalidObjectKey)
    }
}

/// Saves the ended session's Workspace and history.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeliverRevisionSpec {
    pub node_id: NodeId,
    /// Must have a terminal result and a closed process scope before delivery starts.
    pub session_execution_id: ExecutionId,
    pub checkout_execution_id: ExecutionId,
    /// Commit of the clone; the bundle is relative to it.
    pub base_commit: CommitId,
    pub revision_ref: RevisionRef,
    pub bundle_key: ObjectKey,
    pub history_key: ObjectKey,
}

impl DeliverRevisionSpec {
    /// Rejects inputs that could write outside the Revision namespace or the delivery's keys.
    pub(crate) fn validate(&self) -> Result<(), MessageValidationError> {
        if self.node_id.is_empty() {
            return Err(MessageValidationError::EmptyField { field: "node_id" });
        }
        if self.session_execution_id.is_empty() {
            return Err(MessageValidationError::EmptyField {
                field: "session_execution_id",
            });
        }
        if self.checkout_execution_id.is_empty() {
            return Err(MessageValidationError::EmptyField {
                field: "checkout_execution_id",
            });
        }
        validate_commit(&self.base_commit)?;
        self.revision_ref.validate()?;
        self.bundle_key.validate()?;
        self.history_key.validate()?;
        if self.bundle_key == self.history_key {
            return Err(MessageValidationError::InvalidObjectKey);
        }
        Ok(())
    }
}

/// An object the Node uploaded, as the Node measured it before upload.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoredObject {
    pub key: ObjectKey,
    pub size: u64,
    pub sha256: Sha256Digest,
}

impl StoredObject {
    /// Checks the key and canonical digest.
    fn validate(&self) -> Result<(), MessageValidationError> {
        self.key.validate()?;
        self.sha256.validate()
    }
}

/// The final commit differs from the base: a bundle and the history were uploaded.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionDelivered {
    pub node: NodeRuntimeIdentity,
    pub final_commit: CommitId,
    pub base_commit: CommitId,
    pub revision_ref: RevisionRef,
    pub bundle: StoredObject,
    pub history: StoredObject,
}

/// The final commit equals the base: only the history was uploaded.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionUnchanged {
    pub node: NodeRuntimeIdentity,
    pub final_commit: CommitId,
    pub base_commit: CommitId,
    pub revision_ref: RevisionRef,
    pub history: StoredObject,
}

/// Definitive delivery failures; raw Git and HTTP diagnostics stay in Node logs.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisionFailureCode {
    SessionNotSettled,
    CheckoutUnavailable,
    SnapshotFailed,
    BundleFailed,
    HistoryUnavailable,
    UploadFailed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionFailed {
    pub node: NodeRuntimeIdentity,
    pub failure: RevisionFailureCode,
}

/// Delivery's disjoint wire tags cannot be decoded as another business's result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "result", rename_all = "snake_case")]
pub enum RevisionExecutionResult {
    RevisionDelivered(RevisionDelivered),
    RevisionUnchanged(RevisionUnchanged),
    RevisionFailed(RevisionFailed),
}

impl RevisionExecutionResult {
    /// Keeps "changed" and "unchanged" distinguishable by the commits themselves, so Cloud never
    /// registers a bundle-less Revision whose final commit differs from its base.
    pub(crate) fn validate(&self) -> Result<(), MessageValidationError> {
        self.node()
            .validate()
            .map_err(|field| MessageValidationError::EmptyField { field })?;
        match self {
            Self::RevisionDelivered(result) => {
                validate_commit(&result.final_commit)?;
                validate_commit(&result.base_commit)?;
                result.revision_ref.validate()?;
                result.bundle.validate()?;
                result.history.validate()?;
                if result.final_commit == result.base_commit {
                    return Err(MessageValidationError::RevisionCommitMismatch);
                }
                Ok(())
            }
            Self::RevisionUnchanged(result) => {
                validate_commit(&result.final_commit)?;
                result.revision_ref.validate()?;
                result.history.validate()?;
                if result.final_commit != result.base_commit {
                    return Err(MessageValidationError::RevisionCommitMismatch);
                }
                Ok(())
            }
            Self::RevisionFailed(_) => Ok(()),
        }
    }

    /// Preserves the original incarnation when a restarted Node reports stored evidence.
    pub(crate) fn node(&self) -> &NodeRuntimeIdentity {
        match self {
            Self::RevisionDelivered(result) => &result.node,
            Self::RevisionUnchanged(result) => &result.node,
            Self::RevisionFailed(result) => &result.node,
        }
    }
}

/// Requires a full hexadecimal SHA-1 or SHA-256 object ID.
pub(crate) fn validate_commit(commit: &CommitId) -> Result<(), MessageValidationError> {
    let commit = commit.as_str();
    if matches!(commit.len(), 40 | 64) && commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Ok(());
    }
    Err(MessageValidationError::InvalidCommit)
}

/// Presigned upload URL. It is a bearer credential, so `Debug` never prints it.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct PresignedUrl(String);

impl PresignedUrl {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Exposes the URL only to the uploader that sends the request.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for PresignedUrl {
    /// Keeps grants out of diagnostics that format enclosing messages.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PresignedUrl([redacted])")
    }
}

/// HTTP method an upload grant was signed for.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum UploadMethod {
    Put,
}

/// A presigned single-object upload, held only in memory by every party.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectUploadGrant {
    pub object_key: ObjectKey,
    pub url: PresignedUrl,
    pub method: UploadMethod,
    /// Headers the upload must carry unchanged, including the signed `If-None-Match: *` and, for a
    /// grant requested with the object's checksum, the signed `x-amz-checksum-sha256`.
    pub headers: BTreeMap<String, String>,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
}

impl ObjectUploadGrant {
    /// Requires an absolute HTTP(S) URL and a valid key.
    pub(crate) fn validate(&self) -> Result<(), MessageValidationError> {
        self.object_key.validate()?;
        let url_ok = url::Url::parse(self.url.as_str())
            .is_ok_and(|url| matches!(url.scheme(), "https" | "http") && url.host_str().is_some());
        if url_ok {
            return Ok(());
        }
        Err(MessageValidationError::InvalidUploadGrant)
    }
}
