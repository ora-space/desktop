use super::{CloudStore, fault};
use crate::*;
use ora_controller_proto::v1::{
    self as proto, runtime_control_service_client::RuntimeControlServiceClient,
};

fn binding(p: proto::RuntimeBinding) -> RuntimeBinding {
    RuntimeBinding {
        tenant_id: p.tenant_id,
        workspace_id: p.workspace_id,
        sandbox_id: p.sandbox_id,
        runtime_generation: p.runtime_generation,
        node_id: p.node_id,
        node_incarnation_id: p.node_incarnation_id,
        node_instance_id: p.node_instance_id,
        controller_epoch: p.controller_epoch,
        control_epoch: p.control_epoch,
        control_version: p.control_version,
        session_id: p.session_id,
        actor_user_id: p.actor_user_id,
        operation_id: p.operation_id,
        execution_id: p.execution_id,
        node_operation_id: p.node_operation_id,
        input_closed: p.input_closed,
        issued_at_ms: p.issued_at_ms,
        expires_at_ms: p.expires_at_ms,
    }
}

impl CloudStore {
    pub(super) async fn control_bindings(
        &self,
        node: &NodeId,
    ) -> Result<Vec<RuntimeBinding>, Error> {
        let epoch = self.epoch()?;
        let response = fault::read(async {
            RuntimeControlServiceClient::new(self.inner.channel.clone())
                .list_bindings(self.request(proto::ListBindingsRequest { epoch }))
                .await
        })
        .await
        .map_err(|v| self.settle(v))?;
        response
            .bindings
            .into_iter()
            .filter(|p| p.node_id == node.as_str())
            .map(|p| {
                let value = binding(p);
                value.validate()?;
                Ok(value)
            })
            .collect()
    }

    pub(super) async fn control_ack(&self, state: &RuntimeControlState) -> Result<(), Error> {
        // A running fixed activity retains its responsibility; only closure reports all liabilities.
        if !state.binding.input_closed && !state.unfinished_execution_ids.is_empty() {
            return Ok(());
        }
        let epoch = self.epoch()?;
        if state.binding.controller_epoch != epoch {
            // A late acknowledgement cannot revoke the current management lease or reopen input.
            return Ok(());
        }
        let outcome = fault::write(|submission_id| async move {
            RuntimeControlServiceClient::new(self.inner.channel.clone())
                .acknowledge_binding(self.request(proto::AcknowledgeBindingRequest {
                    submission_id,
                    epoch,
                    workspace_id: state.binding.workspace_id.clone(),
                    node_instance_id: state.binding.node_instance_id.clone(),
                    control_epoch: state.binding.control_epoch,
                    control_version: state.binding.control_version,
                    input_closed: state.binding.input_closed,
                    unfinished_execution_ids: state.unfinished_execution_ids.clone(),
                }))
                .await
        })
        .await;
        match outcome {
            Ok(_) | Err(fault::Verdict::StaleRuntimeBinding) => Ok(()),
            Err(verdict) => Err(self.settle(verdict)),
        }
    }

    /// Wraps a clone only after obtaining fresh authority for its registered identity.
    pub(super) async fn controlled_dispatch(
        &self,
        command: CloneRepositoryMessage,
    ) -> Result<Option<ControllerToNodeMessage>, Error> {
        let Some(permit) = self
            .execution_permit(&command.execution_id, &command.payload.spec.node_id)
            .await?
        else {
            return Ok(None);
        };
        let message = ControllerToNodeMessage::ControlledClone(ControlledClone {
            binding: permit,
            command,
        });
        message.validate()?;
        Ok(Some(message))
    }

    /// Plugin installation has the same runtime fencing boundary as clone filesystem changes.
    pub(super) async fn controlled_plugins(
        &self,
        command: PluginCommand,
    ) -> Result<Option<ControllerToNodeMessage>, Error> {
        let Some(permit) = self
            .execution_permit(command.execution_id(), command.node_id())
            .await?
        else {
            return Ok(None);
        };
        let message = ControllerToNodeMessage::ControlledPlugins(ControlledPlugins {
            binding: permit,
            command,
        });
        message.validate()?;
        Ok(Some(message))
    }

    /// Session starts inherit the same exact execution permit as clone and plugin mutations.
    pub(super) async fn controlled_agent(
        &self,
        command: StartAgentSessionMessage,
    ) -> Result<Option<ControllerToNodeMessage>, Error> {
        let Some(permit) = self
            .execution_permit(&command.execution_id, &command.payload.spec.node_id)
            .await?
        else {
            return Ok(None);
        };
        let message = ControllerToNodeMessage::ControlledStartAgentSession(Box::new(
            ControlledStartAgentSession {
                binding: permit,
                command,
            },
        ));
        message.validate()?;
        Ok(Some(message))
    }

    /// A delivery reads the Workspace and uploads its content, so it needs the same exact permit.
    pub(super) async fn controlled_delivery(
        &self,
        command: DeliverRevisionMessage,
    ) -> Result<Option<ControllerToNodeMessage>, Error> {
        let Some(permit) = self
            .execution_permit(&command.execution_id, &command.payload.spec.node_id)
            .await?
        else {
            return Ok(None);
        };
        let message = ControllerToNodeMessage::ControlledDeliverRevision(Box::new(
            ControlledDeliverRevision {
                binding: permit,
                command,
            },
        ));
        message.validate()?;
        Ok(Some(message))
    }

    /// Registration proves historical responsibility, not current permission to start work.
    async fn execution_permit(
        &self,
        execution: &ExecutionId,
        node: &NodeId,
    ) -> Result<Option<RuntimeBinding>, Error> {
        let epoch = self.epoch()?;
        let record = self.record(execution).await?.ok_or(Error::Conflict)?;
        if record.result.is_some() {
            return Ok(None);
        }
        let Some(scope) = self
            .control_bindings(node)
            .await?
            .into_iter()
            .find(|v| v.operation_id == record.operation_id && !v.input_closed)
        else {
            return Ok(None);
        };
        let response = fault::read(async {
            RuntimeControlServiceClient::new(self.inner.channel.clone())
                .get_execution_permit(self.request(proto::GetExecutionPermitRequest {
                    epoch,
                    workspace_id: scope.workspace_id,
                    node_instance_id: scope.node_instance_id,
                    control_epoch: scope.control_epoch,
                    execution_id: execution.as_str().into(),
                }))
                .await
        })
        .await;
        let response = match response {
            Ok(p) => p,
            Err(fault::Verdict::Conflict | fault::Verdict::StaleRuntimeBinding) => return Ok(None),
            Err(v) => return Err(self.settle(v)),
        };
        Ok(Some(binding(response.binding.ok_or(Error::Conflict)?)))
    }
}
