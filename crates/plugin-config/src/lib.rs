//! Compiles and persists host-owned Plugin Configuration.

mod declaration;
mod filesystem;
mod hook;
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
pub use hook::{
    CompileHookConfigurationError, CompiledHookConfiguration, HookCommand, HookDescriptor,
    HookProtocol, compile_hook_configuration_from_bytes,
};
pub use mcp::{
    CompileConfigurationFileError, CompileMcpConfigurationError, CompiledConfigurationFile,
    CompiledMcpConfiguration, MCP_COMMAND_DIRECTORY, McpArgument, McpHttpTransport,
    McpStdioTransport, McpTransport, McpValueExpression, ResolveMcpBindingError,
    ResolvedMcpArgument, ResolvedMcpTransport, compile_configuration_file, resolve_mcp_transport,
};
pub use service::{
    ConfigurationCompleteness, ConfigurationDetails, ConfigurationError, ConfigurationFieldError,
    ConfigurationService, ConfigurationSummary, EffectiveValueSource, SettingDetails,
    recovery_backup_label,
};
