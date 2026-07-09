/// Enumerates plugin-manager failures that callers can handle without knowing process details.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PluginManagerError {
    #[error("plugin not found: {plugin_id}")]
    PluginNotFound { plugin_id: String },
    #[error("plugin {plugin_id} does not provide capability {method}")]
    CapabilityNotFound { plugin_id: String, method: String },
    #[error("plugin process timed out for plugin {plugin_id}")]
    ProcessTimedOut { plugin_id: String },
    #[error("plugin process failed for plugin {plugin_id}: {message}")]
    ProcessFailed { plugin_id: String, message: String },
    #[error(
        "plugin process exited unsuccessfully for plugin {plugin_id}: exit code {exit_code:?}, stderr: {stderr}"
    )]
    ProcessExitFailed {
        plugin_id: String,
        exit_code: Option<i32>,
        stderr: String,
    },
    #[error("plugin response was not valid JSON-RPC: {message}")]
    InvalidResponse { message: String },
    #[error("plugin response id mismatch: expected {expected}, got {actual}")]
    ResponseIdMismatch { expected: String, actual: String },
    #[error("plugin returned JSON-RPC error {code}: {message}")]
    JsonRpcError { code: i64, message: String },
}
