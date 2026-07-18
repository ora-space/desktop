use axum::Json;
use axum::extract::Request;
use axum::extract::rejection::JsonRejection;
use axum::http::{HeaderValue, StatusCode, header::HeaderName};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use ora_application::ApplicationError;
use ora_backend::{BackendError, ErrorClassification, RequestLifecycle, UuidRequestIdGenerator};
use ora_contracts::{EmptyErrorParams, PublicError};
use thiserror::Error;
use tracing::Instrument;

pub const X_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

tokio::task_local! {
    static REQUEST_LIFECYCLE: RequestLifecycle;
}

/// Reports bootstrap-time configuration, listener, and logging failures for the web server entry point.
#[derive(Debug, Error)]
pub enum WebBootstrapError {
    #[error("invalid ORA_HOST value `{value}`")]
    InvalidHost {
        value: String,
        #[source]
        source: std::net::AddrParseError,
    },
    #[error("invalid ORA_PORT value `{value}`")]
    InvalidPort {
        value: String,
        #[source]
        source: std::num::ParseIntError,
    },
    #[error("invalid ORA_LOG_LEVEL value `{value}`")]
    InvalidLogLevel { value: String },
    #[error("invalid ORA_LOG_MODE value `{value}`")]
    InvalidLogMode { value: String },
    #[error("invalid ORA_LOG_MAX_DAYS value `{value}`")]
    InvalidLogMaxDays {
        value: String,
        #[source]
        source: std::num::ParseIntError,
    },
    #[error("ORA_DATA_DIR must not be empty")]
    InvalidDatabasePathEmpty,
    #[error("failed to resolve the current directory")]
    CurrentDirectory(#[source] std::io::Error),
    #[error("ORA_PROJECT_NAME must not be empty")]
    InvalidProjectNameEmpty,
    #[error("ORA_PROJECT_PATH must not be empty")]
    InvalidProjectPathEmpty,
    #[error("ORA_LOG_MAX_DAYS must be greater than zero")]
    InvalidLogMaxDaysZero,
    #[error("server user home directory is unavailable")]
    HomeDirectoryUnavailable,
    #[error("server user home directory must be absolute: {home_directory:?}")]
    HomeDirectoryNotAbsolute { home_directory: std::path::PathBuf },
    #[error("failed to create runtime data directory")]
    DataDirectoryCreate(#[source] std::io::Error),
    #[error("failed to bootstrap SQLite database")]
    DatabaseBootstrap(#[source] ora_db::DatabaseError),
    #[error("failed to reconcile bootstrap project")]
    ProjectBootstrap {
        #[source]
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },
    #[error("failed to reconcile skill storage")]
    SkillStorageReconcile {
        #[source]
        source: ora_application::ApplicationError,
    },
    #[error(transparent)]
    LoggingInit(#[from] ora_logging::LoggingInitError),
    #[error("failed to bind HTTP listener")]
    Bind(#[source] std::io::Error),
    #[error("HTTP server exited unexpectedly")]
    Serve(#[source] std::io::Error),
}

/// Owns the internal failure until Axum serializes its typed public projection.
pub struct WebApiError {
    error: BackendError,
}

impl WebApiError {
    /// Creates a malformed-input failure without returning parser-generated diagnostics.
    pub fn invalid_request(context: &'static str) -> Self {
        Self::semantic(
            ErrorClassification::InvalidRequest,
            PublicError::InvalidRequest(EmptyErrorParams {}),
            context,
        )
    }

    /// Creates a source-free semantic failure from a typed public variant.
    pub fn semantic(
        classification: ErrorClassification,
        public_error: PublicError,
        context: &'static str,
    ) -> Self {
        Self {
            error: BackendError::new(classification, public_error, context),
        }
    }

    /// Creates a typed filesystem failure and retains the concrete filesystem source.
    pub fn with_source(
        classification: ErrorClassification,
        public_error: PublicError,
        context: &'static str,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            error: BackendError::with_source(classification, public_error, context, source),
        }
    }

    /// Creates an internal adapter failure and retains its concrete source.
    pub fn internal(
        context: &'static str,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            error: BackendError::internal(context, source),
        }
    }
}

impl From<ApplicationError> for WebApiError {
    fn from(error: ApplicationError) -> Self {
        Self {
            error: BackendError::from(error),
        }
    }
}

impl From<BackendError> for WebApiError {
    fn from(error: BackendError) -> Self {
        Self { error }
    }
}

impl From<JsonRejection> for WebApiError {
    fn from(_error: JsonRejection) -> Self {
        Self::invalid_request("failed to decode JSON request")
    }
}

impl From<axum::extract::rejection::QueryRejection> for WebApiError {
    fn from(_error: axum::extract::rejection::QueryRejection) -> Self {
        Self::invalid_request("failed to decode query request")
    }
}

impl IntoResponse for WebApiError {
    fn into_response(self) -> Response {
        let lifecycle = current_lifecycle();
        lifecycle.complete_failure(&self.error);
        let status = status_for(self.error.classification());
        (
            status,
            Json(self.error.contract_error(lifecycle.request_id())),
        )
            .into_response()
    }
}

/// Establishes the canonical request ID before any extractor or handler can fail.
pub async fn request_context(mut request: Request, next: Next) -> Response {
    let path = request.uri().path().to_string();
    let is_health_check = matches!(path.as_str(), "/health/live" | "/health/ready");
    let operation = format!("{} {}", request.method(), path);
    let lifecycle = RequestLifecycle::start(operation, &UuidRequestIdGenerator);
    let request_span =
        ora_logging::span_with_request_id("web_request", &lifecycle.request_id().to_string());
    let header_value = HeaderValue::from_str(&lifecycle.request_id().to_string())
        .unwrap_or_else(|error| panic!("UUID request ID was not a valid header value: {error}"));
    request.headers_mut().insert(X_REQUEST_ID, header_value);

    REQUEST_LIFECYCLE
        .scope(
            lifecycle.clone(),
            async move {
                let response = next.run(request).await;
                if response.extensions().get::<DeferredCompletion>().is_none() {
                    if is_health_check {
                        lifecycle.complete_success_debug();
                    } else {
                        lifecycle.complete_success();
                    }
                }
                response
            }
            .instrument(request_span),
        )
        .await
}

pub(crate) fn current_lifecycle() -> RequestLifecycle {
    REQUEST_LIFECYCLE
        .try_with(Clone::clone)
        .unwrap_or_else(|_| RequestLifecycle::start("web_request", &UuidRequestIdGenerator))
}

/// Marks responses whose completion is owned by their full streaming lifetime.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DeferredCompletion;

const fn status_for(classification: ErrorClassification) -> StatusCode {
    match classification {
        ErrorClassification::InvalidRequest => StatusCode::BAD_REQUEST,
        ErrorClassification::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
        ErrorClassification::NotFound => StatusCode::NOT_FOUND,
        ErrorClassification::Conflict => StatusCode::CONFLICT,
        ErrorClassification::Unprocessable => StatusCode::UNPROCESSABLE_ENTITY,
        ErrorClassification::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[cfg(test)]
mod tests {
    use super::status_for;
    use axum::http::StatusCode;
    use ora_backend::ErrorClassification;
    use pretty_assertions::assert_eq;

    /// Verifies transport-only upload limits retain their native HTTP status.
    #[test]
    fn maps_payload_too_large_classification_to_http_413() {
        assert_eq!(
            status_for(ErrorClassification::PayloadTooLarge),
            StatusCode::PAYLOAD_TOO_LARGE
        );
    }
}
