use ora_plugin_protocol::{AgentMethod, HostRequestId, JsonRpcResponse, StreamParams};

use super::{FatalSettlementCause, TerminationIntentKind, WriteCertainty};

/// Monotonic order assigned only by the runtime actor when it accepts an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ActorSequence(pub u64);

/// The first termination intent and the actor turn that froze its fallback meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminationIntent {
    pub sequence: ActorSequence,
    pub kind: TerminationIntentKind,
}

/// Events that arrived after write start but before local full-frame acknowledgement.
#[derive(Debug, Clone, PartialEq)]
pub struct DeferredPendingEvent {
    pub sequence: ActorSequence,
    pub kind: DeferredPendingEventKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DeferredPendingEventKind {
    Terminal(JsonRpcResponse),
    Stream(StreamParams),
    Intent(TerminationIntentKind),
}

/// Causal wire state owned exclusively by the actor, never duplicated in the writer worker.
#[derive(Debug, Clone, PartialEq)]
pub enum PendingWireState {
    Queued,
    WriteStarted {
        deferred_events: Vec<DeferredPendingEvent>,
    },
    Written,
    Cancelling,
    WriteFailed {
        certainty: WriteCertainty,
    },
}

/// One pending request's immutable method identity and write-once settlement facts.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingInvocation {
    pub id: HostRequestId,
    pub method: AgentMethod,
    pub wire: PendingWireState,
    pub termination_intent: Option<TerminationIntent>,
    pub fatal_cause: Option<FatalSettlementCause>,
    pub next_stream_sequence: u64,
}

impl PendingInvocation {
    pub fn new(id: HostRequestId, method: AgentMethod) -> Self {
        Self {
            id,
            method,
            wire: PendingWireState::Queued,
            termination_intent: None,
            fatal_cause: None,
            next_stream_sequence: 1,
        }
    }

    /// Atomically transfers scheduler ownership to the single writer command.
    pub fn start_write(&mut self) -> Result<(), PendingTransitionError> {
        if self.wire != PendingWireState::Queued {
            return Err(PendingTransitionError::WriteAlreadyStarted);
        }
        self.wire = PendingWireState::WriteStarted {
            deferred_events: Vec::new(),
        };
        Ok(())
    }

    /// Defers correlated inbound data until the complete request frame is acknowledged locally.
    pub fn defer_inbound(
        &mut self,
        event: DeferredPendingEvent,
        maximum_events: usize,
    ) -> Result<(), PendingTransitionError> {
        match &mut self.wire {
            PendingWireState::WriteStarted { deferred_events } => {
                if deferred_events.len() >= maximum_events {
                    return Err(PendingTransitionError::DeferredBudgetExceeded);
                }
                deferred_events.push(event);
                Ok(())
            }
            PendingWireState::Written | PendingWireState::Cancelling => {
                Err(PendingTransitionError::NoDeferralRequired)
            }
            PendingWireState::Queued | PendingWireState::WriteFailed { .. } => {
                Err(PendingTransitionError::InboundBeforeWriteStart)
            }
        }
    }

    /// Opens the causal gate and returns deferred events in their original actor order.
    pub fn frame_written(&mut self) -> Result<Vec<DeferredPendingEvent>, PendingTransitionError> {
        let PendingWireState::WriteStarted { deferred_events } = &mut self.wire else {
            return Err(PendingTransitionError::WriterAckWithoutWrite);
        };
        deferred_events.sort_by_key(|event| event.sequence);
        let replay = std::mem::take(deferred_events);
        self.wire = PendingWireState::Written;
        Ok(replay)
    }

    /// Rejects deferred inbound evidence and records only zero-versus-possible write knowledge.
    pub fn write_failed(
        &mut self,
        bytes_written: Option<usize>,
    ) -> Result<WriteCertainty, PendingTransitionError> {
        if !matches!(self.wire, PendingWireState::WriteStarted { .. }) {
            return Err(PendingTransitionError::WriterAckWithoutWrite);
        }
        let certainty = match bytes_written {
            Some(0) => WriteCertainty::NotWritten,
            Some(_) | None => WriteCertainty::PossiblyWritten,
        };
        self.wire = PendingWireState::WriteFailed { certainty };
        Ok(certainty)
    }

    /// Stores only the earliest valid intent; later cancel/deadline events cannot rewrite it.
    pub fn record_intent(&mut self, intent: TerminationIntent) {
        if self
            .termination_intent
            .is_none_or(|current| intent.sequence < current.sequence)
        {
            self.termination_intent = Some(intent);
        }
    }

    /// Latches a fatal cause only for a no-intent request and never replaces prior evidence.
    pub fn latch_fatal_cause(&mut self, cause: FatalSettlementCause) {
        if self.termination_intent.is_none() && self.fatal_cause.is_none() {
            self.fatal_cause = Some(cause);
        }
    }

    /// Returns current local write certainty without pretending WriteStarted has completed.
    pub const fn write_certainty(&self) -> Option<WriteCertainty> {
        match self.wire {
            PendingWireState::Queued => Some(WriteCertainty::NotWritten),
            PendingWireState::Written | PendingWireState::Cancelling => {
                Some(WriteCertainty::Written)
            }
            PendingWireState::WriteFailed { certainty } => Some(certainty),
            PendingWireState::WriteStarted { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PendingTransitionError {
    #[error("request write already started")]
    WriteAlreadyStarted,
    #[error("inbound event arrived before request write start")]
    InboundBeforeWriteStart,
    #[error("request is already written and does not require deferral")]
    NoDeferralRequired,
    #[error("deferred event budget exceeded")]
    DeferredBudgetExceeded,
    #[error("writer completion did not match a write-started request")]
    WriterAckWithoutWrite,
}

#[cfg(test)]
mod tests {
    use ora_plugin_protocol::{
        AgentMethod, HostRequestId, JsonRpcResponse, JsonSafeU64, StreamParams,
    };
    use pretty_assertions::assert_eq;

    use super::{
        ActorSequence, DeferredPendingEvent, DeferredPendingEventKind, PendingInvocation,
        PendingWireState, TerminationIntent,
    };
    use crate::{TerminationIntentKind, WriteCertainty};

    fn pending() -> PendingInvocation {
        PendingInvocation::new(
            HostRequestId::from_sequence(7)
                .unwrap_or_else(|error| panic!("test request id: {error}")),
            AgentMethod::StartConversation,
        )
    }

    /// Replays response-before-ack and deadline-before-response by actor sequence, not task order.
    #[test]
    fn response_before_writer_ack_preserves_actor_order() {
        let mut pending = pending();
        pending
            .start_write()
            .unwrap_or_else(|error| panic!("start write: {error}"));
        pending
            .defer_inbound(
                DeferredPendingEvent {
                    sequence: ActorSequence(3),
                    kind: DeferredPendingEventKind::Intent(TerminationIntentKind::HardDeadline),
                },
                8,
            )
            .unwrap_or_else(|error| panic!("defer deadline: {error}"));
        pending
            .defer_inbound(
                DeferredPendingEvent {
                    sequence: ActorSequence(2),
                    kind: DeferredPendingEventKind::Terminal(JsonRpcResponse::Success {
                        id: HostRequestId::from_sequence(7)
                            .unwrap_or_else(|error| panic!("test response id: {error}")),
                        result: serde_json::json!({"conversationId":"c","finishReason":"completed"}),
                    }),
                },
                8,
            )
            .unwrap_or_else(|error| panic!("defer response: {error}"));

        let replay = pending
            .frame_written()
            .unwrap_or_else(|error| panic!("writer ack: {error}"));
        assert_eq!(
            replay
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            vec![ActorSequence(2), ActorSequence(3)]
        );
        assert_eq!(pending.wire, PendingWireState::Written);
    }

    /// Ensures a partial write discards deferred peer evidence and becomes PossiblyWritten.
    #[test]
    fn partial_write_never_adopts_deferred_stream() {
        let mut pending = pending();
        pending
            .start_write()
            .unwrap_or_else(|error| panic!("start write: {error}"));
        pending
            .defer_inbound(
                DeferredPendingEvent {
                    sequence: ActorSequence(1),
                    kind: DeferredPendingEventKind::Stream(StreamParams {
                        id: "h:7".to_owned(),
                        seq: JsonSafeU64::new(1)
                            .unwrap_or_else(|error| panic!("test sequence: {error}")),
                        value: ora_plugin_protocol::AgentEvent::Status {
                            phase: "working".to_owned(),
                            message: None,
                        },
                    }),
                },
                8,
            )
            .unwrap_or_else(|error| panic!("defer stream: {error}"));
        assert_eq!(
            pending
                .write_failed(Some(3))
                .unwrap_or_else(|error| panic!("write failure: {error}")),
            WriteCertainty::PossiblyWritten
        );
        assert_eq!(
            pending.wire,
            PendingWireState::WriteFailed {
                certainty: WriteCertainty::PossiblyWritten
            }
        );
    }

    /// Keeps the earliest intent and refuses to latch a later fatal cause over it.
    #[test]
    fn intent_and_fatal_causes_are_write_once() {
        let mut pending = pending();
        pending.record_intent(TerminationIntent {
            sequence: ActorSequence(2),
            kind: TerminationIntentKind::ExplicitCancel,
        });
        pending.record_intent(TerminationIntent {
            sequence: ActorSequence(4),
            kind: TerminationIntentKind::HardDeadline,
        });
        pending
            .latch_fatal_cause(crate::FatalSettlementCause::ProcessExited { exit_code: Some(1) });
        assert_eq!(
            pending.termination_intent,
            Some(TerminationIntent {
                sequence: ActorSequence(2),
                kind: TerminationIntentKind::ExplicitCancel,
            })
        );
        assert_eq!(pending.fatal_cause, None);
    }

    /// Freezes zero, partial, and unknown writer failures into the only three causal outcomes.
    #[test]
    fn write_failure_certainty_matrix_is_exhaustive() {
        let actual = [Some(0), Some(1), None]
            .into_iter()
            .map(|bytes_written| {
                let mut pending = pending();
                pending
                    .start_write()
                    .unwrap_or_else(|error| panic!("start write: {error}"));
                let certainty = pending
                    .write_failed(bytes_written)
                    .unwrap_or_else(|error| panic!("write failure: {error}"));
                (certainty, pending.wire)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            actual,
            vec![
                (
                    WriteCertainty::NotWritten,
                    PendingWireState::WriteFailed {
                        certainty: WriteCertainty::NotWritten,
                    },
                ),
                (
                    WriteCertainty::PossiblyWritten,
                    PendingWireState::WriteFailed {
                        certainty: WriteCertainty::PossiblyWritten,
                    },
                ),
                (
                    WriteCertainty::PossiblyWritten,
                    PendingWireState::WriteFailed {
                        certainty: WriteCertainty::PossiblyWritten,
                    },
                ),
            ]
        );
    }
}
