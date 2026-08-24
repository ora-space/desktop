//! Compiles and persists host-owned Plugin Configuration.

mod declaration;
mod service;

pub use declaration::{
    CompileDeclarationError, CompiledDeclaration, MAX_DECLARATION_BYTES, MAX_SETTINGS,
    SettingDeclaration, SettingType, SettingValue, compile_declaration,
};
pub use service::{
    ConfigurationCompleteness, ConfigurationDetails, ConfigurationError, ConfigurationFieldError,
    ConfigurationFileSystem, ConfigurationService, ConfigurationSummary, EffectiveValueSource,
    SettingDetails, StandardConfigurationFileSystem,
};
