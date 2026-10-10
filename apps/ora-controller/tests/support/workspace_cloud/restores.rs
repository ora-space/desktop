//! Prior-Revision authority of the fake Cloud: resumed session inputs and read grants for their
//! prior bundle, issued only for a registered session without a result, as the real Cloud does.
use super::*;

/// The prior bundle every resumed session of these tests restores.
pub const PRIOR_BUNDLE: &str = "runs/prior/revision.bundle";
/// The final commit of that prior Revision.
pub const PRIOR_FINAL: &str = "fedcba9876543210fedcba9876543210fedcba98";
const PRIOR_DIGEST: &str = "3333333333333333333333333333333333333333333333333333333333333333";

#[derive(Default)]
pub(super) struct RestoreState {
    /// Every download grant response Cloud issued, with its request, in order.
    issued: Vec<(
        proto::GrantRevisionDownloadRequest,
        proto::GrantRevisionDownloadResponse,
    )>,
    /// Refuses every download grant as Cloud does once the session cannot restore.
    refuse: bool,
    /// Answers this many download grant calls with UNAVAILABLE before serving again.
    outages: usize,
}

impl WorkspaceCloud {
    /// The prior Revision a resumed session restores.
    pub fn prior_revision() -> proto::PriorRevision {
        proto::PriorRevision {
            revision_id: "revision-1".into(),
            final_commit: PRIOR_FINAL.into(),
            bundle: Some(proto::StoredObject {
                key: PRIOR_BUNDLE.into(),
                size: 42,
                sha256: PRIOR_DIGEST.into(),
            }),
        }
    }

    /// Queues a session of `run` that resumes [`WorkspaceCloud::prior_revision`].
    pub fn queue_resumed_agent(&self, run: &str) {
        self.queue_session(run, Some(Self::prior_revision()));
    }

    /// Makes Cloud refuse download grants with CONFLICT, as after the session ended.
    pub fn refuse_downloads(&self, refuse: bool) {
        self.lock().agents.restores.refuse = refuse;
    }

    /// Makes the next `count` download grant calls fail as a Cloud outage would.
    pub fn fail_downloads(&self, count: usize) {
        self.lock().agents.restores.outages = count;
    }

    /// Every download grant Cloud issued so far, with the request that asked for it.
    pub fn issued_downloads(
        &self,
    ) -> Vec<(
        proto::GrantRevisionDownloadRequest,
        proto::GrantRevisionDownloadResponse,
    )> {
        self.lock().agents.restores.issued.clone()
    }

    /// Signs one read of the prior bundle for a registered resumed session without a result.
    pub(super) fn grant_download(
        &self,
        message: proto::GrantRevisionDownloadRequest,
    ) -> Result<Response<proto::GrantRevisionDownloadResponse>, Status> {
        let mut state = self.lock();
        if message.epoch != EPOCH {
            return Err(Status::failed_precondition("stale_controller"));
        }
        if state.agents.restores.outages > 0 {
            state.agents.restores.outages -= 1;
            return Err(Status::unavailable("persistence unavailable"));
        }
        let record = state
            .clones
            .iter()
            .find(|r| r.execution_id == message.execution_id)
            .cloned()
            .ok_or_else(|| Status::not_found("not_found"))?;
        let Some(proto::execution_input::Spec::AgentSession(spec)) =
            record.input.as_ref().and_then(|i| i.spec.as_ref())
        else {
            return Err(Status::not_found("not_found"));
        };
        let key = spec
            .prior_revision
            .as_ref()
            .and_then(|prior| prior.bundle.as_ref())
            .map(|bundle| bundle.key.clone());
        let (Some(key), false, None) = (key, state.agents.restores.refuse, &record.result) else {
            drop(state);
            self.timeline.push(Event::DownloadRefused {
                execution: message.execution_id,
            });
            return Err(conflict("dispatch_conflict"));
        };
        let serial = state.agents.restores.issued.len();
        let response = proto::GrantRevisionDownloadResponse {
            grants: vec![proto::DownloadGrant {
                url: format!(
                    "https://{STORE_HOST}/{key}?X-Amz-Signature={SIGNATURE}-read-{serial}"
                ),
                object_key: key,
                method: "GET".into(),
                headers: [(
                    "x-amz-meta-proof".to_owned(),
                    format!("{SIGNATURE}-read-{serial}"),
                )]
                .into_iter()
                .collect(),
                expires_at: Some(prost_types::Timestamp {
                    seconds: 1_900_000_000 + serial as i64,
                    nanos: 0,
                }),
            }],
        };
        state
            .agents
            .restores
            .issued
            .push((message.clone(), response.clone()));
        drop(state);
        self.timeline.push(Event::DownloadIssued {
            execution: message.execution_id,
        });
        Ok(Response::new(response))
    }
}
