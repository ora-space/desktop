//! CoordinationStore adaptation delegates session, delivery, plugin and runtime-control rules to
//! their owners.
use super::{CloudStore, agents, coordinate, deliveries, fault, mapping, plugins};
use crate::*;
use ora_controller_proto::v1 as proto;
use std::io;

impl CoordinationStore for CloudStore {
    fn id(&self) -> &ControllerId {
        &self.inner.id
    }
    fn requires_runtime_control(&self) -> bool {
        true
    }
    async fn pending_plugins(&self, node: &NodeId) -> Result<Vec<PluginCommand>, Error> {
        self.pending_plugin_commands(node).await
    }
    async fn original_plugin_dispatch(
        &self,
        session: &NodeRuntimeIdentity,
        operation: &OperationId,
        execution: &ExecutionId,
    ) -> Result<Option<PluginCommand>, Error> {
        self.plugin_command(session, operation, execution).await
    }
    async fn dispatch_plugins(
        &self,
        command: PluginCommand,
    ) -> Result<Option<ControllerToNodeMessage>, Error> {
        self.controlled_plugins(command).await
    }
    async fn take_over_plugins(
        &self,
        session: &NodeRuntimeIdentity,
        event: &PluginsResultMessage,
    ) -> Result<(), Error> {
        self.plugin_event(session, event).await
    }
    async fn record_queried_plugins(
        &self,
        session: &NodeRuntimeIdentity,
        operation: &OperationId,
        execution: &ExecutionId,
        result: &PluginExecutionResult,
    ) -> Result<(), Error> {
        self.plugin_query(session, operation, execution, result)
            .await
    }

    async fn pending_agents(&self, node: &NodeId) -> Result<Vec<StartAgentSessionMessage>, Error> {
        self.agent_pending(node).await
    }
    async fn original_agent_dispatch(
        &self,
        session: &NodeRuntimeIdentity,
        operation: &OperationId,
        execution: &ExecutionId,
    ) -> Result<Option<StartAgentSessionMessage>, Error> {
        let record = self.record(execution).await?.ok_or(Error::Conflict)?;
        if record.node_id != session.node_id.as_str()
            || record.node_operation_id != operation.as_str()
        {
            return Err(Error::Conflict);
        }
        if !agents::mapping::is_agent(&record) {
            return Ok(None);
        }
        Ok(Some(agents::mapping::start(&record, &session.node_id)?))
    }
    async fn dispatch_agent(
        &self,
        command: StartAgentSessionMessage,
    ) -> Result<Option<ControllerToNodeMessage>, Error> {
        self.controlled_agent(command).await
    }
    async fn take_over_thread(
        &self,
        session: &NodeRuntimeIdentity,
        events: &[ThreadEventMessage],
    ) -> Result<(), Error> {
        self.thread_events(session, events).await
    }
    async fn take_over_agent_end(
        &self,
        session: &NodeRuntimeIdentity,
        event: &AgentSessionEndedMessage,
    ) -> Result<(), Error> {
        self.agent_end(session, event).await
    }
    async fn pending_deliveries(
        &self,
        node: &NodeId,
    ) -> Result<Vec<DeliverRevisionMessage>, Error> {
        self.delivery_pending(node).await
    }
    async fn original_delivery_dispatch(
        &self,
        session: &NodeRuntimeIdentity,
        operation: &OperationId,
        execution: &ExecutionId,
    ) -> Result<Option<DeliverRevisionMessage>, Error> {
        self.delivery_command(session, operation, execution).await
    }
    async fn dispatch_delivery(
        &self,
        command: DeliverRevisionMessage,
    ) -> Result<Option<ControllerToNodeMessage>, Error> {
        self.controlled_delivery(command).await
    }
    async fn take_over_revision(
        &self,
        session: &NodeRuntimeIdentity,
        event: &RevisionResultMessage,
    ) -> Result<(), Error> {
        self.revision_event(session, event).await
    }
    async fn grant_upload(
        &self,
        session: &NodeRuntimeIdentity,
        operation: &OperationId,
        execution: &ExecutionId,
        request: GrantRequest,
    ) -> Result<GrantOutcome, Error> {
        self.upload_grants(session, operation, execution, request)
            .await
    }
    async fn wait_agent_command_hint(&self) {
        self.inner.command_hint.notified().await;
    }
    async fn pending_agent_commands(&self, node: &NodeId) -> Result<Vec<AgentCommand>, Error> {
        self.agent_commands(node).await
    }
    async fn agent_command_delivered(&self, command: &AgentCommand) -> Result<(), Error> {
        self.command_delivered(command).await
    }

    async fn runtime_bindings(&self, node: &NodeId) -> Result<Vec<RuntimeBinding>, Error> {
        self.control_bindings(node).await
    }
    async fn acknowledge_runtime_binding(&self, state: &RuntimeControlState) -> Result<(), Error> {
        self.control_ack(state).await
    }
    async fn dispatch_message(
        &self,
        command: CloneRepositoryMessage,
    ) -> Result<Option<ControllerToNodeMessage>, Error> {
        self.controlled_dispatch(command).await
    }

    async fn take_over_node_event(
        &self,
        session: &NodeRuntimeIdentity,
        event: &CloneResultMessage,
    ) -> Result<(), Error> {
        let command = self
            .dispatched(
                session,
                &event.operation_id,
                &event.execution_id,
                &event.payload,
                Some(&event.request_id),
            )
            .await?;
        let epoch = self.epoch()?;
        let result = mapping::result(&event.payload);
        let business_operation = self
            .record(&command.execution_id)
            .await?
            .ok_or(Error::Conflict)?
            .operation_id;
        // The exact event travels verbatim so a replay with the same sequence but different
        // content is detected by the authority as a conflict, as the local receipt table does.
        let encoded = serde_json::to_vec(event)?;
        let write = fault::write(|submission_id| {
            let business_operation = business_operation.clone();
            let (result, encoded, command) = (result.clone(), encoded.clone(), &command);
            async move {
                let request = self.request(proto::TakeOverNodeEventRequest {
                    submission_id,
                    epoch,
                    operation_id: business_operation.clone(),
                    execution_id: command.execution_id.as_str().into(),
                    sequence: event.sequence.value(),
                    result: Some(result),
                    event: encoded,
                });
                self.executions().take_over_node_event(request).await
            }
        });
        write
            .await
            .map(drop)
            .map_err(|verdict| self.settle(verdict))
    }

    async fn record_queried_result(
        &self,
        session: &NodeRuntimeIdentity,
        operation: &OperationId,
        execution: &ExecutionId,
        result: &CloneExecutionResult,
    ) -> Result<(), Error> {
        let command = self
            .dispatched(session, operation, execution, result, /*request*/ None)
            .await?;
        let epoch = self.epoch()?;
        let result = mapping::result(result);
        let business_operation = self
            .record(&command.execution_id)
            .await?
            .ok_or(Error::Conflict)?
            .operation_id;
        let write = fault::write(|submission_id| {
            let business_operation = business_operation.clone();
            let (result, command) = (result.clone(), &command);
            async move {
                let request = self.request(proto::RecordQueriedResultRequest {
                    submission_id,
                    epoch,
                    operation_id: business_operation.clone(),
                    execution_id: command.execution_id.as_str().into(),
                    result: Some(result),
                });
                self.executions().record_queried_result(request).await
            }
        });
        write
            .await
            .map(drop)
            .map_err(|verdict| self.settle(verdict))
    }

    async fn original_dispatch(
        &self,
        session: &NodeRuntimeIdentity,
        operation: &OperationId,
        execution: &ExecutionId,
    ) -> Result<CloneRepositoryMessage, Error> {
        let record = self.record(execution).await?.ok_or(Error::Conflict)?;
        if record.node_operation_id != operation.as_str()
            || record.node_id != session.node_id.as_str()
        {
            return Err(Error::Conflict);
        }
        mapping::command(&record, &session.node_id)
    }

    async fn pending_dispatches(
        &self,
        node: &NodeId,
    ) -> Result<Vec<CloneRepositoryMessage>, Error> {
        let call = async {
            let request = self.request(proto::ListPendingDispatchesRequest {
                node_id: node.as_str().into(),
            });
            self.executions().list_pending_dispatches(request).await
        };
        let response = fault::read(call)
            .await
            .map_err(|verdict| self.settle(verdict))?;
        response
            .records
            .iter()
            // Every other family has its own pending list; only clones remain here.
            .filter(|record| {
                !plugins::mapping::is_plugin(record)
                    && !agents::mapping::is_agent(record)
                    && !deliveries::mapping::is_delivery(record)
            })
            .map(|record| mapping::command(record, node))
            .collect()
    }

    async fn result(&self, execution: &ExecutionId) -> Result<Option<ExecutionOutcome>, Error> {
        self.record(execution)
            .await?
            .and_then(|record| record.result)
            .map(mapping::outcome)
            .transpose()
    }

    fn static_node_established(&self, node: &NodeRuntimeIdentity) {
        if self.inner.node.as_ref() == Some(&node.node_id) {
            self.inner.node_verified.send_if_modified(|verified| {
                let first = !*verified;
                *verified = true;
                first
            });
        }
    }

    fn serve(
        &self,
        shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> impl Future<Output = io::Result<()>> + Send {
        coordinate::coordinate(self.clone(), shutdown)
    }
}
