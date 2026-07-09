use std::path::PathBuf;
use std::time::Duration;

/// Carries one complete plugin process invocation requested by the manager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginProcessRequest {
    pub plugin_id: String,
    pub program: PathBuf,
    pub args: Vec<PathBuf>,
    pub cwd: PathBuf,
    pub stdin: String,
    pub timeout: Duration,
}

/// Carries the collected output from one plugin process invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginProcessOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
}

/// Supplies plugin process execution behind a testable boundary.
///
/// The first production adapter should translate this request into the process crate owned by
/// the runtime team while preserving stdin/stdout as plain text byte-stream payloads.
pub trait PluginProcessRuntime {
    /// Runs one plugin process invocation and returns its collected output.
    fn run_plugin_process(
        &self,
        request: PluginProcessRequest,
    ) -> Result<PluginProcessOutput, PluginProcessRuntimeError>;
}

/// Captures process-layer failures before a plugin can produce JSON-RPC output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginProcessRuntimeError {
    TimedOut,
    OperationFailed(String),
}
