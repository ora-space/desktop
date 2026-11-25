//! Shared wire contract between Ora's Rust host and TypeScript plugin SDK.
//!
//! This crate is the protocol seam: production runtimes, test plugin processes, and generated
//! TypeScript bindings all consume the same method names, DTOs, and binary framing rules.

mod agent;
mod child_process;
mod frame;
mod model_context;
mod registration;
mod storage;
mod workbench;

pub use agent::*;
pub use child_process::*;
pub use frame::*;
pub use model_context::*;
pub use registration::*;
pub use storage::*;
pub use workbench::*;

use std::path::Path;
use ts_rs::{Config, ExportError};

/// JSON-RPC version required by every plugin protocol message.
pub const JSON_RPC_VERSION: &str = "2.0";
/// Registration notification sent exactly once when a plugin process starts.
pub const REGISTER_METHOD: &str = "ora/register";
/// Host notification asking a plugin process to exit gracefully.
pub const SHUTDOWN_METHOD: &str = "ora/shutdown";

/// JSON-RPC code used when a request's parameters do not match its contract.
pub const INVALID_PARAMS_CODE: i64 = -32602;
/// JSON-RPC code used when a receiver does not implement a requested method.
pub const METHOD_NOT_FOUND_CODE: i64 = -32601;
/// JSON-RPC code used for an unexpected method failure.
pub const INTERNAL_ERROR_CODE: i64 = -32603;

/// Exports every TypeScript DTO owned by the plugin protocol into the SDK protocol directory.
pub fn export_typescript_bindings_to(
    output_directory: impl AsRef<Path>,
) -> Result<(), ExportError> {
    let config = Config::new().with_out_dir(output_directory.as_ref());
    agent::export(&config)?;
    model_context::export(&config)?;
    child_process::export(&config)?;
    registration::export(&config)?;
    storage::export(&config)?;
    workbench::export(&config)?;
    Ok(())
}
