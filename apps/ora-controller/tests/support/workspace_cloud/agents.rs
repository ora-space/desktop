//! AgentRun test authority over the real generated gRPC contract.
use super::*;
use ora_controller_proto::v1::agent_run_service_server::AgentRunService;
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(super) struct AgentState {
    pub(super) work: Vec<proto::WorkItem>,
    commands: Vec<proto::ThreadCommand>,
    delivered: HashSet<String>,
    events: HashMap<String, Vec<proto::ThreadEvent>>,
    terminal: HashMap<String, proto::TakeOverNodeEventRequest>,
    pub(super) blocked: HashSet<String>,
    fail_once: Option<tonic::Code>,
    lose_reply: bool,
    pub(super) input_closed: bool,
    pub(super) deliveries: super::deliveries::DeliveryState,
    pub(super) restores: super::restores::RestoreState,
}
impl WorkspaceCloud {
    /// Changes current permission without changing the historical registered dispatch.
    pub fn close_agent_input(&self, closed: bool) {
        self.lock().agents.input_closed = closed;
    }
    /// Queues an exact session input against the current sandbox, without creating local paths.
    pub fn queue_agent(&self, run: &str) {
        self.queue_bound_agent(run, "");
    }

    /// Freezes an opaque model reference in the same work input the real Cloud produces.
    pub fn queue_bound_agent(&self, run: &str, binding: &str) {
        self.queue_session_with_model(run, /*prior*/ None, binding);
    }
    /// Queues a session input, resuming `prior` when given, and wakes the Controller.
    pub(super) fn queue_session(&self, run: &str, prior: Option<proto::PriorRevision>) {
        self.queue_session_with_model(run, prior, "");
    }

    /// Composes the independently frozen restore and model inputs into one test-authority record.
    pub fn queue_session_with_model(
        &self,
        run: &str,
        prior: Option<proto::PriorRevision>,
        binding: &str,
    ) {
        let mut state = self.lock();
        let sandbox = state.sandboxes.last().unwrap();
        let node = state.nodes.last().unwrap().identity.as_ref().unwrap();
        let item = proto::WorkItem {
            operation_id: run.into(),
            target: Some(proto::WorkTarget {
                workspace_id: WORKSPACE.into(),
                sandbox_instance_id: sandbox.id.clone(),
                node_id: node.node_id.clone(),
            }),
            input: Some(proto::ExecutionInput {
                spec: Some(proto::execution_input::Spec::AgentSession(
                    proto::AgentSessionSpec {
                        agent_plugin_id: "official/ora-space.echo".into(),
                        agent_plugin_version: "1.2.3".into(),
                        checkout_execution_id: state.clones[0].execution_id.clone(),
                        model_binding_id: binding.into(),
                        git_identity: Some(proto::GitIdentity {
                            name: "User".into(),
                            email: "user@example.com".into(),
                        }),
                        initial_turn: Some(proto::UserTurn {
                            turn_id: "first-turn".into(),
                            content: vec![proto::ContentBlock {
                                block: Some(proto::content_block::Block::Text(
                                    proto::TextContent {
                                        text: "Issue prompt".into(),
                                    },
                                )),
                            }],
                        }),
                        prior_revision: prior,
                    },
                )),
            }),
        };
        state.agents.work.push(item);
        if let Some(subscriber) = &state.subscriber {
            let _ = subscriber.try_send(Ok(proto::WatchResponse {
                signal: Some(Signal::WorkAvailable(proto::WorkAvailable {
                    operation_id: Some(run.into()),
                })),
            }));
        }
    }
    /// Reads registered session facts for assertions and subsequent command input.
    pub fn agent_records(&self) -> Vec<proto::ExecutionRecord> {
        self.lock()
            .clones
            .iter()
            .filter(|r| {
                matches!(
                    r.input.as_ref().and_then(|i| i.spec.as_ref()),
                    Some(proto::execution_input::Spec::AgentSession(_))
                )
            })
            .cloned()
            .collect()
    }
    /// Leaves a slow execution blocked while unrelated executions remain available.
    pub fn block_agent(&self, run: &str, blocked: bool) {
        let mut state = self.lock();
        if blocked {
            state.agents.blocked.insert(run.into());
        } else {
            state.agents.blocked.remove(run);
        }
    }
    /// Injects one definitive failure or a lost response after committing a batch.
    pub fn fail_agent_batch(&self, code: tonic::Code) {
        self.lock().agents.fail_once = Some(code);
    }
    /// Commits the next batch but loses its response to exercise submission reuse.
    pub fn lose_agent_reply(&self) {
        self.lock().agents.lose_reply = true;
    }
    /// Returns the exact durable records for the requested execution.
    pub fn thread_events(&self, execution: &str) -> Vec<proto::ThreadEvent> {
        self.lock()
            .agents
            .events
            .get(execution)
            .cloned()
            .unwrap_or_default()
    }
    /// Exposes only commands confirmed by the Controller through the delivery RPC.
    pub fn delivered_commands(&self) -> HashSet<String> {
        self.lock().agents.delivered.clone()
    }
    /// Preserves command creation order and wakes the Controller through the real signal stream.
    pub fn queue_agent_command(&self, run: &str, id: &str, end: bool) {
        let mut state = self.lock();
        let record = state.clones.iter().find(|r| r.operation_id == run).unwrap();
        let target = state
            .agents
            .work
            .iter()
            .find(|w| w.operation_id == run)
            .unwrap()
            .target
            .clone();
        let command = proto::ThreadCommand {
            command_id: id.into(),
            run_id: run.into(),
            execution_id: record.execution_id.clone(),
            target,
            command: Some(if end {
                proto::thread_command::Command::EndSession(proto::EndSession {
                    reason: proto::EndSessionReason::UserEnded as i32,
                })
            } else {
                proto::thread_command::Command::SubmitUserTurn(proto::SubmitUserTurn {
                    turn: Some(proto::UserTurn {
                        turn_id: id.into(),
                        content: vec![proto::ContentBlock {
                            block: Some(proto::content_block::Block::Text(proto::TextContent {
                                text: format!("message {id}"),
                            })),
                        }],
                    }),
                })
            }),
        };
        state.agents.commands.push(command);
        if let Some(subscriber) = &state.subscriber {
            let _ = subscriber.try_send(Ok(proto::WatchResponse {
                signal: Some(Signal::ThreadCommandAvailable(
                    proto::ThreadCommandAvailable { run_id: run.into() },
                )),
            }));
        }
    }
    /// Returns the first unregistered work item without consuming it. A run's session and its
    /// later delivery are distinct items, told apart by their frozen input.
    pub(super) fn agent_work(&self) -> Option<proto::WorkItem> {
        let state = self.lock();
        state
            .agents
            .work
            .iter()
            .find(|w| {
                !state
                    .clones
                    .iter()
                    .any(|r| r.operation_id == w.operation_id && r.input == w.input)
            })
            .cloned()
    }
    /// Enforces exact Cloud input and target before registering a session.
    pub(super) fn register_agent(
        &self,
        message: &proto::RecordDispatchRequest,
    ) -> Result<Response<proto::RecordDispatchResponse>, Status> {
        let mut state = self.lock();
        let work = state
            .agents
            .work
            .iter()
            .find(|w| w.operation_id == message.operation_id && w.input == message.input)
            .ok_or_else(|| conflict("missing_work"))?;
        if message.epoch != EPOCH
            || work.input != message.input
            || work.target.as_ref().unwrap().node_id != message.node_id
        {
            return Err(conflict("wrong_input"));
        }
        let record = proto::ExecutionRecord {
            operation_id: message.operation_id.clone(),
            execution_id: message.execution_id.clone(),
            node_operation_id: format!("node-{}", message.execution_id),
            node_id: message.node_id.clone(),
            input: message.input.clone(),
            result: None,
        };
        if !state
            .clones
            .iter()
            .any(|r| r.execution_id == record.execution_id)
        {
            state.clones.push(record.clone());
        }
        drop(state);
        let run = message.operation_id.clone();
        self.timeline.push(
            if matches!(
                message.input.as_ref().and_then(|i| i.spec.as_ref()),
                Some(proto::execution_input::Spec::DeliverRevision(_))
            ) {
                Event::DeliveryRegistered { run }
            } else {
                Event::AgentRegistered { run }
            },
        );
        Ok(Response::new(proto::RecordDispatchResponse {
            record: Some(record),
        }))
    }
    /// A queried terminal cannot substitute for the sequenced terminal event.
    pub(super) fn end_agent(
        &self,
        message: proto::TakeOverNodeEventRequest,
    ) -> Result<Response<proto::TakeOverNodeEventResponse>, Status> {
        let mut state = self.lock();
        let count = state
            .agents
            .events
            .get(&message.execution_id)
            .map_or(0, Vec::len) as u64;
        if message.sequence != count + 1 {
            return Err(conflict("terminal_gap"));
        }
        if let Some(old) = state.agents.terminal.get(&message.execution_id)
            && (old.event != message.event || old.result != message.result)
        {
            return Err(conflict("terminal_conflict"));
        }
        state
            .agents
            .terminal
            .insert(message.execution_id.clone(), message.clone());
        drop(state);
        let record = self.store_result(&message.execution_id, message.result)?;
        self.timeline.push(Event::AgentEnded {
            run: message.operation_id,
        });
        Ok(Response::new(proto::TakeOverNodeEventResponse {
            record: Some(record),
        }))
    }
}
#[tonic::async_trait]
impl AgentRunService for WorkspaceCloud {
    async fn take_over_thread_events(
        &self,
        request: Request<proto::TakeOverThreadEventsRequest>,
    ) -> Result<Response<proto::TakeOverThreadEventsResponse>, Status> {
        let message = request.into_inner();
        self.timeline.push(Event::AgentBatchAttempt {
            run: message.operation_id.clone(),
            submission: message.submission_id.clone(),
        });
        loop {
            let blocked = self.lock().agents.blocked.contains(&message.operation_id);
            if !blocked {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let mut state = self.lock();
        if let Some(code) = state.agents.fail_once.take() {
            return Err(Status::new(code, "injected"));
        }
        if message.epoch != EPOCH || message.events.is_empty() || message.events.len() > 64 {
            return Err(conflict("invalid_batch"));
        }
        if !state.clones.iter().any(|r| {
            r.execution_id == message.execution_id && r.operation_id == message.operation_id
        }) {
            return Err(conflict("unknown_execution"));
        }
        let mut events = state
            .agents
            .events
            .get(&message.execution_id)
            .cloned()
            .unwrap_or_default();
        for event in &message.events {
            let index = event
                .sequence
                .checked_sub(1)
                .ok_or_else(|| conflict("zero_sequence"))? as usize;
            if index < events.len() {
                if events[index] != *event {
                    return Err(conflict("different_replay"));
                }
            } else if index == events.len() {
                events.push(event.clone());
            } else {
                return Err(conflict("gap"));
            }
        }
        let through = events.len() as u64;
        state
            .agents
            .events
            .insert(message.execution_id.clone(), events);
        let lose = std::mem::take(&mut state.agents.lose_reply);
        drop(state);
        self.timeline.push(Event::ThreadTaken {
            run: message.operation_id,
            through,
        });
        if lose {
            return Err(Status::deadline_exceeded("lost committed reply"));
        }
        Ok(Response::new(proto::TakeOverThreadEventsResponse {
            taken_over_through: through,
        }))
    }
    async fn claim_thread_commands(
        &self,
        _: Request<proto::ClaimThreadCommandsRequest>,
    ) -> Result<Response<proto::ClaimThreadCommandsResponse>, Status> {
        let state = self.lock();
        Ok(Response::new(proto::ClaimThreadCommandsResponse {
            commands: state
                .agents
                .commands
                .iter()
                .filter(|c| !state.agents.delivered.contains(&c.command_id))
                .cloned()
                .collect(),
        }))
    }
    async fn record_thread_command_delivered(
        &self,
        request: Request<proto::RecordThreadCommandDeliveredRequest>,
    ) -> Result<Response<proto::RecordThreadCommandDeliveredResponse>, Status> {
        let m = request.into_inner();
        let mut state = self.lock();
        if !state
            .agents
            .commands
            .iter()
            .any(|c| c.command_id == m.command_id && c.execution_id == m.execution_id)
        {
            return Err(conflict("unknown_command"));
        }
        state.agents.delivered.insert(m.command_id.clone());
        drop(state);
        self.timeline
            .push(Event::CommandDelivered { id: m.command_id });
        Ok(Response::new(
            proto::RecordThreadCommandDeliveredResponse {},
        ))
    }
    async fn grant_revision_upload(
        &self,
        request: Request<proto::GrantRevisionUploadRequest>,
    ) -> Result<Response<proto::GrantRevisionUploadResponse>, Status> {
        self.grant(request.into_inner())
    }
    /// Signs a read of a resumed session's prior bundle.
    async fn grant_revision_download(
        &self,
        request: Request<proto::GrantRevisionDownloadRequest>,
    ) -> Result<Response<proto::GrantRevisionDownloadResponse>, Status> {
        self.grant_download(request.into_inner())
    }
}
