//! Simulated Cloud permits test coordination only; real PostgreSQL and Node tests enforce fences.
use super::*;
use ora_controller_proto::v1::runtime_control_service_server::RuntimeControlService;
impl FakeCloud {
    fn runtime_bindings(&self) -> Vec<proto::RuntimeBinding> {
        vec![]
    }
}
#[tonic::async_trait]
impl RuntimeControlService for FakeCloud {
    async fn list_execution_fences(
        &self,
        request: Request<proto::ListExecutionFencesRequest>,
    ) -> Result<Response<proto::ListExecutionFencesResponse>, Status> {
        let r = request.into_inner();
        let state = self.lock();
        if state.epoch != Some(r.epoch) {
            return Err(Status::failed_precondition("stale_controller"));
        }
        let committed: Vec<_> = state
            .non_acceptance_proofs
            .iter()
            .filter_map(|proof| {
                let binding = proof.fence.as_ref()?.origin.as_ref()?.binding.as_ref()?;
                (binding.node_id == r.node_id).then(|| binding.execution_id.clone())
            })
            .collect();
        let fences = state
            .execution_fences
            .iter()
            .filter(|f| {
                f.origin
                    .as_ref()
                    .and_then(|o| o.binding.as_ref())
                    .is_some_and(|b| b.node_id == r.node_id && !committed.contains(&b.execution_id))
            })
            .cloned()
            .collect();
        Ok(Response::new(proto::ListExecutionFencesResponse {
            fences,
            committed_execution_ids: committed,
        }))
    }
    async fn record_execution_non_acceptance(
        &self,
        request: Request<proto::RecordExecutionNonAcceptanceRequest>,
    ) -> Result<Response<proto::RecordExecutionNonAcceptanceResponse>, Status> {
        let r = request.into_inner();
        self.update(|state| {
            state.non_acceptance_attempts += 1;
            if state.reject_proof_writes {
                return Err(Status::unavailable("proof_write_failed"));
            }
            if state.epoch != Some(r.epoch) {
                return Err(Status::failed_precondition("stale_controller"));
            }
            let proof = r
                .proof
                .ok_or_else(|| Status::invalid_argument("proof_required"))?;
            if !state
                .execution_fences
                .iter()
                .any(|f| Some(&f.origin) == proof.fence.as_ref().map(|p| &p.origin))
            {
                return Err(Status::aborted("execution_origin_conflict"));
            }
            if !state.non_acceptance_proofs.contains(&proof) {
                state.non_acceptance_proofs.push(proof);
            }
            Ok(Response::new(
                proto::RecordExecutionNonAcceptanceResponse {},
            ))
        })
    }
    async fn list_bindings(
        &self,
        _: Request<proto::ListBindingsRequest>,
    ) -> Result<Response<proto::ListBindingsResponse>, Status> {
        Ok(Response::new(proto::ListBindingsResponse {
            bindings: self.runtime_bindings(),
        }))
    }
    async fn acknowledge_binding(
        &self,
        _: Request<proto::AcknowledgeBindingRequest>,
    ) -> Result<Response<proto::AcknowledgeBindingResponse>, Status> {
        Ok(Response::new(proto::AcknowledgeBindingResponse {}))
    }
    async fn get_execution_permit(
        &self,
        request: Request<proto::GetExecutionPermitRequest>,
    ) -> Result<Response<proto::GetExecutionPermitResponse>, Status> {
        let _ = request;
        Err(Status::aborted("runtime_control_required"))
    }
    async fn get_effect_permit(
        &self,
        request: Request<proto::GetEffectPermitRequest>,
    ) -> Result<Response<proto::GetEffectPermitResponse>, Status> {
        let _ = request;
        Err(Status::aborted("executor_capability_unavailable"))
    }
    async fn list_force_stops(
        &self,
        _: Request<proto::ListForceStopsRequest>,
    ) -> Result<Response<proto::ListForceStopsResponse>, Status> {
        Ok(Response::new(proto::ListForceStopsResponse {
            plans: vec![],
        }))
    }
    async fn confirm_force_stop(
        &self,
        _: Request<proto::ConfirmForceStopRequest>,
    ) -> Result<Response<proto::ConfirmForceStopResponse>, Status> {
        Err(Status::failed_precondition("no_force_intent"))
    }
}
