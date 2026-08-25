//! Compiles and persists host-owned Plugin Configuration.

mod declaration;
mod filesystem;
mod mcp;
mod service;
mod values;
#[cfg(windows)]
mod windows_permissions;

pub use declaration::{
    CompileDeclarationError, CompiledDeclaration, MAX_DECLARATION_BYTES, MAX_SETTINGS,
    SettingDeclaration, SettingType, SettingValue, compile_declaration,
};
pub use filesystem::{ConfigurationFileSystem, StandardConfigurationFileSystem};
pub use mcp::{
    CompileConfigurationFileError, CompileMcpConfigurationError, CompiledConfigurationFile,
    CompiledMcpConfiguration, MCP_COMMAND_DIRECTORY, McpArgument, McpHttpTransport,
    McpStdioTransport, McpTransport, McpValueExpression, compile_configuration_file,
};
pub use service::{
    ConfigurationCompleteness, ConfigurationDetails, ConfigurationError, ConfigurationFieldError,
    ConfigurationService, ConfigurationSummary, EffectiveValueSource, SettingDetails,
    recovery_backup_label,
};
