use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_with::{DefaultOnError, serde_as, skip_serializing_none};
use ts_rs::TS;

use super::{common::SessionId, serde_util::IntoOption};

/// Request to write content to a text file.
///
/// Only available if the client supports the `fs.writeTextFile` capability.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/file.ts")]
#[serde(rename_all = "camelCase")]
pub struct WriteTextFileRequest {
    /// The session ID for this request.
    pub session_id: SessionId,
    /// Absolute path to the file to write.
    pub path: PathBuf,
    /// The text content to write to the file.
    pub content: String,
}

impl WriteTextFileRequest {
    /// Builds [`WriteTextFileRequest`] with the required request fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(
        session_id: impl Into<SessionId>,
        path: impl Into<PathBuf>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            path: path.into(),
            content: content.into(),
        }
    }
}

/// Response to `fs/write_text_file`
#[serde_as]
#[skip_serializing_none]
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/file.ts")]
#[serde(rename_all = "camelCase")]
pub struct WriteTextFileResponse {}

impl WriteTextFileResponse {
    /// Builds [`WriteTextFileResponse`] with the required response fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Request to read content from a text file.
///
/// Only available if the client supports the `fs.readTextFile` capability.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/file.ts")]
#[serde(rename_all = "camelCase")]
pub struct ReadTextFileRequest {
    /// The session ID for this request.
    pub session_id: SessionId,
    /// Absolute path to the file to read.
    pub path: PathBuf,
    /// Line number to start reading from (1-based).
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub line: Option<u32>,
    /// Maximum number of lines to read.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub limit: Option<u32>,
}

impl ReadTextFileRequest {
    /// Builds [`ReadTextFileRequest`] with the required request fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(session_id: impl Into<SessionId>, path: impl Into<PathBuf>) -> Self {
        Self {
            session_id: session_id.into(),
            path: path.into(),
            line: None,
            limit: None,
        }
    }

    /// Line number to start reading from (1-based).
    #[must_use]
    pub fn line(mut self, line: impl IntoOption<u32>) -> Self {
        self.line = line.into_option();
        self
    }

    /// Maximum number of lines to read.
    #[must_use]
    pub fn limit(mut self, limit: impl IntoOption<u32>) -> Self {
        self.limit = limit.into_option();
        self
    }
}

/// Response containing the contents of a text file.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/file.ts")]
#[serde(rename_all = "camelCase")]
pub struct ReadTextFileResponse {
    /// Content payload returned by this response.
    pub content: String,
}

impl ReadTextFileResponse {
    /// Builds [`ReadTextFileResponse`] with the required response fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
        }
    }
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    WriteTextFileRequest::export(config)?;
    WriteTextFileResponse::export(config)?;
    ReadTextFileRequest::export(config)?;
    ReadTextFileResponse::export(config)?;
    Ok(())
}
