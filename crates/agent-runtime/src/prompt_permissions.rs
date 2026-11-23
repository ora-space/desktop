//! Pending permission writes and replies deferred during a finite wait's deadline arbitration.
use crate::RuntimeError;
use crate::connection::AgentAcpClient;
use crate::prompt_liveness::PromptLiveness;
use crate::support::{pick_auto_allow_option, respond_permission, runtime_unavailable_because};
use agent_client_protocol_schema::v1::RequestId;
use ora_acp::PermissionRequest;
use ora_contracts::RespondToPermissionRequest;
use ora_domain::SessionId;
use ora_logging::ora_debug;
use std::collections::{HashMap, VecDeque};
use tokio::time::Instant;

#[derive(Default)]
pub(super) struct PromptPermissions {
    pub(super) pending: HashMap<String, (RequestId, Vec<String>)>,
    deferred: VecDeque<PermissionRequest>,
}

impl PromptPermissions {
    /// Resume only unanswered requests when a queued progress event renewed the window.
    pub(super) fn take_deferred(&mut self) -> Option<PermissionRequest> {
        while let Some(request) = self.deferred.pop_front() {
            if self.pending.contains_key(&request.request_id.to_string()) {
                return Some(request);
            }
        }
        None
    }

    /// Register before attempting a write so expiry can cancel an unanswered request.
    pub(super) async fn answer(
        &mut self,
        client: &AgentAcpClient,
        session_id: &SessionId,
        permission: PermissionRequest,
        liveness: &mut PromptLiveness,
    ) -> Result<(), RuntimeError> {
        let id = permission.request_id.to_string();
        self.pending.entry(id.clone()).or_insert_with(|| {
            (
                permission.request_id.clone(),
                permission
                    .request
                    .options
                    .iter()
                    .map(|option| option.option_id.to_string())
                    .collect(),
            )
        });
        let deadline = liveness.wait_deadline();
        if deadline.is_some_and(|end| Instant::now() >= end) {
            self.deferred.push_back(permission);
            return Ok(());
        }
        let option = pick_auto_allow_option(&permission.request.options).ok_or_else(|| {
            runtime_unavailable_because("permission request offered no allow option")
        })?;
        ora_debug!(%session_id, request_id = %id, option_count = permission.request.options.len(), "answering permission request");
        let reply = respond_permission(
            client,
            RespondToPermissionRequest {
                session_id: session_id.to_string(),
                permission_request_id: id,
                option_id: option.option_id.to_string(),
            },
            &mut self.pending,
        );
        if let Some(deadline) = deadline {
            tokio::select! {
                biased;
                () = tokio::time::sleep_until(deadline) => {
                    self.deferred.push_back(permission);
                    return Ok(());
                }
                result = reply => { result?; }
            }
        } else {
            reply.await?;
        }
        liveness.permission_settled(&permission.request.tool_call);
        Ok(())
    }
}
