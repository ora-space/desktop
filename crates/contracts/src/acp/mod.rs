pub mod authentication;
pub mod common;
pub mod content;
pub mod error;
pub mod file;
pub mod initialization;
pub mod literals;
pub mod mcp;
pub mod notification;
pub mod permission;
pub mod plan;
pub mod prompt;
pub mod rpc;
pub mod serde_util;
pub mod session;
pub mod session_config_options;
pub mod session_mode;
pub mod slash_command;
pub mod terminal;
pub mod tool_call;

/// Exports every ACP TypeScript binding declared across the `acp` module family.
///
/// Each sub-module owns its own exhaustive binding list; this aggregates them so the
/// crate-level export entry point only needs a single `acp::export` call.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    common::export(config)?;
    content::export(config)?;
    authentication::export(config)?;
    initialization::export(config)?;
    session_config_options::export(config)?;
    session_mode::export(config)?;
    slash_command::export(config)?;
    plan::export(config)?;
    prompt::export(config)?;
    mcp::export(config)?;
    terminal::export(config)?;
    tool_call::export(config)?;
    session::export(config)?;
    notification::export(config)?;
    permission::export(config)?;
    file::export(config)?;
    Ok(())
}
