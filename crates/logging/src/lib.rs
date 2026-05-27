mod config;
mod correlation;
mod error;
mod file_output;
mod formatter;
mod gitlancer_bridge;
mod guard;
mod init;
mod macros;
mod method_name;
mod test_support;

#[cfg(test)]
mod tests;

pub use config::{FileLoggingConfig, LogLevel, LogOutput, LoggingConfig, RotationPolicy};
pub use correlation::{
    runtime_span, span_with_correlation, span_with_request_id, span_with_trace_id,
};
pub use error::{FileSystemAction, LoggingInitError};
pub use gitlancer_bridge::{OraGitlancerLogger, register_gitlancer_logger};
pub use guard::LoggingGuard;
pub use init::init_logging;
pub use test_support::{with_recorded_trace_logging, with_trace_logging};

#[cfg(test)]
pub(crate) use init::build_dispatch;

#[doc(hidden)]
pub mod __private {
    pub use crate::method_name::method_name_from_marker_type_name;
}
