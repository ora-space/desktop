//! Prior-Revision inputs of the fake Cloud: resumed sessions name where the Node fetches the
//! prior Revision on the Git Server, as the real Cloud fixes them when releasing the session.
use super::*;

/// The final commit of the prior Revision every resumed session of these tests restores.
pub const PRIOR_FINAL: &str = "fedcba9876543210fedcba9876543210fedcba98";
/// The base commit that prior Revision was built on.
pub const PRIOR_BASE: &str = "89abcdef0123456789abcdef0123456789abcdef";
/// The delivery ref holding the prior final commit.
pub const PRIOR_DELIVERY_REF: &str = "refs/ora/deliveries/revision-1";

impl WorkspaceCloud {
    /// The prior Revision a resumed session restores.
    pub fn prior_revision() -> proto::PriorRevision {
        proto::PriorRevision {
            revision_id: "revision-1".into(),
            final_commit: PRIOR_FINAL.into(),
            git_repository_url: REPOSITORY.into(),
            delivery_ref: PRIOR_DELIVERY_REF.into(),
            base_commit: PRIOR_BASE.into(),
        }
    }

    /// Queues a session of `run` that resumes [`WorkspaceCloud::prior_revision`].
    pub fn queue_resumed_agent(&self, run: &str) {
        self.queue_session(run, Some(Self::prior_revision()));
    }
}
