use ora_contracts::{EmptyErrorParams, PublicError};
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
        }
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
