use std::collections::BTreeMap;
use std::time::Duration;

use ora_plugin_protocol::{
    AgentConversationId, AgentEvent, AgentRequest, CancelRequestParams, DeactivateParams,
    DeactivationReason, HostRequestId, JsonRpcEnvelope, JsonRpcNotification,
    JsonRpcResponse, METHOD_CANCEL_REQUEST, METHOD_DEACTIVATE, METHOD_EXIT, METHOD_STREAM,
    PluginId, StreamParams, encode_json_rpc_notification, encode_json_rpc_request,
};
use ora_process::ProcessTreeController;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;

use crate::{
    ActorSequence, AgentContractFailure, AgentInvocationHandle, AgentInvocationResult,
    DrainTrigger, FatalSettlementCause, GenerationProcessEvent, GenerationTransport,
    HandshakeProof, PendingInvocation, PendingTransitionError, PendingWireState, PluginDeadlines,
    PluginError, PluginLimits, ProtocolFailure, ReaderEvent, SessionControlKind, StopReason,
    TerminationIntent, TerminationIntentKind, TransportFailureStage, ValidatedLaunchDescriptor,
    WriteCertainty, WriterCommandOwner, WriterCompletion, WriterLane, parse_agent_terminal,
    settle_fatal_invocation, settle_termination_intent,
};

const INVOCATION_EVENT_CAPACITY: usize = 64;
const DEFERRED_EVENT_CAPACITY: usize = 128;

pub(crate) struct QueuedInvocation {
    pub request: AgentRequest,
    pub deadline: Instant,
    pub accepted: oneshot::Sender<Result<AgentInvocationHandle, PluginError>>,
}

pub(crate) enum GenerationCommand {
    Invoke(QueuedInvocation),
    CloseAdmission(oneshot::Sender<Result<(), PluginError>>),
    Stop {
        reason: StopReason,
        completed: oneshot::Sender<Result<(), PluginError>>,
    },
    InvocationDeadline(String),
    CancelCap(String),
    EnqueueFailed {
        owner: WriterCommandOwner,
        lane: WriterLane,
    },
    DeactivateDeadline,
    ExitDeadline,
    DrainDeadline,
}

pub(crate) struct GenerationActorHandle {
    pub commands: mpsc::Sender<GenerationCommand>,
    pub completed: oneshot::Receiver<GenerationExit>,
}

#[derive(Debug)]
pub(crate) struct GenerationExit {
    pub expected: bool,
    pub exit_code: Option<i32>,
    pub cleanup_result: Result<(), PluginError>,
    pub stop_reason: Option<StopReason>,
}

struct ActiveInvocation {
    model: PendingInvocation,
    request: AgentRequest,
    deadline: Instant,
    events: mpsc::Sender<ora_plugin_protocol::AgentEvent>,
    completion: Option<oneshot::Sender<Result<AgentInvocationResult, PluginError>>>,
    conversation_id: Option<AgentConversationId>,
}

enum StopPhase {
    WaitingInvocations {
        reason: StopReason,
    },
    Deactivating {
        reason: StopReason,
        id: HostRequestId,
        frame_written: bool,
        response: Option<JsonRpcResponse>,
    },
    Exiting {
        reason: StopReason,
    },
    WaitingForDrain {
        reason: StopReason,
    },
}

/// Starts the single owner of one generation's pending table, transport, and cleanup state.
pub(crate) fn spawn_generation_actor<Controller>(
    descriptor: ValidatedLaunchDescriptor,
    transport: GenerationTransport<Controller>,
    proof: HandshakeProof,
    limits: PluginLimits,
    deadlines: PluginDeadlines,
) -> GenerationActorHandle
where
    Controller: ProcessTreeController,
{
    let (commands_tx, commands_rx) = mpsc::channel(256);
    let (cancel_tx, cancel_rx) = mpsc::channel(128);
    let (completed_tx, completed_rx) = oneshot::channel();
    let actor = GenerationActor {
        plugin_id: descriptor.plugin_id,
        generation: transport.generation,
        controller: transport.controller,
        writer: Some(transport.writer),
        writer_events: transport.writer_events,
        reader_events: transport.reader_events,
        process_events: transport.process_events,
        commands_tx: commands_tx.clone(),
        commands_rx,
        cancel_tx,
        cancel_rx,
        completed_tx: Some(completed_tx),
        limits,
        deadlines,
        next_request_sequence: proof.next_request_sequence,
        actor_sequence: 0,
        pending: BTreeMap::new(),
        admission_open: true,
        primary_trigger: None,
        stop_phase: None,
        stop_waiters: Vec::new(),
        stdout_done: false,
        stderr_done: false,
        direct_done: false,
        tree_done: false,
        exit_code: None,
        cleanup_error: None,
    };
    tokio::spawn(actor.run());
    GenerationActorHandle {
        commands: commands_tx,
        completed: completed_rx,
    }
}

struct GenerationActor<Controller> {
    plugin_id: PluginId,
    generation: u64,
    controller: Controller,
    writer: Option<crate::WriterQueues>,
    writer_events: mpsc::Receiver<WriterCompletion>,
    reader_events: mpsc::Receiver<ReaderEvent>,
    process_events: mpsc::Receiver<GenerationProcessEvent>,
    commands_tx: mpsc::Sender<GenerationCommand>,
    commands_rx: mpsc::Receiver<GenerationCommand>,
    cancel_tx: mpsc::Sender<String>,
    cancel_rx: mpsc::Receiver<String>,
    completed_tx: Option<oneshot::Sender<GenerationExit>>,
    limits: PluginLimits,
    deadlines: PluginDeadlines,
    next_request_sequence: u64,
    actor_sequence: u64,
    pending: BTreeMap<String, ActiveInvocation>,
    admission_open: bool,
    primary_trigger: Option<DrainTrigger>,
    stop_phase: Option<StopPhase>,
    stop_waiters: Vec<oneshot::Sender<Result<(), PluginError>>>,
    stdout_done: bool,
    stderr_done: bool,
    direct_done: bool,
    tree_done: bool,
    exit_code: Option<i32>,
    cleanup_error: Option<PluginError>,
}

impl<Controller> GenerationActor<Controller>
where
    Controller: ProcessTreeController,
{
    async fn run(mut self) {
        loop {
            tokio::select! {
                command = self.commands_rx.recv() => {
                    let Some(command) = command else {
                        self.begin_fatal(
                            DrainTrigger::StopEscalation,
                            FatalSettlementCause::ConnectionLost {
                                stage: TransportFailureStage::SessionDrain,
                            },
                        );
                        continue;
                    };
                    self.handle_command(command);
                }
                cancel = self.cancel_rx.recv() => {
                    if let Some(request_id) = cancel {
                        self.handle_intent(&request_id, TerminationIntentKind::ExplicitCancel);
                    }
                }
                writer = self.writer_events.recv() => {
                    if let Some(writer) = writer {
                        self.handle_writer_completion(writer);
                    }
                }
                reader = self.reader_events.recv() => {
                    match reader {
                        Some(reader) => self.handle_reader_event(reader),
                        None => self.handle_reader_event(ReaderEvent::BoundaryEof),
                    }
                }
                process = self.process_events.recv() => {
                    if let Some(process) = process {
                        self.handle_process_event(process);
                    }
                }
            }
            self.advance_stop();
            if self.should_finish() {
                self.finish_generation();
                return;
            }
        }
    }

    fn handle_command(&mut self, command: GenerationCommand) {
        match command {
            GenerationCommand::Invoke(invocation) => self.accept_invocation(invocation),
            GenerationCommand::CloseAdmission(completed) => {
                self.admission_open = false;
                let _ = completed.send(Ok(()));
            }
            GenerationCommand::Stop { reason, completed } => {
                self.admission_open = false;
                self.stop_waiters.push(completed);
                if self.stop_phase.is_none() {
                    self.stop_phase = Some(StopPhase::WaitingInvocations { reason });
                    let ids = self.pending.keys().cloned().collect::<Vec<_>>();
                    for id in ids {
                        self.handle_intent(&id, TerminationIntentKind::HostStop);
                    }
                }
            }
            GenerationCommand::InvocationDeadline(id) => {
                self.handle_intent(&id, TerminationIntentKind::HardDeadline);
            }
            GenerationCommand::CancelCap(id) => self.handle_cancel_cap(&id),
            GenerationCommand::EnqueueFailed { owner, lane } => {
                self.handle_enqueue_failure(owner, lane);
            }
            GenerationCommand::DeactivateDeadline => {
                if matches!(self.stop_phase, Some(StopPhase::Deactivating { .. })) {
                    self.begin_exit();
                }
            }
            GenerationCommand::ExitDeadline => {
                if matches!(
                    self.stop_phase,
                    Some(StopPhase::Exiting { .. } | StopPhase::WaitingForDrain { .. })
                ) && !self.direct_done
                {
                    self.begin_fatal(
                        DrainTrigger::StopEscalation,
                        FatalSettlementCause::ConnectionLost {
                            stage: TransportFailureStage::SessionDrain,
                        },
                    );
                }
            }
            GenerationCommand::DrainDeadline => {
                if !self.should_finish() {
                    let _ = self.controller.terminate_tree();
                }
            }
        }
    }

    fn accept_invocation(&mut self, invocation: QueuedInvocation) {
        let request_id = match self.allocate_request_id() {
            Ok(request_id) => request_id,
            Err(error) => {
                let _ = invocation.accepted.send(Err(error));
                self.begin_fatal(
                    DrainTrigger::StopEscalation,
                    FatalSettlementCause::ConnectionLost {
                        stage: TransportFailureStage::SessionDrain,
                    },
                );
                return;
            }
        };
        if !self.admission_open || self.primary_trigger.is_some() {
            let _ = invocation
                .accepted
                .send(Err(PluginError::PluginRuntimeUnavailable));
            return;
        }
        if Instant::now() >= invocation.deadline {
            let _ = invocation.accepted.send(Err(PluginError::RequestTimedOut {
                plugin_id: self.plugin_id.clone(),
                request_id: request_id.as_str().to_owned(),
            }));
            return;
        }
        if self.pending.len() >= self.limits.runtime.max_pending_requests as usize {
            let _ = invocation.accepted.send(Err(PluginError::PluginBusy {
                plugin_id: self.plugin_id.clone(),
                request_id: request_id.as_str().to_owned(),
            }));
            return;
        }
        if invocation
            .request
            .validate_with_limits(&self.limits.runtime)
            .is_err()
        {
            let _ = invocation
                .accepted
                .send(Err(PluginError::AgentContractViolation {
                    plugin_id: self.plugin_id.clone(),
                    request_id: request_id.as_str().to_owned(),
                    reason: AgentContractFailure::InvalidRequestDto,
                }));
            return;
        }
        let params = match invocation.request.to_params_value() {
            Ok(params) => params,
            Err(_) => {
                let _ = invocation
                    .accepted
                    .send(Err(PluginError::AgentContractViolation {
                        plugin_id: self.plugin_id.clone(),
                        request_id: request_id.as_str().to_owned(),
                        reason: AgentContractFailure::InvalidRequestDto,
                    }));
                return;
            }
        };
        let payload = match encode_json_rpc_request(
            &request_id,
            invocation.request.method().as_str(),
            &params,
        ) {
            Ok(payload) => payload,
            Err(_) => {
                let _ = invocation
                    .accepted
                    .send(Err(PluginError::AgentContractViolation {
                        plugin_id: self.plugin_id.clone(),
                        request_id: request_id.as_str().to_owned(),
                        reason: AgentContractFailure::InvalidRequestDto,
                    }));
                return;
            }
        };

        let mut model = PendingInvocation::new(request_id.clone(), invocation.request.method());
        if model.start_write().is_err() {
            let _ = invocation.accepted.send(Err(PluginError::Internal {
                message: "new invocation could not enter WriteStarted".to_owned(),
            }));
            return;
        }
        let (events_tx, events_rx) = mpsc::channel(INVOCATION_EVENT_CAPACITY);
        let (completion_tx, completion_rx) = oneshot::channel();
        let id_text = request_id.as_str().to_owned();
        let conversation_id = match &invocation.request {
            AgentRequest::SendMessage(request) => Some(request.conversation_id.clone()),
            AgentRequest::CancelConversation(request) => Some(request.conversation_id.clone()),
            AgentRequest::DiscoverInstallations(_)
            | AgentRequest::GetConfigurationSummary(_)
            | AgentRequest::ListSkills(_)
            | AgentRequest::ListMcpServers(_)
            | AgentRequest::ListConversations(_)
            | AgentRequest::StartConversation(_) => None,
        };
        self.pending.insert(
            id_text.clone(),
            ActiveInvocation {
                model,
                request: invocation.request,
                deadline: invocation.deadline,
                events: events_tx,
                completion: Some(completion_tx),
                conversation_id,
            },
        );
        let handle = AgentInvocationHandle::new(
            id_text.clone(),
            events_rx,
            completion_rx,
            self.cancel_tx.clone(),
        );
        if invocation.accepted.send(Ok(handle)).is_err() {
            self.handle_intent(&id_text, TerminationIntentKind::ExplicitCancel);
        }

        let remaining = invocation
            .deadline
            .saturating_duration_since(Instant::now());
        self.spawn_enqueue(
            WriterCommandOwner::Request(request_id),
            payload,
            WriterLane::Ordinary,
            remaining,
        );
        self.schedule_command(remaining, GenerationCommand::InvocationDeadline(id_text));
    }

    fn handle_writer_completion(&mut self, completion: WriterCompletion) {
        match completion {
            WriterCompletion::FrameWritten { generation, owner }
                if generation == self.generation =>
            {
                match owner {
                    WriterCommandOwner::Request(id) => {
                        let id = id.as_str().to_owned();
                        let replay = self
                            .pending
                            .get_mut(&id)
                            .and_then(|pending| pending.model.frame_written().ok());
                        if let Some(replay) = replay {
                            for event in replay {
                                match event.kind {
                                    crate::DeferredPendingEventKind::Terminal(response) => {
                                        self.process_terminal(&id, response);
                                    }
                                    crate::DeferredPendingEventKind::Stream(stream) => {
                                        self.process_stream(&id, stream);
                                    }
                                    crate::DeferredPendingEventKind::Intent(intent) => {
                                        self.apply_intent(&id, event.sequence, intent);
                                    }
                                }
                                if !self.pending.contains_key(&id) {
                                    break;
                                }
                            }
                        }
                    }
                    WriterCommandOwner::TransportCancel(_) => {}
                    WriterCommandOwner::SessionControl(SessionControlKind::Deactivate) => {
                        if let Some(StopPhase::Deactivating {
                            frame_written,
                            response,
                            ..
                        }) = &mut self.stop_phase
                        {
                            *frame_written = true;
                            if response.is_some() {
                                self.begin_exit();
                            }
                        }
                    }
                    WriterCommandOwner::SessionControl(SessionControlKind::Exit) => {
                        if let Some(StopPhase::Exiting { reason }) = self.stop_phase.take() {
                            self.writer.take();
                            self.stop_phase = Some(StopPhase::WaitingForDrain { reason });
                        }
                    }
                    WriterCommandOwner::SessionControl(_) => self.begin_fatal(
                        DrainTrigger::ProtocolFailure(ProtocolFailure::UnexpectedLifecycleMessage),
                        FatalSettlementCause::ConnectionLost {
                            stage: TransportFailureStage::SessionDrain,
                        },
                    ),
                }
            }
            WriterCompletion::WriteFailed {
                generation,
                owner,
                bytes_written,
                stage,
                failure,
            } if generation == self.generation => {
                if let WriterCommandOwner::Request(id) = &owner
                    && let Some(pending) = self.pending.get_mut(id.as_str())
                {
                    let _ = pending.model.write_failed(bytes_written);
                }
                self.begin_fatal(
                    DrainTrigger::WriterFailure { stage, failure },
                    FatalSettlementCause::ConnectionLost {
                        stage: match stage {
                            crate::WriterFailureStage::Request => {
                                TransportFailureStage::RequestWrite
                            }
                            crate::WriterFailureStage::TransportCancel => {
                                TransportFailureStage::TransportCancelWrite
                            }
                            crate::WriterFailureStage::SessionControl => {
                                TransportFailureStage::SessionDrain
                            }
                        },
                    },
                );
            }
            _ => {}
        }
    }

    fn handle_reader_event(&mut self, event: ReaderEvent) {
        match event {
            ReaderEvent::Envelope(JsonRpcEnvelope::Response(response)) => {
                if self.capture_deactivate_response(&response) {
                    return;
                }
                let id = response_id(&response).as_str().to_owned();
                if !self.pending.contains_key(&id) {
                    self.begin_fatal(
                        DrainTrigger::ProtocolFailure(ProtocolFailure::UnknownResponseId),
                        FatalSettlementCause::ConnectionLost {
                            stage: TransportFailureStage::ResponseRead,
                        },
                    );
                    return;
                }
                let sequence = self.next_actor_sequence();
                let deferred = self.pending.get_mut(&id).is_some_and(|pending| {
                    matches!(pending.model.wire, PendingWireState::WriteStarted { .. })
                });
                if deferred {
                    let event = crate::DeferredPendingEvent {
                        sequence,
                        kind: crate::DeferredPendingEventKind::Terminal(response),
                    };
                    if self.pending.get_mut(&id).is_some_and(|pending| {
                        pending
                            .model
                            .defer_inbound(event, DEFERRED_EVENT_CAPACITY)
                            .is_err()
                    }) {
                        self.handle_intent(&id, TerminationIntentKind::Backpressure);
                    }
                } else {
                    self.process_terminal(&id, response);
                }
            }
            ReaderEvent::Envelope(JsonRpcEnvelope::Notification(notification)) => {
                self.handle_notification(notification);
            }
            ReaderEvent::Envelope(JsonRpcEnvelope::Request(_)) => self.begin_fatal(
                DrainTrigger::ProtocolFailure(ProtocolFailure::DirectionViolation),
                FatalSettlementCause::ConnectionLost {
                    stage: TransportFailureStage::ResponseRead,
                },
            ),
            ReaderEvent::BoundaryEof => {
                self.stdout_done = true;
                if self.stop_phase.is_none() {
                    self.begin_fatal(
                        DrainTrigger::StdoutBoundaryEof,
                        FatalSettlementCause::ConnectionLost {
                            stage: TransportFailureStage::ResponseRead,
                        },
                    );
                }
            }
            ReaderEvent::Failure(failure) => {
                self.stdout_done = true;
                self.begin_fatal(
                    match failure {
                        crate::ReaderFailure::Io(failure) => {
                            DrainTrigger::StdoutReadFailure(failure)
                        }
                        crate::ReaderFailure::Protocol(failure) => {
                            DrainTrigger::ProtocolFailure(failure)
                        }
                    },
                    FatalSettlementCause::ConnectionLost {
                        stage: TransportFailureStage::ResponseRead,
                    },
                );
            }
        }
    }

    fn handle_notification(&mut self, notification: JsonRpcNotification) {
        if notification.method != METHOD_STREAM {
            self.begin_fatal(
                DrainTrigger::ProtocolFailure(ProtocolFailure::DirectionViolation),
                FatalSettlementCause::ConnectionLost {
                    stage: TransportFailureStage::ResponseRead,
                },
            );
            return;
        }
        let stream = notification
            .params
            .and_then(|params| serde_json::from_value::<StreamParams>(params).ok());
        let Some(stream) = stream else {
            self.begin_fatal(
                DrainTrigger::ProtocolFailure(ProtocolFailure::InvalidStreamSequence),
                FatalSettlementCause::ConnectionLost {
                    stage: TransportFailureStage::ResponseRead,
                },
            );
            return;
        };
        let id = stream.id.clone();
        if !self.pending.contains_key(&id) {
            self.begin_fatal(
                DrainTrigger::ProtocolFailure(ProtocolFailure::UnknownResponseId),
                FatalSettlementCause::ConnectionLost {
                    stage: TransportFailureStage::ResponseRead,
                },
            );
            return;
        }
        let sequence = self.next_actor_sequence();
        let deferred = self.pending.get_mut(&id).is_some_and(|pending| {
            matches!(pending.model.wire, PendingWireState::WriteStarted { .. })
        });
        if deferred {
            let event = crate::DeferredPendingEvent {
                sequence,
                kind: crate::DeferredPendingEventKind::Stream(stream),
            };
            if self.pending.get_mut(&id).is_some_and(|pending| {
                pending
                    .model
                    .defer_inbound(event, DEFERRED_EVENT_CAPACITY)
                    .is_err()
            }) {
                self.handle_intent(&id, TerminationIntentKind::Backpressure);
            }
        } else {
            self.process_stream(&id, stream);
        }
    }

    fn handle_process_event(&mut self, event: GenerationProcessEvent) {
        match event {
            GenerationProcessEvent::DirectExit(result) => {
                self.direct_done = true;
                match result {
                    Ok(exit) => {
                        self.exit_code = exit.exit_code;
                        if self.stop_phase.is_none() {
                            self.begin_fatal(
                                DrainTrigger::DirectProcessExit,
                                FatalSettlementCause::ProcessExited {
                                    exit_code: exit.exit_code,
                                },
                            );
                        }
                    }
                    Err(error) => {
                        self.cleanup_error = Some(PluginError::Internal {
                            message: format!("direct process watcher failed: {error}"),
                        });
                        self.begin_fatal(
                            DrainTrigger::ProcessTreeFailure(error),
                            FatalSettlementCause::ConnectionLost {
                                stage: TransportFailureStage::SessionDrain,
                            },
                        );
                    }
                }
            }
            GenerationProcessEvent::TreeEmpty(result) => {
                self.tree_done = true;
                if let Err(error) = result {
                    self.cleanup_error = Some(match error {
                        ora_process::ProcessTreeError::TreeCleanupTimeout => {
                            PluginError::TreeCleanupTimeout {
                                plugin_id: self.plugin_id.clone(),
                                generation: self.generation,
                            }
                        }
                        other => PluginError::Internal {
                            message: format!("process tree watcher failed: {other}"),
                        },
                    });
                }
            }
            GenerationProcessEvent::StderrDrained(_) => self.stderr_done = true,
        }
    }

    fn process_terminal(&mut self, id: &str, response: JsonRpcResponse) {
        let Some(mut pending) = self.pending.remove(id) else {
            return;
        };
        let intent = pending.model.termination_intent.map(|intent| intent.kind);
        let safety = pending.request.method().metadata().safety_control;
        let parsed = parse_agent_terminal(
            &self.plugin_id,
            id,
            &pending.request,
            response,
            &self.limits.runtime,
        )
        .and_then(|result| {
            if let AgentInvocationResult::Turn(turn) = &result
                && pending.conversation_id.as_ref() != Some(&turn.conversation_id)
            {
                return Err(PluginError::AgentContractViolation {
                    plugin_id: self.plugin_id.clone(),
                    request_id: id.to_owned(),
                    reason: AgentContractFailure::ConversationCorrelation,
                });
            }
            Ok(result)
        });
        let contract_violation = matches!(&parsed, Err(PluginError::AgentContractViolation { .. }));
        let result = match parsed {
            Err(_error) if safety => {
                let failure = PluginError::UnknownOutcome {
                    plugin_id: self.plugin_id.clone(),
                    request_id: id.to_owned(),
                    cause: crate::UnknownOutcomeCause::CancellationUnconfirmed,
                };
                self.begin_fatal(
                    DrainTrigger::ProtocolFailure(ProtocolFailure::InvalidEnvelope),
                    FatalSettlementCause::ConnectionLost {
                        stage: TransportFailureStage::ResponseRead,
                    },
                );
                Err(failure)
            }
            Err(error) => Err(error),
            Ok(result) if safety => Ok(result),
            Ok(_) if intent == Some(TerminationIntentKind::Backpressure) => {
                Err(PluginError::BackpressureExceeded {
                    plugin_id: self.plugin_id.clone(),
                    request_id: id.to_owned(),
                })
            }
            Ok(_)
                if intent == Some(TerminationIntentKind::HardDeadline)
                    && pending.completion.is_none() =>
            {
                self.advance_stop();
                return;
            }
            Ok(result) => Ok(result),
        };
        if contract_violation && !safety {
            self.begin_fatal(
                DrainTrigger::ProtocolFailure(ProtocolFailure::InvalidEnvelope),
                FatalSettlementCause::ConnectionLost {
                    stage: TransportFailureStage::ResponseRead,
                },
            );
        }
        if let Some(completion) = pending.completion.take() {
            let _ = completion.send(result);
        }
    }

    fn process_stream(&mut self, id: &str, stream: StreamParams) {
        if stream
            .value
            .validate_with_limits(&self.limits.runtime)
            .is_err()
        {
            self.begin_fatal(
                DrainTrigger::ProtocolFailure(ProtocolFailure::InvalidStreamSequence),
                FatalSettlementCause::ConnectionLost {
                    stage: TransportFailureStage::ResponseRead,
                },
            );
            return;
        }
        let Some(pending) = self.pending.get_mut(id) else {
            return;
        };
        if !pending.request.method().metadata().streaming
            || stream.seq.get() != pending.model.next_stream_sequence
        {
            self.begin_fatal(
                DrainTrigger::ProtocolFailure(ProtocolFailure::InvalidStreamSequence),
                FatalSettlementCause::ConnectionLost {
                    stage: TransportFailureStage::ResponseRead,
                },
            );
            return;
        }
        let correlation_violation = match (&pending.request, &stream.value) {
            (
                AgentRequest::StartConversation(_),
                AgentEvent::ConversationStarted { conversation_id },
            ) if pending.conversation_id.is_none() => {
                pending.conversation_id = Some(conversation_id.clone());
                false
            }
            (AgentRequest::StartConversation(_), AgentEvent::ConversationStarted { .. })
            | (AgentRequest::SendMessage(_), AgentEvent::ConversationStarted { .. }) => true,
            (AgentRequest::StartConversation(_), _) => pending.conversation_id.is_none(),
            (AgentRequest::SendMessage(_), _) => false,
            (
                AgentRequest::DiscoverInstallations(_)
                | AgentRequest::GetConfigurationSummary(_)
                | AgentRequest::ListSkills(_)
                | AgentRequest::ListMcpServers(_)
                | AgentRequest::ListConversations(_)
                | AgentRequest::CancelConversation(_),
                _,
            ) => true,
        };
        if correlation_violation {
            self.begin_fatal(
                DrainTrigger::ProtocolFailure(ProtocolFailure::InvalidStreamSequence),
                FatalSettlementCause::ConnectionLost {
                    stage: TransportFailureStage::ResponseRead,
                },
            );
            return;
        }
        pending.model.next_stream_sequence += 1;
        match pending.events.try_send(stream.value) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.handle_intent(id, TerminationIntentKind::Backpressure);
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.handle_intent(id, TerminationIntentKind::ExplicitCancel);
            }
        }
    }

    fn handle_intent(&mut self, id: &str, kind: TerminationIntentKind) {
        let safety = self
            .pending
            .get(id)
            .is_some_and(|pending| pending.request.method().metadata().safety_control);
        if safety && kind == TerminationIntentKind::ExplicitCancel {
            return;
        }
        let sequence = self.next_actor_sequence();
        let write_started = self.pending.get(id).is_some_and(|pending| {
            matches!(pending.model.wire, PendingWireState::WriteStarted { .. })
        });
        if write_started {
            let event = crate::DeferredPendingEvent {
                sequence,
                kind: crate::DeferredPendingEventKind::Intent(kind),
            };
            if self.pending.get_mut(id).is_some_and(|pending| {
                pending
                    .model
                    .defer_inbound(event, DEFERRED_EVENT_CAPACITY)
                    .is_err()
            }) {
                self.begin_fatal(
                    DrainTrigger::StopEscalation,
                    FatalSettlementCause::ConnectionLost {
                        stage: TransportFailureStage::SessionDrain,
                    },
                );
            }
            return;
        }
        self.apply_intent(id, sequence, kind);
    }

    fn apply_intent(&mut self, id: &str, sequence: ActorSequence, kind: TerminationIntentKind) {
        let Some(pending) = self.pending.get_mut(id) else {
            return;
        };
        pending
            .model
            .record_intent(TerminationIntent { sequence, kind });
        let Some(intent) = pending.model.termination_intent else {
            return;
        };
        if intent.sequence != sequence {
            return;
        }
        let certainty = pending.model.write_certainty();
        let safety = pending.request.method().metadata().safety_control;
        if safety && kind == TerminationIntentKind::HardDeadline {
            self.complete_safety_unknown(id);
            self.begin_fatal(
                DrainTrigger::StopEscalation,
                FatalSettlementCause::ConnectionLost {
                    stage: TransportFailureStage::SessionDrain,
                },
            );
            return;
        }
        match certainty {
            Some(WriteCertainty::NotWritten) => {
                self.complete_with_intent(id);
                self.pending.remove(id);
            }
            Some(WriteCertainty::Written | WriteCertainty::PossiblyWritten) => {
                if kind == TerminationIntentKind::HardDeadline {
                    self.complete_with_intent(id);
                }
                self.send_transport_cancel(id);
                self.schedule_cancel_cap(id);
            }
            None => {}
        }
    }

    fn handle_cancel_cap(&mut self, id: &str) {
        if !self.pending.contains_key(id) {
            return;
        }
        self.complete_with_intent(id);
        self.begin_fatal(
            DrainTrigger::StopEscalation,
            FatalSettlementCause::ConnectionLost {
                stage: TransportFailureStage::SessionDrain,
            },
        );
    }

    fn complete_with_intent(&mut self, id: &str) {
        let Some(pending) = self.pending.get_mut(id) else {
            return;
        };
        let Some(intent) = pending.model.termination_intent else {
            return;
        };
        let certainty = pending
            .model
            .write_certainty()
            .unwrap_or(WriteCertainty::PossiblyWritten);
        if let Some(completion) = pending.completion.take() {
            let _ = completion.send(Err(settle_termination_intent(
                self.plugin_id.clone(),
                id.to_owned(),
                pending.request.method().metadata().semantics,
                certainty,
                intent.kind,
            )));
        }
    }

    fn complete_safety_unknown(&mut self, id: &str) {
        if let Some(pending) = self.pending.get_mut(id)
            && let Some(completion) = pending.completion.take()
        {
            let _ = completion.send(Err(PluginError::UnknownOutcome {
                plugin_id: self.plugin_id.clone(),
                request_id: id.to_owned(),
                cause: crate::UnknownOutcomeCause::CancellationUnconfirmed,
            }));
        }
    }

    fn send_transport_cancel(&mut self, id: &str) {
        let Some(pending) = self.pending.get_mut(id) else {
            return;
        };
        let params = CancelRequestParams { id: id.to_owned() };
        let Ok(payload) = encode_json_rpc_notification(METHOD_CANCEL_REQUEST, Some(&params)) else {
            self.begin_fatal(
                DrainTrigger::StopEscalation,
                FatalSettlementCause::ConnectionLost {
                    stage: TransportFailureStage::TransportCancelWrite,
                },
            );
            return;
        };
        pending.model.wire = PendingWireState::Cancelling;
        let owner = WriterCommandOwner::TransportCancel(pending.model.id.clone());
        self.spawn_enqueue(
            owner,
            payload,
            WriterLane::TransportCancel,
            self.deadlines.transport_cancel_write,
        );
    }

    fn handle_enqueue_failure(&mut self, owner: WriterCommandOwner, lane: WriterLane) {
        if let WriterCommandOwner::Request(id) = &owner
            && let Some(mut pending) = self.pending.remove(id.as_str())
        {
            let _ = pending.model.write_failed(Some(0));
            if let Some(completion) = pending.completion.take() {
                let _ = completion.send(Err(PluginError::BackpressureExceeded {
                    plugin_id: self.plugin_id.clone(),
                    request_id: id.as_str().to_owned(),
                }));
            }
            return;
        }
        self.begin_fatal(
            DrainTrigger::StopEscalation,
            FatalSettlementCause::ConnectionLost {
                stage: match lane {
                    WriterLane::Ordinary => TransportFailureStage::RequestWrite,
                    WriterLane::TransportCancel => TransportFailureStage::TransportCancelWrite,
                    WriterLane::SessionControl => TransportFailureStage::SessionDrain,
                },
            },
        );
    }

    fn begin_fatal(&mut self, trigger: DrainTrigger, cause: FatalSettlementCause) {
        if self.primary_trigger.is_some() {
            return;
        }
        self.admission_open = false;
        self.primary_trigger = Some(trigger);
        for pending in self.pending.values_mut() {
            pending.model.latch_fatal_cause(cause);
        }
        if let Err(error) = self.controller.terminate_tree() {
            self.cleanup_error = Some(PluginError::Internal {
                message: format!("failed to terminate plugin tree: {error}"),
            });
        }
        self.schedule_command(self.deadlines.pipe_drain, GenerationCommand::DrainDeadline);
    }

    fn advance_stop(&mut self) {
        let reason = match self.stop_phase {
            Some(StopPhase::WaitingInvocations { reason }) if self.pending.is_empty() => reason,
            _ => return,
        };
        let id = match self.allocate_request_id() {
            Ok(id) => id,
            Err(_) => {
                self.begin_exit();
                return;
            }
        };
        let params = DeactivateParams {
            reason: deactivation_reason(reason),
        };
        let payload = match encode_json_rpc_request(&id, METHOD_DEACTIVATE, &params) {
            Ok(payload) => payload,
            Err(_) => {
                self.begin_exit();
                return;
            }
        };
        self.stop_phase = Some(StopPhase::Deactivating {
            reason,
            id,
            frame_written: false,
            response: None,
        });
        self.spawn_enqueue(
            WriterCommandOwner::SessionControl(SessionControlKind::Deactivate),
            payload,
            WriterLane::SessionControl,
            self.deadlines.deactivate,
        );
        self.schedule_command(
            self.deadlines.deactivate,
            GenerationCommand::DeactivateDeadline,
        );
    }

    fn capture_deactivate_response(&mut self, response: &JsonRpcResponse) -> bool {
        let matches = match &self.stop_phase {
            Some(StopPhase::Deactivating { id, .. }) => response_id(response) == id,
            _ => false,
        };
        if !matches {
            return false;
        }
        if let Some(StopPhase::Deactivating {
            frame_written,
            response: stored,
            ..
        }) = &mut self.stop_phase
        {
            *stored = Some(response.clone());
            if *frame_written {
                self.begin_exit();
            }
        }
        true
    }

    fn begin_exit(&mut self) {
        let reason = match self.stop_phase.take() {
            Some(StopPhase::WaitingInvocations { reason })
            | Some(StopPhase::Deactivating { reason, .. })
            | Some(StopPhase::Exiting { reason })
            | Some(StopPhase::WaitingForDrain { reason }) => reason,
            None => StopReason::ManualStop,
        };
        let payload = match encode_json_rpc_notification::<serde_json::Value>(METHOD_EXIT, None) {
            Ok(payload) => payload,
            Err(_) => {
                self.begin_fatal(
                    DrainTrigger::StopEscalation,
                    FatalSettlementCause::ConnectionLost {
                        stage: TransportFailureStage::SessionDrain,
                    },
                );
                return;
            }
        };
        self.stop_phase = Some(StopPhase::Exiting { reason });
        self.spawn_enqueue(
            WriterCommandOwner::SessionControl(SessionControlKind::Exit),
            payload,
            WriterLane::SessionControl,
            self.deadlines.exit,
        );
        self.schedule_command(self.deadlines.exit, GenerationCommand::ExitDeadline);
    }

    fn spawn_enqueue(
        &self,
        owner: WriterCommandOwner,
        payload: Vec<u8>,
        lane: WriterLane,
        timeout: Duration,
    ) {
        let Some(writer) = self.writer.clone() else {
            let _ = self
                .commands_tx
                .try_send(GenerationCommand::EnqueueFailed { owner, lane });
            return;
        };
        let commands = self.commands_tx.clone();
        let generation = self.generation;
        tokio::spawn(async move {
            if writer
                .enqueue(generation, owner.clone(), &payload, lane, timeout)
                .await
                .is_err()
            {
                let _ = commands
                    .send(GenerationCommand::EnqueueFailed { owner, lane })
                    .await;
            }
        });
    }

    fn schedule_cancel_cap(&self, id: &str) {
        let Some(pending) = self.pending.get(id) else {
            return;
        };
        let remaining = pending.deadline.saturating_duration_since(Instant::now());
        let delay = remaining.min(self.deadlines.transport_cancel_total);
        self.schedule_command(delay, GenerationCommand::CancelCap(id.to_owned()));
    }

    fn schedule_command(&self, delay: Duration, command: GenerationCommand) {
        let commands = self.commands_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = commands.send(command).await;
        });
    }

    fn allocate_request_id(&mut self) -> Result<HostRequestId, PluginError> {
        let sequence = self.next_request_sequence;
        let request_id =
            HostRequestId::from_sequence(sequence).map_err(|_| PluginError::Internal {
                message: "runtime request identity space is exhausted".to_owned(),
            })?;
        self.next_request_sequence =
            sequence
                .checked_add(1)
                .ok_or_else(|| PluginError::Internal {
                    message: "runtime request identity space is exhausted".to_owned(),
                })?;
        Ok(request_id)
    }

    fn next_actor_sequence(&mut self) -> ActorSequence {
        self.actor_sequence = self.actor_sequence.saturating_add(1);
        ActorSequence(self.actor_sequence)
    }

    fn should_finish(&self) -> bool {
        (self.primary_trigger.is_some()
            || matches!(self.stop_phase, Some(StopPhase::WaitingForDrain { .. })))
            && self.stdout_done
            && self.direct_done
            && self.tree_done
            && self.stderr_done
    }

    fn finish_generation(&mut self) {
        let ids = self.pending.keys().cloned().collect::<Vec<_>>();
        for id in ids {
            let Some(mut pending) = self.pending.remove(&id) else {
                continue;
            };
            if pending.request.method().metadata().safety_control {
                if let Some(completion) = pending.completion.take() {
                    let _ = completion.send(Err(PluginError::UnknownOutcome {
                        plugin_id: self.plugin_id.clone(),
                        request_id: id,
                        cause: crate::UnknownOutcomeCause::CancellationUnconfirmed,
                    }));
                }
                continue;
            }
            let certainty = pending
                .model
                .write_certainty()
                .unwrap_or(WriteCertainty::PossiblyWritten);
            let failure = if let Some(intent) = pending.model.termination_intent {
                settle_termination_intent(
                    self.plugin_id.clone(),
                    id.clone(),
                    pending.request.method().metadata().semantics,
                    certainty,
                    intent.kind,
                )
            } else {
                settle_fatal_invocation(
                    self.plugin_id.clone(),
                    id.clone(),
                    pending.request.method().metadata().semantics,
                    certainty,
                    pending
                        .model
                        .fatal_cause
                        .unwrap_or(FatalSettlementCause::ConnectionLost {
                            stage: TransportFailureStage::SessionDrain,
                        }),
                )
            };
            if let Some(completion) = pending.completion.take() {
                let _ = completion.send(Err(failure));
            }
        }

        let cleanup_result = self.cleanup_error.clone().map_or(Ok(()), Err);
        for waiter in self.stop_waiters.drain(..) {
            let _ = waiter.send(cleanup_result.clone());
        }
        if let Some(completed) = self.completed_tx.take() {
            let stop_reason = match self.stop_phase {
                Some(StopPhase::WaitingInvocations { reason })
                | Some(StopPhase::Deactivating { reason, .. })
                | Some(StopPhase::Exiting { reason })
                | Some(StopPhase::WaitingForDrain { reason }) => Some(reason),
                None => None,
            };
            let _ = completed.send(GenerationExit {
                expected: stop_reason.is_some(),
                exit_code: self.exit_code,
                cleanup_result,
                stop_reason,
            });
        }
    }
}

fn response_id(response: &JsonRpcResponse) -> &HostRequestId {
    match response {
        JsonRpcResponse::Success { id, .. } | JsonRpcResponse::Error { id, .. } => id,
    }
}

const fn deactivation_reason(reason: StopReason) -> DeactivationReason {
    match reason {
        StopReason::ManualStop => DeactivationReason::ManualStop,
        StopReason::Disable => DeactivationReason::Disable,
        StopReason::Uninstall => DeactivationReason::Uninstall,
        StopReason::Shutdown => DeactivationReason::Shutdown,
        StopReason::GrantChanged => DeactivationReason::GrantChanged,
    }
}

fn _assert_pending_error_is_bounded(_: PendingTransitionError) {}
