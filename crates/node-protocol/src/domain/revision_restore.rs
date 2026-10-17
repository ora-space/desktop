//! Resuming a session from the latest Revision of its Issue: the prior Revision a session restores
//! from the Git Server before its agent starts, and the commit a delivery compares against.

use super::revision::validate_commit;
use crate::{CommitId, DeliveryRef, GitRepositoryUrl, MessageValidationError, RevisionId};
use serde::{Deserialize, Serialize};

/// The Revision a session resumes, fixed by Cloud when it released the session: where the Node
/// fetches its final commit, and the base it was built on, which tells whether the Workspace
/// snapshot still contains that history.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PriorRevision {
    pub revision_id: RevisionId,
    pub final_commit: CommitId,
    pub git_repository_url: GitRepositoryUrl,
    pub delivery_ref: DeliveryRef,
    pub base_commit: CommitId,
}

impl PriorRevision {
    /// Requires an identity and full commits, so the Node can verify the fetched commit against
    /// exactly these values.
    pub(crate) fn validate(&self) -> Result<(), MessageValidationError> {
        if self.revision_id.is_empty() {
            return Err(MessageValidationError::EmptyField {
                field: "prior_revision.revision_id",
            });
        }
        validate_commit(&self.final_commit)?;
        validate_commit(&self.base_commit)
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
