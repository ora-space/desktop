//! Cloud remains the only durable authority for session dispatches, Thread receipts and commands.
pub(super) mod mapping;
use super::{CloudStore, fault};
use crate::*;
use ora_controller_proto::v1::{self as proto, agent_run_service_client::AgentRunServiceClient};
use tonic::transport::Channel;

impl CloudStore {
    /// Uses the existing management channel and holder metadata for the AgentRun contract.
    fn agents(&self) -> AgentRunServiceClient<Channel> {
        AgentRunServiceClient::new(self.inner.channel.clone())
    }

    /// Resolves original session ownership even after Cloud already recorded its terminal result.
    pub(super) async fn agent_record(
        &self,
        session: &NodeRuntimeIdentity,
        operation: &OperationId,
        execution: &ExecutionId,
    ) -> Result<proto::ExecutionRecord, Error> {
        let record = self.record(execution).await?.ok_or(Error::Conflict)?;
        if record.node_id != session.node_id.as_str()
            || record.node_operation_id != operation.as_str()
            || !mapping::is_agent(&record)
        {
            return Err(Error::Conflict);
        }
        mapping::start(&record, &session.node_id)?;
        Ok(record)
    }

    /// Lists only the session family; clone and plugin polling keep their existing ownership.
    pub(super) async fn agent_pending(
        &self,
        node: &NodeId,
    ) -> Result<Vec<StartAgentSessionMessage>, Error> {
        let response = fault::read(async {
            self.executions()
                .list_pending_dispatches(self.request(proto::ListPendingDispatchesRequest {
                    node_id: node.as_str().into(),
                }))
                .await
        })
        .await
        .map_err(|v| self.settle(v))?;
        response
            .records
            .iter()
            .filter(|r| mapping::is_agent(r))
            .map(|r| mapping::start(r, node))
            .collect()
    }

    /// Commits exact ordered Thread records, refusing foreign envelopes before calling Cloud.
    pub(super) async fn thread_events(
        &self,
        session: &NodeRuntimeIdentity,
        events: &[ThreadEventMessage],
    ) -> Result<(), Error> {
        let first = events.first().ok_or(Error::Conflict)?;
        if events.len() > 64 {
            return Err(Error::Conflict);
        }
        let record = self
            .agent_record(session, &first.operation_id, &first.execution_id)
            .await?;
        let mut previous = None;
        let events = events
            .iter()
            .map(|event| {
                event.validate()?;
                if event.operation_id != first.operation_id
                    || event.execution_id != first.execution_id
                    || previous
                        .is_some_and(|n: u64| n.checked_add(1) != Some(event.sequence.value()))
                {
                    return Err(Error::Conflict);
                }
                previous = Some(event.sequence.value());
                Ok(proto::ThreadEvent {
                    sequence: event.sequence.value(),
                    turn_id: event.payload.turn_id.as_ref().map(|v| v.as_str().into()),
                    record: serde_json::to_string(&event.payload.record)?,
                    truncated: event.payload.truncated,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let through = events.last().ok_or(Error::Conflict)?.sequence;
        let epoch = self.epoch()?;
        let response = fault::write(|submission_id| {
            let request = proto::TakeOverThreadEventsRequest {
                submission_id,
                epoch,
                operation_id: record.operation_id.clone(),
                execution_id: record.execution_id.clone(),
                events: events.clone(),
            };
            async move {
                self.agents()
                    .take_over_thread_events(self.request(request))
                    .await
            }
        })
        .await
        .map_err(|v| self.settle(v))?;
        if response.taken_over_through < through {
            return Err(Error::Conflict);
        }
        Ok(())
    }

    /// Takes over a real terminal envelope; query results deliberately cannot bypass Thread replay.
    pub(super) async fn agent_end(
        &self,
        session: &NodeRuntimeIdentity,
        event: &AgentSessionEndedMessage,
    ) -> Result<(), Error> {
        event.validate()?;
        let AgentSessionResult::AgentSessionEnded(ended) = &event.payload;
        if ended.node.node_id != session.node_id {
            return Err(Error::Conflict);
        }
        let record = self
            .agent_record(session, &event.operation_id, &event.execution_id)
            .await?;
        let epoch = self.epoch()?;
        let encoded = serde_json::to_vec(event)?;
        fault::write(|submission_id| {
            let request = proto::TakeOverNodeEventRequest {
                submission_id,
                epoch,
                operation_id: record.operation_id.clone(),
                execution_id: record.execution_id.clone(),
                sequence: event.sequence.value(),
                result: Some(mapping::result(&event.payload)),
                event: encoded.clone(),
            };
            async move {
                self.executions()
                    .take_over_node_event(self.request(request))
                    .await
            }
        })
        .await
        .map(drop)
        .map_err(|v| self.settle(v))
    }

    /// Reads Cloud's ordered queue; registration and current runtime scope prove the destination.
    pub(super) async fn agent_commands(&self, node: &NodeId) -> Result<Vec<AgentCommand>, Error> {
        let epoch = self.epoch()?;
        let response = fault::read(async {
            self.agents()
                .claim_thread_commands(
                    self.request(proto::ClaimThreadCommandsRequest { epoch, limit: 100 }),
                )
                .await
        })
        .await
        .map_err(|v| self.settle(v))?;
        let bindings = self.control_bindings(node).await?;
        let mut commands = Vec::new();
        // A skipped head also blocks later commands of that run in this response.
        let mut seen = std::collections::HashSet::new();
        for command in response.commands {
            if !seen.insert(command.run_id.clone()) {
                continue;
            }
            let Some(target) = &command.target else {
                return Err(Error::Conflict);
            };
            if target.node_id != node.as_str() {
                continue;
            }
            let Some(scope) = bindings.iter().find(|b| {
                b.operation_id == command.run_id
                    && b.workspace_id == target.workspace_id
                    && b.sandbox_id == target.sandbox_instance_id
                    && !b.input_closed
            }) else {
                continue;
            };
            if scope.controller_epoch != epoch {
                continue;
            }
            let Some(record) = self
                .record(&ExecutionId::new(command.execution_id.clone()))
                .await?
            else {
                continue;
            };
            commands.push(mapping::command(&command, &record, node)?);
        }
        Ok(commands)
    }

    /// A reply acknowledges receipt, including session_ended; Cloud settles discarded turns itself.
    pub(super) async fn command_delivered(&self, command: &AgentCommand) -> Result<(), Error> {
        let epoch = self.epoch()?;
        fault::write(|submission_id| {
            let request = proto::RecordThreadCommandDeliveredRequest {
                submission_id,
                epoch,
                command_id: command.id().as_str().into(),
                execution_id: command.execution().as_str().into(),
            };
            async move {
                self.agents()
                    .record_thread_command_delivered(self.request(request))
                    .await
            }
        })
        .await
        .map(drop)
        .map_err(|v| self.settle(v))
    }

    /// Freezes the Cloud input before any Node frame; a lost registration reply reuses its UUID.
    pub(super) async fn record_agent(
        &self,
        epoch: i64,
        item: &proto::WorkItem,
        node: &NodeId,
    ) -> Result<(), Error> {
        let execution = uuid::Uuid::new_v4().to_string();
        let preview = proto::ExecutionRecord {
            operation_id: item.operation_id.clone(),
            node_operation_id: item.operation_id.clone(),
            execution_id: execution.clone(),
            node_id: node.as_str().into(),
            input: item.input.clone(),
            result: None,
        };
        mapping::start(&preview, node)?;
        let response = fault::write(|submission_id| {
            let request = proto::RecordDispatchRequest {
                submission_id,
                epoch,
                operation_id: item.operation_id.clone(),
                execution_id: execution.clone(),
                node_id: node.as_str().into(),
                input: item.input.clone(),
            };
            async move {
                self.executions()
                    .record_dispatch(self.request(request))
                    .await
            }
        })
        .await
        .map_err(|v| self.settle(v))?;
        let record = response.record.ok_or(Error::Conflict)?;
        if record.operation_id != item.operation_id
            || record.execution_id != execution
            || record.input != item.input
        {
            return Err(Error::Conflict);
        }
        mapping::start(&record, node)?;
        Ok(())
    }
}
