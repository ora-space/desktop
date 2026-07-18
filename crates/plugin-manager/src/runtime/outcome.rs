use ora_plugin_protocol::{InvocationSemantics, PluginId};

use crate::PluginError;

/// Stable transport stages exposed without attacker-controlled I/O text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportFailureStage {
    RequestWrite,
    TransportCancelWrite,
    ResponseRead,
    SessionDrain,
}

/// Write-once cause captured for requests that had no earlier termination intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FatalSettlementCause {
    ConnectionLost { stage: TransportFailureStage },
    ProcessExited { exit_code: Option<i32> },
}

/// Closed ambiguity reasons returned only when a non-idempotent action may have executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnknownOutcomeCause {
    DeadlineExceeded,
    CancellationUnconfirmed,
    ConnectionLost,
    ProcessExited,
}

/// The Host's final knowledge about whether a complete request frame reached the local pipe API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteCertainty {
    NotWritten,
    Written,
    PossiblyWritten,
}

/// The first valid caller/session intent whose sequence freezes fallback classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminationIntentKind {
    ExplicitCancel,
    HostStop,
    Backpressure,
    HardDeadline,
}

/// Maps a no-intent fatal drain through the only allowed idempotency/write-certainty matrix.
pub fn settle_fatal_invocation(
    plugin_id: PluginId,
    request_id: String,
    semantics: InvocationSemantics,
    certainty: WriteCertainty,
    cause: FatalSettlementCause,
) -> PluginError {
    if semantics == InvocationSemantics::NonIdempotent && certainty != WriteCertainty::NotWritten {
        let cause = match cause {
            FatalSettlementCause::ConnectionLost { .. } => UnknownOutcomeCause::ConnectionLost,
            FatalSettlementCause::ProcessExited { .. } => UnknownOutcomeCause::ProcessExited,
        };
        return PluginError::UnknownOutcome {
            plugin_id,
            request_id,
            cause,
        };
    }

    match cause {
        FatalSettlementCause::ConnectionLost { stage } => PluginError::TransportFailed {
            plugin_id,
            request_id,
            stage,
        },
        FatalSettlementCause::ProcessExited { exit_code } => PluginError::PluginExited {
            plugin_id,
            exit_code,
        },
    }
}

/// Maps first-intent fallback without allowing a later deadline or fatal event to rewrite it.
pub fn settle_termination_intent(
    plugin_id: PluginId,
    request_id: String,
    semantics: InvocationSemantics,
    certainty: WriteCertainty,
    intent: TerminationIntentKind,
) -> PluginError {
    match intent {
        TerminationIntentKind::HardDeadline => {
            if semantics == InvocationSemantics::NonIdempotent
                && certainty != WriteCertainty::NotWritten
            {
                PluginError::UnknownOutcome {
                    plugin_id,
                    request_id,
                    cause: UnknownOutcomeCause::DeadlineExceeded,
                }
            } else {
                PluginError::RequestTimedOut {
                    plugin_id,
                    request_id,
                }
            }
        }
        TerminationIntentKind::Backpressure => {
            if semantics == InvocationSemantics::NonIdempotent
                && certainty != WriteCertainty::NotWritten
            {
                PluginError::UnknownOutcome {
                    plugin_id,
                    request_id,
                    cause: UnknownOutcomeCause::CancellationUnconfirmed,
                }
            } else {
                PluginError::BackpressureExceeded {
                    plugin_id,
                    request_id,
                }
            }
        }
        TerminationIntentKind::ExplicitCancel | TerminationIntentKind::HostStop => {
            if semantics == InvocationSemantics::NonIdempotent
                && certainty != WriteCertainty::NotWritten
            {
                PluginError::UnknownOutcome {
                    plugin_id,
                    request_id,
                    cause: UnknownOutcomeCause::CancellationUnconfirmed,
                }
            } else {
                PluginError::Cancelled {
                    plugin_id,
                    request_id,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ora_plugin_protocol::{InvocationSemantics, PluginId};
    use pretty_assertions::assert_eq;

    use super::{
        FatalSettlementCause, TerminationIntentKind, TransportFailureStage, UnknownOutcomeCause,
        WriteCertainty, settle_fatal_invocation, settle_termination_intent,
    };
    use crate::PluginError;

    fn plugin_id() -> PluginId {
        PluginId::parse("ora.runtime").unwrap_or_else(|error| panic!("test plugin id: {error}"))
    }

    /// Freezes the non-idempotent ambiguity boundary for every local write classification.
    #[test]
    fn fatal_matrix_never_replays_or_claims_a_written_non_idempotent_result() {
        let cause = FatalSettlementCause::ConnectionLost {
            stage: TransportFailureStage::ResponseRead,
        };
        assert_eq!(
            settle_fatal_invocation(
                plugin_id(),
                "h:1".to_owned(),
                InvocationSemantics::NonIdempotent,
                WriteCertainty::NotWritten,
                cause,
            ),
            PluginError::TransportFailed {
                plugin_id: plugin_id(),
                request_id: "h:1".to_owned(),
                stage: TransportFailureStage::ResponseRead,
            }
        );
        for certainty in [WriteCertainty::Written, WriteCertainty::PossiblyWritten] {
            assert_eq!(
                settle_fatal_invocation(
                    plugin_id(),
                    "h:1".to_owned(),
                    InvocationSemantics::NonIdempotent,
                    certainty,
                    cause,
                ),
                PluginError::UnknownOutcome {
                    plugin_id: plugin_id(),
                    request_id: "h:1".to_owned(),
                    cause: UnknownOutcomeCause::ConnectionLost,
                }
            );
        }
    }

    /// Ensures a cancellation accepted before the hard deadline keeps cancellation semantics.
    #[test]
    fn first_intent_controls_fallback() {
        assert_eq!(
            settle_termination_intent(
                plugin_id(),
                "h:2".to_owned(),
                InvocationSemantics::NonIdempotent,
                WriteCertainty::Written,
                TerminationIntentKind::ExplicitCancel,
            ),
            PluginError::UnknownOutcome {
                plugin_id: plugin_id(),
                request_id: "h:2".to_owned(),
                cause: UnknownOutcomeCause::CancellationUnconfirmed,
            }
        );
    }

    /// Exhausts both fatal causes across every semantics and local write-certainty combination.
    #[test]
    fn fatal_settlement_matrix_is_exhaustive() {
        let causes = [
            FatalSettlementCause::ConnectionLost {
                stage: TransportFailureStage::RequestWrite,
            },
            FatalSettlementCause::ProcessExited { exit_code: Some(9) },
        ];
        for cause in causes {
            for semantics in [
                InvocationSemantics::Idempotent,
                InvocationSemantics::NonIdempotent,
            ] {
                for certainty in [
                    WriteCertainty::NotWritten,
                    WriteCertainty::Written,
                    WriteCertainty::PossiblyWritten,
                ] {
                    let ambiguous = semantics == InvocationSemantics::NonIdempotent
                        && certainty != WriteCertainty::NotWritten;
                    let expected = match (ambiguous, cause) {
                        (true, FatalSettlementCause::ConnectionLost { .. }) => {
                            PluginError::UnknownOutcome {
                                plugin_id: plugin_id(),
                                request_id: "h:matrix".to_owned(),
                                cause: UnknownOutcomeCause::ConnectionLost,
                            }
                        }
                        (true, FatalSettlementCause::ProcessExited { .. }) => {
                            PluginError::UnknownOutcome {
                                plugin_id: plugin_id(),
                                request_id: "h:matrix".to_owned(),
                                cause: UnknownOutcomeCause::ProcessExited,
                            }
                        }
                        (false, FatalSettlementCause::ConnectionLost { stage }) => {
                            PluginError::TransportFailed {
                                plugin_id: plugin_id(),
                                request_id: "h:matrix".to_owned(),
                                stage,
                            }
                        }
                        (false, FatalSettlementCause::ProcessExited { exit_code }) => {
                            PluginError::PluginExited {
                                plugin_id: plugin_id(),
                                exit_code,
                            }
                        }
                    };
                    assert_eq!(
                        settle_fatal_invocation(
                            plugin_id(),
                            "h:matrix".to_owned(),
                            semantics,
                            certainty,
                            cause,
                        ),
                        expected
                    );
                }
            }
        }
    }

    /// Exhausts every first-intent fallback across semantics and local write certainty.
    #[test]
    fn termination_intent_matrix_is_exhaustive() {
        for intent in [
            TerminationIntentKind::ExplicitCancel,
            TerminationIntentKind::HostStop,
            TerminationIntentKind::Backpressure,
            TerminationIntentKind::HardDeadline,
        ] {
            for semantics in [
                InvocationSemantics::Idempotent,
                InvocationSemantics::NonIdempotent,
            ] {
                for certainty in [
                    WriteCertainty::NotWritten,
                    WriteCertainty::Written,
                    WriteCertainty::PossiblyWritten,
                ] {
                    let ambiguous = semantics == InvocationSemantics::NonIdempotent
                        && certainty != WriteCertainty::NotWritten;
                    let expected = if ambiguous {
                        PluginError::UnknownOutcome {
                            plugin_id: plugin_id(),
                            request_id: "h:intent".to_owned(),
                            cause: match intent {
                                TerminationIntentKind::HardDeadline => {
                                    UnknownOutcomeCause::DeadlineExceeded
                                }
                                TerminationIntentKind::ExplicitCancel
                                | TerminationIntentKind::HostStop
                                | TerminationIntentKind::Backpressure => {
                                    UnknownOutcomeCause::CancellationUnconfirmed
                                }
                            },
                        }
                    } else {
                        match intent {
                            TerminationIntentKind::HardDeadline => PluginError::RequestTimedOut {
                                plugin_id: plugin_id(),
                                request_id: "h:intent".to_owned(),
                            },
                            TerminationIntentKind::Backpressure => {
                                PluginError::BackpressureExceeded {
                                    plugin_id: plugin_id(),
                                    request_id: "h:intent".to_owned(),
                                }
                            }
                            TerminationIntentKind::ExplicitCancel
                            | TerminationIntentKind::HostStop => PluginError::Cancelled {
                                plugin_id: plugin_id(),
                                request_id: "h:intent".to_owned(),
                            },
                        }
                    };
                    assert_eq!(
                        settle_termination_intent(
                            plugin_id(),
                            "h:intent".to_owned(),
                            semantics,
                            certainty,
                            intent,
                        ),
                        expected
                    );
                }
            }
        }
    }
}
