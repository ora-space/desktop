mod content;
mod prompt_turn;
mod session_delete;
mod tool_calls;

#[cfg(test)]
mod tests;

pub use content::{
    Annotations, AudioContent, BlobResourceContents, ContentBlock, EmbeddedResourceContents,
    ImageContent, ResourceLink, Role, TextContent, TextResourceContents,
};
pub use prompt_turn::{
    AgentMessageChunk, Cost, PlanEntry, PlanEntryPriority, PlanEntryStatus, PlanUpdate,
    SessionCancelNotification, SessionPromptRequest, SessionPromptResponse, SessionUpdate,
    SessionUpdateNotification, StopReason, UsageUpdate,
};
pub use session_delete::{SessionDeleteRequest, SessionDeleteResponse};
pub use tool_calls::{
    ContentToolCallContent, DiffToolCallContent, PermissionOption, PermissionOptionKind,
    SessionRequestPermissionRequest, SessionRequestPermissionResponse, TerminalToolCallContent,
    ToolCall, ToolCallContent, ToolCallLocation, ToolCallStatus, ToolCallUpdate, ToolKind,
    ToolPermissionOutcome,
};

use ts_rs::{Config, ExportError, TS};

/// Exports all ACP DTOs into their shared TypeScript files.
pub(crate) fn export_typescript_bindings(config: &Config) -> Result<(), ExportError> {
    Role::export(config)?;
    Annotations::export(config)?;
    TextContent::export(config)?;
    ImageContent::export(config)?;
    AudioContent::export(config)?;
    TextResourceContents::export(config)?;
    BlobResourceContents::export(config)?;
    EmbeddedResourceContents::export(config)?;
    ResourceLink::export(config)?;
    ContentBlock::export(config)?;

    ToolKind::export(config)?;
    ToolCallStatus::export(config)?;
    ContentToolCallContent::export(config)?;
    DiffToolCallContent::export(config)?;
    TerminalToolCallContent::export(config)?;
    ToolCallContent::export(config)?;
    ToolCallLocation::export(config)?;
    ToolCall::export(config)?;
    ToolCallUpdate::export(config)?;
    PermissionOptionKind::export(config)?;
    PermissionOption::export(config)?;
    ToolPermissionOutcome::export(config)?;
    SessionRequestPermissionRequest::export(config)?;
    SessionRequestPermissionResponse::export(config)?;

    StopReason::export(config)?;
    SessionPromptRequest::export(config)?;
    SessionPromptResponse::export(config)?;
    SessionCancelNotification::export(config)?;
    PlanEntryPriority::export(config)?;
    PlanEntryStatus::export(config)?;
    PlanEntry::export(config)?;
    PlanUpdate::export(config)?;
    AgentMessageChunk::export(config)?;
    Cost::export(config)?;
    UsageUpdate::export(config)?;
    SessionUpdate::export(config)?;
    SessionUpdateNotification::export(config)?;

    SessionDeleteRequest::export(config)?;
    SessionDeleteResponse::export(config)?;

    Ok(())
}
