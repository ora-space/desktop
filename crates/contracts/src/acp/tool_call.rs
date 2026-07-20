use std::{path::PathBuf, sync::Arc};

use derive_more::{Display, From};
use serde::{Deserialize, Serialize};
use serde_with::{DefaultOnError, serde_as, skip_serializing_none};
use ts_rs::TS;

use super::{content::ContentBlock, error::Error, serde_util::IntoOption, terminal::TerminalId};

/// Represents a tool call that the language model has requested.
///
/// Tool calls are actions that the agent executes on behalf of the language model,
/// such as reading files, executing code, or fetching data from external sources.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "acp/tool_call.ts")]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    /// Unique identifier for this tool call within the session.
    pub tool_call_id: ToolCallId,
    /// Human-readable title describing what the tool is doing.
    pub title: String,
    /// The category of tool being invoked.
    /// Helps clients choose appropriate icons and UI treatment.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, skip_serializing_if = "ToolKind::is_default")]
    pub kind: ToolKind,
    /// Current execution status of the tool call.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, skip_serializing_if = "ToolCallStatus::is_default")]
    pub status: ToolCallStatus,
    /// Content produced by the tool call.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<ToolCallContent>,
    /// File locations affected by this tool call.
    /// Enables "follow-along" features in clients.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locations: Vec<ToolCallLocation>,
    /// Raw input parameters sent to the tool.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    #[ts(type = "unknown")]
    pub raw_input: Option<serde_json::Value>,
    /// Raw output returned by the tool.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    #[ts(type = "unknown")]
    pub raw_output: Option<serde_json::Value>,
}

impl ToolCall {
    /// Builds [`ToolCall`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(tool_call_id: impl Into<ToolCallId>, title: impl Into<String>) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            title: title.into(),
            kind: ToolKind::default(),
            status: ToolCallStatus::default(),
            content: Vec::default(),
            locations: Vec::default(),
            raw_input: None,
            raw_output: None,
        }
    }

    /// The category of tool being invoked.
    /// Helps clients choose appropriate icons and UI treatment.
    #[must_use]
    pub fn kind(mut self, kind: ToolKind) -> Self {
        self.kind = kind;
        self
    }

    /// Current execution status of the tool call.
    #[must_use]
    pub fn status(mut self, status: ToolCallStatus) -> Self {
        self.status = status;
        self
    }

    /// Content produced by the tool call.
    #[must_use]
    pub fn content(mut self, content: Vec<ToolCallContent>) -> Self {
        self.content = content;
        self
    }

    /// File locations affected by this tool call.
    /// Enables "follow-along" features in clients.
    #[must_use]
    pub fn locations(mut self, locations: Vec<ToolCallLocation>) -> Self {
        self.locations = locations;
        self
    }

    /// Raw input parameters sent to the tool.
    #[must_use]
    pub fn raw_input(mut self, raw_input: impl IntoOption<serde_json::Value>) -> Self {
        self.raw_input = raw_input.into_option();
        self
    }

    /// Raw output returned by the tool.
    #[must_use]
    pub fn raw_output(mut self, raw_output: impl IntoOption<serde_json::Value>) -> Self {
        self.raw_output = raw_output.into_option();
        self
    }

    /// Update an existing tool call with the values in the provided update
    /// fields. Fields with collections of values are overwritten, not extended.
    pub fn update(&mut self, fields: ToolCallUpdateFields) {
        if let Some(title) = fields.title {
            self.title = title;
        }
        if let Some(kind) = fields.kind {
            self.kind = kind;
        }
        if let Some(status) = fields.status {
            self.status = status;
        }
        if let Some(content) = fields.content {
            self.content = content;
        }
        if let Some(locations) = fields.locations {
            self.locations = locations;
        }
        if let Some(raw_input) = fields.raw_input {
            self.raw_input = Some(raw_input);
        }
        if let Some(raw_output) = fields.raw_output {
            self.raw_output = Some(raw_output);
        }
    }
}

/// An update to an existing tool call.
///
/// Used to report progress and results as tools execute. All fields except
/// the tool call ID are optional - only changed fields need to be included.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "acp/tool_call.ts")]
#[serde(rename_all = "camelCase")]
pub struct ToolCallUpdate {
    /// The ID of the tool call being updated.
    pub tool_call_id: ToolCallId,
    /// Fields being updated.
    #[serde(flatten)]
    pub fields: ToolCallUpdateFields,
}

impl ToolCallUpdate {
    /// Builds [`ToolCallUpdate`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(tool_call_id: impl Into<ToolCallId>, fields: ToolCallUpdateFields) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            fields,
        }
    }
}

/// Optional fields that can be updated in a tool call.
///
/// All fields are optional - only include the ones being changed.
/// Collections (content, locations) are overwritten, not extended.
#[serde_as]
#[skip_serializing_none]
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "acp/tool_call.ts")]
#[serde(rename_all = "camelCase")]
pub struct ToolCallUpdateFields {
    /// Update the tool kind.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub kind: Option<ToolKind>,
    /// Update the execution status.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub status: Option<ToolCallStatus>,
    /// Update the human-readable title.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub title: Option<String>,
    /// Replace the content collection.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub content: Option<Vec<ToolCallContent>>,
    /// Replace the locations collection.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub locations: Option<Vec<ToolCallLocation>>,
    /// Update the raw input.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    #[ts(type = "unknown")]
    pub raw_input: Option<serde_json::Value>,
    /// Update the raw output.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    #[ts(type = "unknown")]
    pub raw_output: Option<serde_json::Value>,
}

impl ToolCallUpdateFields {
    /// Builds [`ToolCallUpdateFields`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Update the tool kind.
    #[must_use]
    pub fn kind(mut self, kind: impl IntoOption<ToolKind>) -> Self {
        self.kind = kind.into_option();
        self
    }

    /// Update the execution status.
    #[must_use]
    pub fn status(mut self, status: impl IntoOption<ToolCallStatus>) -> Self {
        self.status = status.into_option();
        self
    }

    /// Update the human-readable title.
    #[must_use]
    pub fn title(mut self, title: impl IntoOption<String>) -> Self {
        self.title = title.into_option();
        self
    }

    /// Replace the content collection.
    #[must_use]
    pub fn content(mut self, content: impl IntoOption<Vec<ToolCallContent>>) -> Self {
        self.content = content.into_option();
        self
    }

    /// Replace the locations collection.
    #[must_use]
    pub fn locations(mut self, locations: impl IntoOption<Vec<ToolCallLocation>>) -> Self {
        self.locations = locations.into_option();
        self
    }

    /// Update the raw input.
    #[must_use]
    pub fn raw_input(mut self, raw_input: impl IntoOption<serde_json::Value>) -> Self {
        self.raw_input = raw_input.into_option();
        self
    }

    /// Update the raw output.
    #[must_use]
    pub fn raw_output(mut self, raw_output: impl IntoOption<serde_json::Value>) -> Self {
        self.raw_output = raw_output.into_option();
        self
    }
}

/// If a given tool call doesn't exist yet, allows for attempting to construct
/// one from a tool call update if possible.
impl TryFrom<ToolCallUpdate> for ToolCall {
    type Error = Error;

    fn try_from(update: ToolCallUpdate) -> Result<Self, Self::Error> {
        let ToolCallUpdate {
            tool_call_id,
            fields:
                ToolCallUpdateFields {
                    kind,
                    status,
                    title,
                    content,
                    locations,
                    raw_input,
                    raw_output,
                },
        } = update;

        Ok(Self {
            tool_call_id,
            title: title.ok_or_else(|| {
                Error::invalid_params().data(serde_json::json!("title is required for a tool call"))
            })?,
            kind: kind.unwrap_or_default(),
            status: status.unwrap_or_default(),
            content: content.unwrap_or_default(),
            locations: locations.unwrap_or_default(),
            raw_input,
            raw_output,
        })
    }
}

impl From<ToolCall> for ToolCallUpdate {
    fn from(value: ToolCall) -> Self {
        let ToolCall {
            tool_call_id,
            title,
            kind,
            status,
            content,
            locations,
            raw_input,
            raw_output,
        } = value;
        Self {
            tool_call_id,
            fields: ToolCallUpdateFields {
                kind: Some(kind),
                status: Some(status),
                title: Some(title),
                content: Some(content),
                locations: Some(locations),
                raw_input,
                raw_output,
            },
        }
    }
}

/// Unique identifier for a tool call within a session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Display, From, TS)]
#[serde(transparent)]
#[from(Arc<str>, String, &'static str)]
#[ts(type = "string", export_to = "acp/tool_call.ts")]
pub struct ToolCallId(pub Arc<str>);

impl ToolCallId {
    /// Wraps a protocol string as a typed [`ToolCallId`].
    #[must_use]
    pub fn new(id: impl Into<Arc<str>>) -> Self {
        Self(id.into())
    }
}

impl IntoOption<ToolCallId> for &str {
    fn into_option(self) -> Option<ToolCallId> {
        Some(ToolCallId::new(self))
    }
}

/// Categories of tools that can be invoked.
///
/// Tool kinds help clients choose appropriate icons and optimize how they
/// display tool execution progress.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/tool_call.ts")]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    /// Reading files or data.
    Read,
    /// Modifying files or content.
    Edit,
    /// Removing files or data.
    Delete,
    /// Moving or renaming files.
    Move,
    /// Searching for information.
    Search,
    /// Running commands or code.
    Execute,
    /// Internal reasoning or planning.
    Think,
    /// Retrieving external data.
    Fetch,
    /// Switching the current session mode.
    SwitchMode,
    /// Other tool types (default).
    #[default]
    #[serde(other)]
    Other,
}

impl ToolKind {
    #[expect(clippy::trivially_copy_pass_by_ref, reason = "Required by serde")]
    fn is_default(&self) -> bool {
        matches!(self, ToolKind::Other)
    }
}

/// Execution status of a tool call.
///
/// Tool calls progress through different statuses during their lifecycle.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/tool_call.ts")]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    /// The tool call hasn't started running yet because the input is either
    /// streaming or we're awaiting approval.
    #[default]
    Pending,
    /// The tool call is currently running.
    InProgress,
    /// The tool call completed successfully.
    Completed,
    /// The tool call failed with an error.
    Failed,
}

impl ToolCallStatus {
    #[expect(clippy::trivially_copy_pass_by_ref, reason = "Required by serde")]
    fn is_default(&self) -> bool {
        matches!(self, ToolCallStatus::Pending)
    }
}

/// Content produced by a tool call.
///
/// Tool calls can produce different types of content including
/// standard content blocks (text, images) or file diffs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "acp/tool_call.ts")]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolCallContent {
    /// Standard content block (text, images, resources).
    Content(Content),
    /// File modification shown as a diff.
    Diff(Diff),
    /// Embed a terminal created with `terminal/create` by its id.
    ///
    /// The terminal must be added before calling `terminal/release`.
    Terminal(Terminal),
}

impl<T: Into<ContentBlock>> From<T> for ToolCallContent {
    fn from(content: T) -> Self {
        ToolCallContent::Content(Content::new(content))
    }
}

impl From<Diff> for ToolCallContent {
    fn from(diff: Diff) -> Self {
        ToolCallContent::Diff(diff)
    }
}

/// Standard content block (text, images, resources).
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "acp/tool_call.ts")]
#[serde(rename_all = "camelCase")]
pub struct Content {
    /// The actual content block.
    pub content: ContentBlock,
}

impl Content {
    /// Builds [`Content`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(content: impl Into<ContentBlock>) -> Self {
        Self {
            content: content.into(),
        }
    }
}

/// Embed a terminal created with `terminal/create` by its id.
///
/// The terminal must be added before calling `terminal/release`.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "acp/tool_call.ts")]
#[serde(rename_all = "camelCase")]
pub struct Terminal {
    /// Identifier of the terminal instance to embed in the content stream.
    pub terminal_id: TerminalId,
}

impl Terminal {
    /// Builds [`Terminal`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(terminal_id: impl Into<TerminalId>) -> Self {
        Self {
            terminal_id: terminal_id.into(),
        }
    }
}

/// A diff representing file modifications.
///
/// Shows changes to files in a format suitable for display in the client UI.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "acp/tool_call.ts")]
#[serde(rename_all = "camelCase")]
pub struct Diff {
    /// The absolute file path being modified.
    pub path: PathBuf,
    /// The original content (None for new files).
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub old_text: Option<String>,
    /// The new content after modification.
    pub new_text: String,
}

impl Diff {
    /// Builds [`Diff`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, new_text: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            old_text: None,
            new_text: new_text.into(),
        }
    }

    /// The original content (None for new files).
    #[must_use]
    pub fn old_text(mut self, old_text: impl IntoOption<String>) -> Self {
        self.old_text = old_text.into_option();
        self
    }
}

/// A file location being accessed or modified by a tool.
///
/// Enables clients to implement "follow-along" features that track
/// which files the agent is working with in real-time.
#[serde_as]
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "acp/tool_call.ts")]
#[serde(rename_all = "camelCase")]
pub struct ToolCallLocation {
    /// The absolute file path being accessed or modified.
    pub path: PathBuf,
    /// Optional line number within the file.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub line: Option<u32>,
}

impl ToolCallLocation {
    /// Builds [`ToolCallLocation`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            line: None,
        }
    }

    /// Optional line number within the file.
    #[must_use]
    pub fn line(mut self, line: impl IntoOption<u32>) -> Self {
        self.line = line.into_option();
        self
    }
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    ToolCallId::export(config)?;
    ToolKind::export(config)?;
    ToolCallStatus::export(config)?;
    Content::export(config)?;
    Terminal::export(config)?;
    Diff::export(config)?;
    ToolCallContent::export(config)?;
    ToolCallLocation::export(config)?;
    ToolCallUpdateFields::export(config)?;
    ToolCall::export(config)?;
    ToolCallUpdate::export(config)?;
    Ok(())
}
