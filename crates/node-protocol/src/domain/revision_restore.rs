//! Resuming a session from the latest Revision of its Issue: the prior Revision a session restores
//! before its agent starts, the commit a delivery compares against, and the memory-only download
//! grant for the prior bundle.

use super::revision::{PresignedUrl, validate_commit};
use crate::{CommitId, MessageValidationError, ObjectKey, RevisionId, StoredObject};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use time::OffsetDateTime;

/// The Revision a session resumes, fixed by Cloud when it released the session. The bundle names
/// the verified object holding it, which may belong to an earlier Revision the prior one reused.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PriorRevision {
    pub revision_id: RevisionId,
    pub final_commit: CommitId,
    pub bundle: StoredObject,
}

impl PriorRevision {
    /// Requires an identity, a full commit and a well-formed object description, so the Node can
    /// verify the downloaded bytes and the restored commit against exactly these values.
    pub(crate) fn validate(&self) -> Result<(), MessageValidationError> {
        if self.revision_id.is_empty() {
            return Err(MessageValidationError::EmptyField {
                field: "prior_revision.revision_id",
            });
        }
        validate_commit(&self.final_commit)?;
        self.bundle.validate()
    }

    /// The part a delivery of the same run compares its final commit against.
    pub fn commit(&self) -> PriorRevisionCommit {
        PriorRevisionCommit {
            revision_id: self.revision_id.clone(),
            final_commit: self.final_commit.clone(),
        }
    }
}

/// The prior Revision as a delivery sees it: a final commit equal to this one is unchanged.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PriorRevisionCommit {
    pub revision_id: RevisionId,
    pub final_commit: CommitId,
}

impl PriorRevisionCommit {
    /// Requires an identity and a full commit.
    pub(crate) fn validate(&self) -> Result<(), MessageValidationError> {
        if self.revision_id.is_empty() {
            return Err(MessageValidationError::EmptyField {
                field: "prior_revision.revision_id",
            });
        }
        validate_commit(&self.final_commit)
    }
}

/// HTTP method a download grant was signed for.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum DownloadMethod {
    Get,
}

/// A presigned single-object read of a prior bundle, held only in memory by every party.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectDownloadGrant {
    pub object_key: ObjectKey,
    pub url: PresignedUrl,
    pub method: DownloadMethod,
    /// Headers the download must carry unchanged.
    pub headers: BTreeMap<String, String>,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
}

impl ObjectDownloadGrant {
    /// Requires an absolute HTTP(S) URL and a valid key.
    pub(crate) fn validate(&self) -> Result<(), MessageValidationError> {
        self.object_key.validate()?;
        if self.url.is_absolute_http() {
            return Ok(());
        }
        Err(MessageValidationError::InvalidDownloadGrant)
    }
}
