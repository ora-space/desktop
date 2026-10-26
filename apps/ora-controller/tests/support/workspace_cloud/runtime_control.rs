//! Simulated Cloud permits test coordination only; real PostgreSQL and Node tests enforce fences.
use super::*;
use ora_controller_proto::v1::runtime_control_service_server::RuntimeControlService;
use std::collections::HashMap;

/// Test-owned Cloud intentions and replies; no caller role is assumed by the delivery contract.
#[derive(Default)]
pub(super) struct ForceState {
    plans: Vec<proto::RuntimeForceStopPlan>,
    permits: HashMap<String, Result<proto::RuntimeEffectPermit, Status>>,
    requests: Vec<proto::GetEffectPermitRequest>,
    confirmations: Vec<proto::ConfirmForceStopRequest>,
    next_permit_lifetime: Option<Duration>,
    replies: Vec<(proto::GetEffectPermitRequest, proto::RuntimeEffectPermit)>,
}

impl WorkspaceCloud {
    /// Changes only the next permit, so a resumed operation receives normal fresh authority.
    pub fn next_effect_permit_lifetime(&self, lifetime: Duration) {
        self.lock().force.next_permit_lifetime = Some(lifetime);
    }

    /// Records every permit issued, in order, so tests can prove a retry obtained fresh authority.
    pub fn effect_permit_replies(
        &self,
    ) -> Vec<(proto::GetEffectPermitRequest, proto::RuntimeEffectPermit)> {
        self.lock().force.replies.clone()
    }

    /// Installs exact plans and optional permit outcomes before the next Controller wake.
    pub fn force_plans(
        &self,
        plans: Vec<proto::RuntimeForceStopPlan>,
        permits: HashMap<String, Result<proto::RuntimeEffectPermit, Status>>,
    ) {
        let mut state = self.lock();
        state.force.plans = plans;
        state.force.permits = permits;
        // A committed intention wakes an already live watch, rather than waiting for lease renewal.
        if let Some(subscriber) = &state.subscriber {
            let _ = subscriber.try_send(Ok(proto::WatchResponse {
                signal: Some(Signal::WorkAvailable(proto::WorkAvailable {
                    operation_id: None,
                })),
            }));
        }
    }

    /// Returns immutable evidence of the worker's requests and accepted confirmations.
    pub fn force_calls(
        &self,
    ) -> (
        Vec<proto::GetEffectPermitRequest>,
        Vec<proto::ConfirmForceStopRequest>,
    ) {
        let state = self.lock();
        (
            state.force.requests.clone(),
            state.force.confirmations.clone(),
        )
    }

    /// Returns planned effects and generation together, so tombstone tests detect replacement work.
    pub fn effect_identity(&self) -> (Vec<proto::Effect>, i64) {
        let state = self.lock();
        (
            state
                .effects
                .iter()
                .map(|(_, effect)| effect.clone())
                .collect(),
            state.workspace.runtime_generation,
        )
    }

    /// Reoffers retry-wait work as a normal future claim would; blocked work stays blocked.
    pub fn reoffer_retries(&self) {
        for op in &mut self.lock().ops {
            if op.operation.state() == proto::OperationState::RetryWait {
                op.operation.state = proto::OperationState::Queued as i32;
            }
        }
    }

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
                        Some(
                            proto::execution_input::Spec::AgentSession(_)
                                | proto::execution_input::Spec::DeliverRevision(_)
                        )
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
    async fn list_execution_fences(
        &self,
        _: Request<proto::ListExecutionFencesRequest>,
    ) -> Result<Response<proto::ListExecutionFencesResponse>, Status> {
        Ok(Response::new(proto::ListExecutionFencesResponse::default()))
    }
    async fn record_execution_non_acceptance(
        &self,
        _: Request<proto::RecordExecutionNonAcceptanceRequest>,
    ) -> Result<Response<proto::RecordExecutionNonAcceptanceResponse>, Status> {
        Err(Status::failed_precondition("no_execution_fence"))
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
        let mut state = self.lock();
        state.force.requests.push(message.clone());
        if !message.force_stop_id.is_empty() {
            self.timeline.push(Event::ForceProgress);
        }
        if let Some(outcome) = state.force.permits.get(&message.effect_id) {
            return outcome.clone().map(|permit| {
                Response::new(proto::GetEffectPermitResponse {
                    permit: Some(permit),
                })
            });
        }
        let effect = state
            .effects
            .iter()
            .find(|(_, e)| e.id == message.effect_id)
            .map(|(_, effect)| effect)
            .or_else(|| {
                state
                    .force
                    .plans
                    .iter()
                    .flat_map(|plan| &plan.effects)
                    .find(|effect| effect.id == message.effect_id)
            })
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
        let lifetime = state
            .force
            .next_permit_lifetime
            .take()
            .unwrap_or(Duration::from_secs(/*secs*/ 10));
        let permit = proto::RuntimeEffectPermit {
            tenant_id: "tenant".into(),
            workspace_id: WORKSPACE.into(),
            sandbox_id,
            runtime_generation: state.workspace.runtime_generation,
            controller_epoch: EPOCH,
            control_epoch: 1,
            issued_at_ms: now,
            expires_at_ms: now + lifetime.as_millis() as i64,
        };
        state.force.replies.push((message, permit.clone()));
        Ok(Response::new(proto::GetEffectPermitResponse {
            permit: Some(permit),
        }))
    }
    async fn list_force_stops(
        &self,
        _: Request<proto::ListForceStopsRequest>,
    ) -> Result<Response<proto::ListForceStopsResponse>, Status> {
        self.timeline.push(Event::ForceListed);
        Ok(Response::new(proto::ListForceStopsResponse {
            plans: self.lock().force.plans.clone(),
        }))
    }
    async fn confirm_force_stop(
        &self,
        request: Request<proto::ConfirmForceStopRequest>,
    ) -> Result<Response<proto::ConfirmForceStopResponse>, Status> {
        let message = request.into_inner();
        let mut state = self.lock();
        let plan = state
            .force
            .plans
            .iter_mut()
            .find(|plan| plan.id == message.force_stop_id)
            .ok_or_else(|| Status::aborted("no_force_intent"))?;
        if plan.version != message.version || !message.terminated || !message.late_ensure_fenced {
            return Err(Status::aborted("unconfirmed_target"));
        }
        let effect = plan
            .effects
            .iter_mut()
            .find(|effect| effect.id == message.effect_id)
            .ok_or_else(|| Status::not_found("effect_not_found"))?;
        effect.state = proto::EffectState::Succeeded as i32;
        plan.version += 1;
        let version = plan.version;
        state.force.confirmations.push(message);
        // An empty plan is no longer a pending intention, as in the production listing.
        state.force.plans.retain(|plan| {
            plan.effects
                .iter()
                .any(|effect| effect.state() != proto::EffectState::Succeeded)
        });
        self.timeline.push(Event::ForceProgress);
        Ok(Response::new(proto::ConfirmForceStopResponse {
            version,
            completed: state.force.plans.is_empty(),
        }))
    }
}
