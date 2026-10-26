//! Original-Node proof transport, separate from execution terminals and their receipt/ACK path.
use super::{CloudStore, fault, runtime_control::binding};
use crate::*;
use ora_controller_proto::v1::{
    self as proto, runtime_control_service_client::RuntimeControlServiceClient,
};

#[cfg(test)]
#[path = "../../tests/support/non_acceptance_adapter.rs"]
mod tests;

/// Copies authority without replacing historical incarnation or epoch fields with the current lease.
fn wire_binding(b: &RuntimeBinding) -> proto::RuntimeBinding {
    proto::RuntimeBinding {
        tenant_id: b.tenant_id.clone(),
        workspace_id: b.workspace_id.clone(),
        sandbox_id: b.sandbox_id.clone(),
        runtime_generation: b.runtime_generation,
        node_id: b.node_id.clone(),
        node_incarnation_id: b.node_incarnation_id.clone(),
        node_instance_id: b.node_instance_id.clone(),
        controller_epoch: b.controller_epoch,
        control_epoch: b.control_epoch,
        control_version: b.control_version,
        session_id: b.session_id.clone(),
        actor_user_id: b.actor_user_id.clone(),
        operation_id: b.operation_id.clone(),
        execution_id: b.execution_id.clone(),
        node_operation_id: b.node_operation_id.clone(),
        input_closed: b.input_closed,
        issued_at_ms: b.issued_at_ms,
        expires_at_ms: b.expires_at_ms,
    }
}

impl CloudStore {
    /// Reads original origins that remain unresolved after the Cloud business deadline.
    pub(super) async fn non_acceptance_fences(
        &self,
        node: &NodeId,
    ) -> Result<ExecutionFenceBatch, Error> {
        let epoch = self.epoch()?;
        let response = fault::read(async {
            RuntimeControlServiceClient::new(self.inner.channel.clone())
                .list_execution_fences(self.request(proto::ListExecutionFencesRequest {
                    epoch,
                    node_id: node.as_str().to_owned(),
                }))
                .await
        })
        .await
        .map_err(|v| self.settle(v))?;
        let requests = response
            .fences
            .into_iter()
            .map(|f| {
                let original = f.origin.ok_or(Error::Conflict)?;
                let fence = ExecutionFence {
                    authority: binding(f.authority.ok_or(Error::Conflict)?),
                    origin: ExecutionOrigin {
                        binding: binding(original.binding.ok_or(Error::Conflict)?),
                        work_id: original.work_id,
                        input_json: original.input_json,
                    },
                };
                fence.validate()?;
                Ok(fence)
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let mut committed = Vec::new();
        for id in response.committed_execution_ids {
            if id.is_empty() || id.len() > 256 {
                return Err(Error::Conflict);
            }
            committed.push(ExecutionId::new(id));
        }
        Ok(ExecutionFenceBatch {
            requests,
            committed,
        })
    }

    /// A durable proof gets its own submission; success does not acknowledge a Node event sequence.
    pub(super) async fn accept_non_acceptance(
        &self,
        proof: &ExecutionNonAcceptance,
    ) -> Result<(), Error> {
        proof.validate()?;
        let epoch = self.epoch()?;
        let fence = &proof.fence;
        let wire = proto::ExecutionNonAcceptance {
            proof_id: proof.proof_id.clone(),
            fence: Some(proto::ExecutionFence {
                authority: Some(wire_binding(&fence.authority)),
                origin: Some(proto::ExecutionOrigin {
                    binding: Some(wire_binding(&fence.origin.binding)),
                    work_id: fence.origin.work_id.clone(),
                    input_json: fence.origin.input_json.clone(),
                }),
            }),
        };
        fault::write(|submission_id| {
            let wire = wire.clone();
            async move {
                RuntimeControlServiceClient::new(self.inner.channel.clone())
                    .record_execution_non_acceptance(self.request(
                        proto::RecordExecutionNonAcceptanceRequest {
                            submission_id,
                            epoch,
                            proof: Some(wire),
                        },
                    ))
                    .await
            }
        })
        .await
        .map_err(|v| self.settle(v))?;
        Ok(())
    }
}
