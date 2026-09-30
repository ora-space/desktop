//! Simulated Cloud permits test coordination only; real PostgreSQL and Node tests enforce fences.
use super::*;
use ora_controller_proto::v1::runtime_control_service_server::RuntimeControlService;
impl WorkspaceCloud {
    fn runtime_bindings(&self) -> Vec<proto::RuntimeBinding> {
        let state = self.lock();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let op = state.ops.iter().find(|o| {
            matches!(
                o.operation.state(),
                proto::OperationState::Running
                    | proto::OperationState::Queued
                    | proto::OperationState::RetryWait
                    | proto::OperationState::Blocked
            )
        });
        let operations = op.map(|o| o.operation.id.clone()).into_iter().chain(
            state
                .clones
                .iter()
                .filter(|r| {
                    matches!(
                        r.input.as_ref().and_then(|i| i.spec.as_ref()),
                        Some(proto::execution_input::Spec::AgentSession(_))
                    )
                })
                .map(|r| r.operation_id.clone()),
        );
        let generation = state.workspace.runtime_generation;
        let input_closed = state.agents.input_closed;
        operations
            .flat_map(|operation| {
                state
                    .nodes
                    .iter()
                    .filter(|n| n.connection == proto::NodeConnection::Connected as i32)
                    .map(move |n| {
                        let node = n.identity.as_ref().unwrap();
                        proto::RuntimeBinding {
                            tenant_id: "tenant".into(),
                            workspace_id: WORKSPACE.into(),
                            sandbox_id: n.sandbox_instance_id.clone(),
                            runtime_generation: generation,
                            node_id: node.node_id.clone(),
                            node_incarnation_id: node.node_incarnation_id.clone(),
                            node_instance_id: n.id.clone(),
                            controller_epoch: EPOCH,
                            control_epoch: 1,
                            control_version: 1,
                            session_id: String::new(),
                            actor_user_id: "original-actor".into(),
                            operation_id: operation.clone(),
                            execution_id: String::new(),
                            node_operation_id: String::new(),
                            input_closed,
                            issued_at_ms: now,
                            expires_at_ms: now + 60000,
                        }
                    })
            })
            .collect()
    }
}
#[tonic::async_trait]
impl RuntimeControlService for WorkspaceCloud {
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
        let message = request.into_inner();
        let record = self
            .lock()
            .clones
            .iter()
            .find(|r| r.execution_id == message.execution_id && r.result.is_none())
            .cloned()
            .ok_or_else(|| conflict("stale_execution"))?;
        let mut binding = self
            .runtime_bindings()
            .into_iter()
            .find(|b| b.operation_id == record.operation_id)
            .ok_or_else(|| conflict("runtime_control_required"))?;
        binding.execution_id = record.execution_id;
        binding.node_operation_id = record.node_operation_id;
        Ok(Response::new(proto::GetExecutionPermitResponse {
            binding: Some(binding),
        }))
    }
    async fn get_effect_permit(
        &self,
        request: Request<proto::GetEffectPermitRequest>,
    ) -> Result<Response<proto::GetEffectPermitResponse>, Status> {
        let message = request.into_inner();
        let state = self.lock();
        let (_, effect) = state
            .effects
            .iter()
            .find(|(_, e)| e.id == message.effect_id)
            .ok_or_else(|| conflict("stale_effect"))?;
        let sandbox_id = match effect.request.as_ref().and_then(|r| r.request.as_ref()) {
            Some(EffectRequest::SandboxEnsure(_)) => effect.id.clone(),
            Some(EffectRequest::SandboxTerminate(r)) => r.sandbox_instance_id.clone(),
            _ => String::new(),
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        Ok(Response::new(proto::GetEffectPermitResponse {
            permit: Some(proto::RuntimeEffectPermit {
                tenant_id: "tenant".into(),
                workspace_id: WORKSPACE.into(),
                sandbox_id,
                runtime_generation: state.workspace.runtime_generation,
                controller_epoch: EPOCH,
                control_epoch: 1,
                issued_at_ms: now,
                expires_at_ms: now + 10000,
            }),
        }))
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
