use serde::{Deserialize, Serialize};
use serde_with::{DefaultOnError, serde_as, skip_serializing_none};
use ts_rs::TS;

use super::serde_util::IntoOption;

/// A user-invokable slash command.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/slash_command.ts")]
pub struct AvailableCommand {
    /// Command name (e.g., `create_plan`, `research_codebase`).
    pub name: String,
    /// Human-readable description of what the command does.
    pub description: String,
    /// Input for the command if required
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub input: Option<AvailableCommandInput>,
}

impl AvailableCommand {
    /// Builds [`AvailableCommand`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input: None,
        }
    }

    /// Input for the command if required
    #[must_use]
    pub fn input(mut self, input: impl IntoOption<AvailableCommandInput>) -> Self {
        self.input = input.into_option();
        self
    }
}

/// Input details for a slash command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(untagged, rename_all = "camelCase")]
#[ts(export_to = "acp/slash_command.ts")]
pub enum AvailableCommandInput {
    /// All text that was typed after the command name is provided as input.
    Unstructured(UnstructuredCommandInput),
}

/// All text that was typed after the command name is provided as input.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/slash_command.ts")]
pub struct UnstructuredCommandInput {
    /// A hint to display when the input hasn't been provided yet
    pub hint: String,
}

impl UnstructuredCommandInput {
    /// Builds [`UnstructuredCommandInput`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(hint: impl Into<String>) -> Self {
        Self { hint: hint.into() }
    }
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    AvailableCommand::export(config)?;
    AvailableCommandInput::export(config)?;
    UnstructuredCommandInput::export(config)?;
    Ok(())
}
