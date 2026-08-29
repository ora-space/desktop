//! Invokes `agent/configureWorkspace` without logging snapshot secrets.

#![allow(dead_code)] // First production caller is the Agent Target worker in #489.

use super::{
    ExpectedReceiptCoverage, McpConfigurationReceipt, PreparedMcpConfiguration,
    ReceiptValidationError, parse_mcp_configuration_receipt, snapshot_request_json,
    validate_mcp_configuration_receipt,
};
use ora_logging::{ErrorReport, ora_info};
use ora_plugin_runtime::{CONFIGURE_WORKSPACE_METHOD, PluginRuntime, PluginRuntimeError};
use serde_json::Value;
use std::future::Future;
use thiserror::Error;

/// Abstracts one plugin invoke so snapshot/receipt protocol tests do not need a Deno process.
///
/// Implementations must return the JSON-RPC result only. Logging the params would put header and
/// environment values into Host traces.
pub(crate) trait ConfigureWorkspaceRuntime {
    /// Invokes one registered plugin method and returns its JSON result.
    fn invoke(
        &self,
        method: &str,
        params: Value,
    ) -> impl Future<Output = Result<Value, PluginRuntimeError>> + Send;
}

impl ConfigureWorkspaceRuntime for PluginRuntime {
    /// Delegates to the process runtime without logging params, so header values stay off traces.
    async fn invoke(&self, method: &str, params: Value) -> Result<Value, PluginRuntimeError> {
        PluginRuntime::invoke(self, method, params).await
    }
}

/// Why configure failed without becoming a Ready generation.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub(crate) enum ConfigureWorkspaceError {
    #[error("agent plugin did not register agent/configureWorkspace")]
    MethodNotRegistered,
    #[error("agent plugin configure call timed out")]
    TimedOut,
    #[error("agent plugin configure failed: {0}")]
    Failed(String),
    #[error("agent plugin configure receipt is invalid: {0}")]
    InvalidReceipt(ReceiptValidationError),
}

impl From<PluginRuntimeError> for ConfigureWorkspaceError {
    fn from(error: PluginRuntimeError) -> Self {
        match error {
            PluginRuntimeError::MethodNotRegistered(_) => Self::MethodNotRegistered,
            PluginRuntimeError::CallTimeout => Self::TimedOut,
            PluginRuntimeError::MissingEntrypoint(_)
            | PluginRuntimeError::Spawn(_)
            | PluginRuntimeError::MissingStdio
            | PluginRuntimeError::ReadyTimeout
            | PluginRuntimeError::Unavailable(_)
            | PluginRuntimeError::RequestChannelClosed
            | PluginRuntimeError::Remote { .. } => {
                Self::Failed(ErrorReport::sanitize_text(&error.to_string()))
            }
        }
    }
}

/// Sends one complete snapshot, then accepts the receipt only when coverage is exact.
pub(crate) async fn configure_workspace<R: ConfigureWorkspaceRuntime>(
    runtime: &R,
    prepared: &PreparedMcpConfiguration,
    expected: &ExpectedReceiptCoverage,
) -> Result<McpConfigurationReceipt, ConfigureWorkspaceError> {
    ora_info!(
        message = "agent configure workspace",
        operation_id = %prepared.operation_id,
        agent_target_id = %prepared.agent_target_id,
        generation = prepared.generation.value(),
    );
    let result = runtime
        .invoke(CONFIGURE_WORKSPACE_METHOD, snapshot_request_json(prepared))
        .await?;
    let receipt =
        parse_mcp_configuration_receipt(result).map_err(ConfigureWorkspaceError::InvalidReceipt)?;
    validate_mcp_configuration_receipt(&receipt, expected)
        .map_err(ConfigureWorkspaceError::InvalidReceipt)?;
    Ok(receipt)
}
