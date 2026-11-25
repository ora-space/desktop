use ora_contracts::{EmptyErrorParams, PublicError};
use ora_history::TurnFailure;
use std::error::Error;
use std::fmt;
use std::sync::Arc;

/// Shared, clonable source chain retained behind a runtime failure.
pub type SharedError = Arc<dyn Error + Send + Sync + 'static>;

/// Classifies runtime failures so a host can map them onto its own status and logging semantics.
///
/// The variants mirror the classifications a host adapter already distinguishes. A host error that
/// crosses into the runtime (for example a Workspace that cannot be resolved) has to come back out
/// with the class it went in with, so the set is kept lossless rather than narrowed to the classes
/// the runtime produces itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorClassification {
    InvalidRequest,
    PayloadTooLarge,
    NotFound,
    /// Access was refused by the host rather than by the runtime's own request validation.
    Forbidden,
    /// The host environment could not carry out an action the runtime delegated to it.
    HostUnavailable,
    Conflict,
    Unprocessable,
    Internal,
}

/// Preserves internal diagnostics while exposing only a typed public error to the host.
///
/// `Display` is the semantic context alone; the lower-level cause stays reachable through
/// [`Error::source`] so hosts keep the complete chain in their logs without leaking it publicly.
#[derive(Clone, Debug)]
pub struct RuntimeError {
    classification: ErrorClassification,
    public_error: PublicError,
    context: String,
    source: Option<SharedError>,
    /// The Agent answered the prompt (with an error, or after a watchdog cancel) before this
    /// failure, so its main flow reached the response fence.
    answered: bool,
    /// The bounded reason this turn failed, recorded on its `TurnEnded`. Set only when the runtime
    /// observed a failure first; turns that were cancelled or ended by the session carry none.
    turn_failure: Option<TurnFailure>,
}

impl RuntimeError {
    /// A target changed during replacement; a new explicit attempt must own any next process.
    pub(crate) fn mcp_retry_required() -> Self {
        Self::internal(
            "MCP target changed during process replacement",
            std::io::Error::other("explicit retry required"),
        )
        .during_mcp()
    }
    /// Marks a failed MCP delivery boundary without reclassifying ordinary Agent failures.
    pub(crate) fn during_mcp(self) -> Self {
        Self::with_source(
            self.classification,
            self.public_error.clone(),
            "MCP admission failed",
            McpAdmissionFailure(self),
        )
    }

    /// A turn cancel whose Agent did not answer the prompt by the caller's deadline; the provider
    /// session was isolated and the turn's work may still be running.
    pub(crate) fn turn_cancel_unanswered() -> Self {
        Self::with_source(
            ErrorClassification::Unprocessable,
            PublicError::AgentTimedOut(ora_contracts::EmptyErrorParams {}),
            "agent did not answer the cancelled prompt before its deadline",
            TurnCancelUnanswered,
        )
    }

    /// Distinguishes an unanswered turn cancel from other failures of the cancelled prompt.
    pub fn is_turn_cancel_unanswered(&self) -> bool {
        self.source
            .as_deref()
            .is_some_and(<dyn Error + Send + Sync + 'static>::is::<TurnCancelUnanswered>)
    }

    /// Marks a prompt failure the Agent itself answered: an ACP error response, or a watchdog
    /// stop whose cancel the Agent confirmed before the runtime gave up.
    pub(crate) fn answered(mut self) -> Self {
        self.answered = true;
        self
    }

    /// Whether the Agent answered the failed prompt; only such a failure can leave the provider
    /// session in a state another turn may follow (cancel-turn D2).
    pub fn agent_answered(&self) -> bool {
        self.answered
    }

    /// Hosts can wait for MCP convergence while treating unrelated Agent failures as terminal.
    pub fn is_mcp_admission(&self) -> bool {
        self.source
            .as_deref()
            .is_some_and(<dyn Error + Send + Sync + 'static>::is::<McpAdmissionFailure>)
    }
    /// Creates a semantic runtime failure that has no lower-level source.
    pub fn new(
        classification: ErrorClassification,
        public_error: PublicError,
        context: impl Into<String>,
    ) -> Self {
        Self {
            classification,
            public_error,
            context: context.into(),
            source: None,
            answered: false,
            turn_failure: None,
        }
    }

    /// Creates an internal failure while retaining its concrete source chain.
    pub fn internal(context: &'static str, source: impl Error + Send + Sync + 'static) -> Self {
        Self::with_source(
            ErrorClassification::Internal,
            PublicError::InternalError(EmptyErrorParams {}),
            context,
            source,
        )
    }

    /// Creates a classified semantic failure while retaining a lower-level source.
    pub fn with_source(
        classification: ErrorClassification,
        public_error: PublicError,
        context: &'static str,
        source: impl Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            classification,
            public_error,
            context: context.to_string(),
            source: Some(Arc::new(source)),
            answered: false,
            turn_failure: None,
        }
    }

    /// Rebuilds a failure a host already classified, keeping its context and source chain intact.
    pub fn from_parts(
        classification: ErrorClassification,
        public_error: PublicError,
        context: String,
        source: Option<SharedError>,
    ) -> Self {
        Self {
            classification,
            public_error,
            context,
            source,
            answered: false,
            turn_failure: None,
        }
    }

    /// Attaches the bounded reason this turn failed, recorded on its `TurnEnded`.
    pub(crate) fn with_turn_failure(mut self, failure: TurnFailure) -> Self {
        self.turn_failure = Some(failure);
        self
    }

    /// The bounded reason this turn failed, if the runtime observed a failure first.
    pub fn turn_failure(&self) -> Option<&TurnFailure> {
        self.turn_failure.as_ref()
    }

    /// Returns the category a host maps into native status and logging semantics.
    pub const fn classification(&self) -> ErrorClassification {
        self.classification
    }

    /// Returns the strongly typed public error without exposing internal diagnostics.
    pub const fn public_error(&self) -> &PublicError {
        &self.public_error
    }

    /// Splits the failure so a host can rebuild it as its own error type without losing any part.
    pub fn into_parts(
        self,
    ) -> (
        ErrorClassification,
        PublicError,
        String,
        Option<SharedError>,
    ) {
        (
            self.classification,
            self.public_error,
            self.context,
            self.source,
        )
    }
}

#[derive(Debug, thiserror::Error)]
#[error("MCP admission failed")]
struct McpAdmissionFailure(#[source] RuntimeError);

/// Marks [`RuntimeError::turn_cancel_unanswered`].
#[derive(Debug, thiserror::Error)]
#[error("turn cancel unanswered")]
struct TurnCancelUnanswered;

impl fmt::Display for RuntimeError {
    /// Formats only the semantic context added by the runtime.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.context)
    }
}

impl Error for RuntimeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source.as_deref().map(|source| source as &dyn Error)
    }
}
