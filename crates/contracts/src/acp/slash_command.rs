use super::common::Meta;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A user-invokable slash command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/slash_command.ts")]
pub struct AvailableCommand {
    /// The command name without a slash.
    pub name: String,
    /// The command description.
    pub description: String,
    /// Optional command input details.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub input: Option<AvailableCommandInput>,
    /// Optional extension data.
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    #[ts(type = "Record<string, unknown>")]
    #[ts(optional)]
    pub meta: Option<Meta>,
}

/// Input details for a slash command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/slash_command.ts")]
pub struct AvailableCommandInput {
    /// The prompt shown before input is provided.
    pub hint: String,
    /// Optional extension data.
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    #[ts(type = "Record<string, unknown>")]
    #[ts(optional)]
    pub meta: Option<Meta>,
}
