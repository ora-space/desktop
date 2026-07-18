use std::fmt::Display;

use derive_more::Display;
use serde::{Deserialize, Serialize};
use serde_with::{DefaultOnError, serde_as, skip_serializing_none};

use super::serde_util::IntoOption;

/// Convenience result type using this protocol version's error type.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// JSON-RPC error object.
///
/// Represents an error that occurred during method execution, following the
/// JSON-RPC 2.0 error object specification with optional additional data.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Error {
    /// A number indicating the error type that occurred.
    /// This must be an integer as defined in the JSON-RPC specification.
    pub code: ErrorCode,
    /// A string providing a short description of the error.
    /// The message should be limited to a concise single sentence.
    pub message: String,
    /// Optional primitive or structured value that contains additional information about the error.
    /// This may include debugging information or context-specific details.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

impl Error {
    /// Creates a new error with the given code and message.
    ///
    /// The code parameter can be an `ErrorCode` constant or a tuple of (code, message).
    #[must_use]
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Error {
            code: code.into(),
            message: message.into(),
            data: None,
        }
    }

    /// Adds additional data to the error.
    ///
    /// This method is chainable and allows attaching context-specific information
    /// to help with debugging or provide more details about the error.
    #[must_use]
    pub fn data(mut self, data: impl IntoOption<serde_json::Value>) -> Self {
        self.data = data.into_option();
        self
    }

    /// Invalid JSON was received by the server. An error occurred on the server while parsing the JSON text.
    #[must_use]
    pub fn parse_error() -> Self {
        ErrorCode::ParseError.into()
    }

    /// The JSON sent is not a valid Request object.
    #[must_use]
    pub fn invalid_request() -> Self {
        ErrorCode::InvalidRequest.into()
    }

    /// The method does not exist / is not available.
    #[must_use]
    pub fn method_not_found() -> Self {
        ErrorCode::MethodNotFound.into()
    }

    /// Invalid method parameter(s).
    #[must_use]
    pub fn invalid_params() -> Self {
        ErrorCode::InvalidParams.into()
    }

    /// Internal JSON-RPC error.
    #[must_use]
    pub fn internal_error() -> Self {
        ErrorCode::InternalError.into()
    }

    /// Request was cancelled.
    ///
    /// Execution of the method was aborted either due to a cancellation request from the caller
    /// or because of resource constraints or shutdown.
    #[must_use]
    pub fn request_cancelled() -> Self {
        ErrorCode::RequestCancelled.into()
    }

    /// Authentication required.
    #[must_use]
    pub fn auth_required() -> Self {
        ErrorCode::AuthRequired.into()
    }

    /// A given resource, such as a file, was not found.
    #[must_use]
    pub fn resource_not_found(uri: Option<String>) -> Self {
        let err: Self = ErrorCode::ResourceNotFound.into();
        if let Some(uri) = uri {
            err.data(serde_json::json!({ "uri": uri }))
        } else {
            err
        }
    }

    /// Converts a standard error into an internal JSON-RPC error.
    ///
    /// The error's string representation is included as additional data.
    #[must_use]
    pub fn into_internal_error(err: impl std::error::Error) -> Self {
        Error::internal_error().data(err.to_string())
    }
}

/// Predefined error codes for common JSON-RPC and ACP-specific errors.
///
/// These codes follow the JSON-RPC 2.0 specification for standard errors
/// and use the reserved range (-32000 to -32099) for protocol-specific errors.
#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize, Display)]
#[serde(from = "i32", into = "i32")]
pub enum ErrorCode {
    // Standard errors
    /// Invalid JSON was received by the server.
    /// An error occurred on the server while parsing the JSON text.
    #[display("Parse error")]
    ParseError, // -32700
    /// The JSON sent is not a valid Request object.
    #[display("Invalid request")]
    InvalidRequest, // -32600
    /// The method does not exist or is not available.
    #[display("Method not found")]
    MethodNotFound, // -32601
    /// Invalid method parameter(s).
    #[display("Invalid params")]
    InvalidParams, // -32602
    /// Internal JSON-RPC error.
    /// Reserved for implementation-defined server errors.
    #[display("Internal error")]
    InternalError, // -32603
    /// Execution of the method was aborted either due to a cancellation request from the caller or
    /// because of resource constraints or shutdown.
    #[display("Request cancelled")]
    RequestCancelled, // -32800

    // Custom errors
    /// Authentication is required before this operation can be performed.
    #[display("Authentication required")]
    AuthRequired, // -32000
    /// A given resource, such as a file, was not found.
    #[display("Resource not found")]
    ResourceNotFound, // -32002
    /// Other undefined error code.
    #[display("Unknown error")]
    Other(i32),
}

impl From<i32> for ErrorCode {
    fn from(value: i32) -> Self {
        match value {
            -32700 => ErrorCode::ParseError,
            -32600 => ErrorCode::InvalidRequest,
            -32601 => ErrorCode::MethodNotFound,
            -32602 => ErrorCode::InvalidParams,
            -32603 => ErrorCode::InternalError,
            -32800 => ErrorCode::RequestCancelled,
            -32000 => ErrorCode::AuthRequired,
            -32002 => ErrorCode::ResourceNotFound,
            _ => ErrorCode::Other(value),
        }
    }
}

impl From<ErrorCode> for i32 {
    fn from(value: ErrorCode) -> Self {
        match value {
            ErrorCode::ParseError => -32700,
            ErrorCode::InvalidRequest => -32600,
            ErrorCode::MethodNotFound => -32601,
            ErrorCode::InvalidParams => -32602,
            ErrorCode::InternalError => -32603,
            ErrorCode::RequestCancelled => -32800,
            ErrorCode::AuthRequired => -32000,
            ErrorCode::ResourceNotFound => -32002,
            ErrorCode::Other(value) => value,
        }
    }
}

impl std::fmt::Debug for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {self}", i32::from(*self))
    }
}

impl From<ErrorCode> for Error {
    fn from(error_code: ErrorCode) -> Self {
        Error::new(error_code.into(), error_code.to_string())
    }
}

impl std::error::Error for Error {}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.message.is_empty() {
            write!(f, "{}", i32::from(self.code))?;
        } else {
            write!(f, "{}", self.message)?;
        }

        if let Some(data) = &self.data {
            let pretty = serde_json::to_string_pretty(data).unwrap_or_else(|_| data.to_string());
            write!(f, ": {pretty}")?;
        }

        Ok(())
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Error::invalid_params().data(error.to_string())
    }
}
