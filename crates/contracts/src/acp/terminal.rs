use std::{path::PathBuf, sync::Arc};

use derive_more::{Display, From};
use serde::{Deserialize, Serialize};
use serde_with::{DefaultOnError, serde_as, skip_serializing_none};
use ts_rs::TS;

use super::common::{EnvVariable, SessionId};
use super::serde_util::IntoOption;

/// Typed identifier used for terminal values on the wire.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Display, From, TS)]
#[serde(transparent)]
#[from(Arc<str>, String, &'static str)]
#[ts(type = "string", export_to = "acp/terminal.ts")]
pub struct TerminalId(pub Arc<str>);

impl TerminalId {
    /// Wraps a protocol string as a typed [`TerminalId`].
    #[must_use]
    pub fn new(id: impl Into<Arc<str>>) -> Self {
        Self(id.into())
    }
}

/// Request to create a new terminal and execute a command.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/terminal.ts")]
#[serde(rename_all = "camelCase")]
pub struct CreateTerminalRequest {
    /// The session ID for this request.
    pub session_id: SessionId,
    /// The command to execute.
    pub command: String,
    /// Array of command arguments.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Environment variables for the command.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<EnvVariable>,
    /// Working directory for the command. Must be an absolute path.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    /// Maximum number of output bytes to retain.
    ///
    /// When the limit is exceeded, the Client truncates from the beginning of the output
    /// to stay within the limit.
    ///
    /// The Client MUST ensure truncation happens at a character boundary to maintain valid
    /// string output, even if this means the retained output is slightly less than the
    /// specified limit.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub output_byte_limit: Option<u64>,
}

impl CreateTerminalRequest {
    /// Builds [`CreateTerminalRequest`] with the required request fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(session_id: impl Into<SessionId>, command: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            command: command.into(),
            args: Vec::new(),
            env: Vec::new(),
            cwd: None,
            output_byte_limit: None,
        }
    }

    /// Array of command arguments.
    #[must_use]
    pub fn args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    /// Environment variables for the command.
    #[must_use]
    pub fn env(mut self, env: Vec<EnvVariable>) -> Self {
        self.env = env;
        self
    }

    /// Working directory for the command. Must be an absolute path.
    #[must_use]
    pub fn cwd(mut self, cwd: impl IntoOption<PathBuf>) -> Self {
        self.cwd = cwd.into_option();
        self
    }

    /// Maximum number of output bytes to retain.
    ///
    /// When the limit is exceeded, the Client truncates from the beginning of the output
    /// to stay within the limit.
    ///
    /// The Client MUST ensure truncation happens at a character boundary to maintain valid
    /// string output, even if this means the retained output is slightly less than the
    /// specified limit.
    #[must_use]
    pub fn output_byte_limit(mut self, output_byte_limit: impl IntoOption<u64>) -> Self {
        self.output_byte_limit = output_byte_limit.into_option();
        self
    }
}

/// Response containing the ID of the created terminal.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/terminal.ts")]
#[serde(rename_all = "camelCase")]
pub struct CreateTerminalResponse {
    /// The unique identifier for the created terminal.
    pub terminal_id: TerminalId,
}

impl CreateTerminalResponse {
    /// Builds [`CreateTerminalResponse`] with the required response fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(terminal_id: impl Into<TerminalId>) -> Self {
        Self {
            terminal_id: terminal_id.into(),
        }
    }
}

/// Request to get the current output and status of a terminal.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/terminal.ts")]
#[serde(rename_all = "camelCase")]
pub struct TerminalOutputRequest {
    /// The session ID for this request.
    pub session_id: SessionId,
    /// The ID of the terminal to get output from.
    pub terminal_id: TerminalId,
}

impl TerminalOutputRequest {
    /// Builds [`TerminalOutputRequest`] with the required request fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(session_id: impl Into<SessionId>, terminal_id: impl Into<TerminalId>) -> Self {
        Self {
            session_id: session_id.into(),
            terminal_id: terminal_id.into(),
        }
    }
}

/// Response containing the terminal output and exit status.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/terminal.ts")]
#[serde(rename_all = "camelCase")]
pub struct TerminalOutputResponse {
    /// The terminal output captured so far.
    pub output: String,
    /// Whether the output was truncated due to byte limits.
    pub truncated: bool,
    /// Exit status if the command has completed.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub exit_status: Option<TerminalExitStatus>,
}

impl TerminalOutputResponse {
    /// Builds [`TerminalOutputResponse`] with the required response fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(output: impl Into<String>, truncated: bool) -> Self {
        Self {
            output: output.into(),
            truncated,
            exit_status: None,
        }
    }

    /// Exit status if the command has completed.
    #[must_use]
    pub fn exit_status(mut self, exit_status: impl IntoOption<TerminalExitStatus>) -> Self {
        self.exit_status = exit_status.into_option();
        self
    }
}

/// Request to release a terminal and free its resources.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/terminal.ts")]
#[serde(rename_all = "camelCase")]
pub struct ReleaseTerminalRequest {
    /// The session ID for this request.
    pub session_id: SessionId,
    /// The ID of the terminal to release.
    pub terminal_id: TerminalId,
}

impl ReleaseTerminalRequest {
    /// Builds [`ReleaseTerminalRequest`] with the required request fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(session_id: impl Into<SessionId>, terminal_id: impl Into<TerminalId>) -> Self {
        Self {
            session_id: session_id.into(),
            terminal_id: terminal_id.into(),
        }
    }
}

/// Response to terminal/release method
#[serde_as]
#[skip_serializing_none]
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/terminal.ts")]
#[serde(rename_all = "camelCase")]
pub struct ReleaseTerminalResponse {}

impl ReleaseTerminalResponse {
    /// Builds [`ReleaseTerminalResponse`] with the required response fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Request to kill a terminal without releasing it.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/terminal.ts")]
#[serde(rename_all = "camelCase")]
pub struct KillTerminalRequest {
    /// The session ID for this request.
    pub session_id: SessionId,
    /// The ID of the terminal to kill.
    pub terminal_id: TerminalId,
}

impl KillTerminalRequest {
    /// Builds [`KillTerminalRequest`] with the required request fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(session_id: impl Into<SessionId>, terminal_id: impl Into<TerminalId>) -> Self {
        Self {
            session_id: session_id.into(),
            terminal_id: terminal_id.into(),
        }
    }
}

/// Response to `terminal/kill` method
#[serde_as]
#[skip_serializing_none]
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/terminal.ts")]
#[serde(rename_all = "camelCase")]
pub struct KillTerminalResponse {}

impl KillTerminalResponse {
    /// Builds [`KillTerminalResponse`] with the required response fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Request to wait for a terminal command to exit.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/terminal.ts")]
#[serde(rename_all = "camelCase")]
pub struct WaitForTerminalExitRequest {
    /// The session ID for this request.
    pub session_id: SessionId,
    /// The ID of the terminal to wait for.
    pub terminal_id: TerminalId,
}

impl WaitForTerminalExitRequest {
    /// Builds [`WaitForTerminalExitRequest`] with the required request fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(session_id: impl Into<SessionId>, terminal_id: impl Into<TerminalId>) -> Self {
        Self {
            session_id: session_id.into(),
            terminal_id: terminal_id.into(),
        }
    }
}

/// Response containing the exit status of a terminal command.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/terminal.ts")]
#[serde(rename_all = "camelCase")]
pub struct WaitForTerminalExitResponse {
    /// The exit status of the terminal command.
    #[serde(flatten)]
    pub exit_status: TerminalExitStatus,
}

impl WaitForTerminalExitResponse {
    /// Builds [`WaitForTerminalExitResponse`] with the required response fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(exit_status: TerminalExitStatus) -> Self {
        Self { exit_status }
    }
}

/// Exit status of a terminal command.
#[serde_as]
#[skip_serializing_none]
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/terminal.ts")]
#[serde(rename_all = "camelCase")]
pub struct TerminalExitStatus {
    /// The process exit code (may be null if terminated by signal).
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub exit_code: Option<u32>,
    /// The signal that terminated the process (may be null if exited normally).
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub signal: Option<String>,
}

impl TerminalExitStatus {
    /// Builds [`TerminalExitStatus`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The process exit code (may be null if terminated by signal).
    #[must_use]
    pub fn exit_code(mut self, exit_code: impl IntoOption<u32>) -> Self {
        self.exit_code = exit_code.into_option();
        self
    }

    /// The signal that terminated the process (may be null if exited normally).
    #[must_use]
    pub fn signal(mut self, signal: impl IntoOption<String>) -> Self {
        self.signal = signal.into_option();
        self
    }
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    TerminalId::export(config)?;
    TerminalExitStatus::export(config)?;
    CreateTerminalRequest::export(config)?;
    CreateTerminalResponse::export(config)?;
    TerminalOutputRequest::export(config)?;
    TerminalOutputResponse::export(config)?;
    ReleaseTerminalRequest::export(config)?;
    ReleaseTerminalResponse::export(config)?;
    KillTerminalRequest::export(config)?;
    KillTerminalResponse::export(config)?;
    WaitForTerminalExitRequest::export(config)?;
    WaitForTerminalExitResponse::export(config)?;
    Ok(())
}
