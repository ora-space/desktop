//! Compiles one Hook Plugin's `assets/config.json` — the immutable Hook Protocol descriptor and
//! optional Settings subset — into a strongly typed Hook Configuration.
//!
//! The compiled value is static install-time truth only: it proves the descriptor is legal, not
//! that a future Agent Plugin will consume it or that the executable starts. Resolution against a
//! running process is a later, separate step and is deliberately not modeled here.

#[cfg(test)]
mod tests;

use crate::declaration::{
    CompileDeclarationError, CompiledDeclaration, MAX_DECLARATION_BYTES,
    compile_declaration_from_value, parse_strict_json,
};
use ora_utils::path::PortableRelativePath;
use semver::Version;
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

/// Reports a Hook Configuration that cannot be compiled without ambiguity.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CompileHookConfigurationError {
    #[error(transparent)]
    Declaration(#[from] CompileDeclarationError),
    #[error("Hook configuration does not match schema version one: {0}")]
    InvalidStructure(String),
    #[error("unsupported Hook configuration schema version {0}")]
    UnsupportedSchemaVersion(u32),
    #[error("unsupported Hook protocol `{0}`")]
    UnsupportedProtocol(String),
    #[error("invalid Hook protocol descriptor `{field}`: {reason}")]
    InvalidDescriptor { field: String, reason: String },
    #[error(
        "invalid Setting `{setting_id}`: type `{found}` is not supported by Hook configuration schema version one"
    )]
    UnsupportedSettingType { setting_id: String, found: String },
}

/// Holds one validated Hook Configuration compiled from `assets/config.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledHookConfiguration {
    pub schema_version: u32,
    /// The user-facing Settings subset, absent when the package declares no Settings.
    pub settings: Option<CompiledDeclaration>,
    pub hook: HookDescriptor,
}

/// Holds the validated, versioned Hook Protocol descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookDescriptor {
    pub protocol: HookProtocol,
    /// Package-relative executable path under `assets/`; filesystem containment is re-checked by
    /// the package validator that owns the package root.
    pub executable: PortableRelativePath,
    pub command: HookCommand,
    /// Embedded tool version, independent from the Hook Plugin version.
    pub tool_version: Version,
}

/// Enumerates the closed set of supported Hook Protocols.
///
/// A protocol is a versioned, strongly typed contract that identifies how an Agent Plugin
/// integrates a Hook Plugin. RTK uses `rtk-rewrite-v1`; future protocols are added here as
/// explicit variants rather than accepting arbitrary strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookProtocol {
    /// Invokes `rtk rewrite` and preserves the command decision represented by exit status and
    /// output. The descriptor reports the embedded RTK tool version independently.
    RtkRewriteV1,
}

impl HookProtocol {
    /// Parses one protocol string into its closed enum variant.
    pub fn parse(value: &str) -> Result<Self, CompileHookConfigurationError> {
        match value {
            "rtk-rewrite-v1" => Ok(Self::RtkRewriteV1),
            found => Err(CompileHookConfigurationError::UnsupportedProtocol(
                found.to_owned(),
            )),
        }
    }

    /// Returns the canonical protocol spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RtkRewriteV1 => "rtk-rewrite-v1",
        }
    }
}

/// Holds the validated bare command alias through which an Agent Plugin may expose a Hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookCommand(String);

impl HookCommand {
    /// Parses a normalized bare command alias, rejecting path separators and emptiness so PATH
    /// resolution can never silently select the wrong Hook.
    pub fn parse(value: &str) -> Result<Self, CompileHookConfigurationError> {
        if value.is_empty() {
            return Err(CompileHookConfigurationError::InvalidDescriptor {
                field: "hook.command".to_string(),
                reason: "command must not be empty".to_string(),
            });
        }
        // A command alias is a bare name: a path separator would let a Hook masquerade as an
        // arbitrary filesystem path and break deterministic PATH resolution.
        if value.contains('/') || value.contains('\\') {
            return Err(CompileHookConfigurationError::InvalidDescriptor {
                field: "hook.command".to_string(),
                reason: "command must not contain a path separator".to_string(),
            });
        }
        // Command names are produced by build scripts and consumed verbatim, so control
        // characters, whitespace, or non-ASCII bytes are packaging mistakes.
        if value.chars().any(|character| {
            character.is_control() || character.is_whitespace() || !character.is_ascii()
        }) {
            return Err(CompileHookConfigurationError::InvalidDescriptor {
                field: "hook.command".to_string(),
                reason: "command must contain ASCII text without whitespace or control characters"
                    .to_string(),
            });
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the canonical command spelling.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawHookConfiguration {
    schema_version: u32,
    #[serde(default)]
    settings: Option<Value>,
    hook: RawHookDescriptor,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawHookDescriptor {
    protocol: String,
    executable: String,
    command: String,
    tool_version: String,
}

/// Compiles one duplicate-free Hook configuration JSON value.
pub(crate) fn compile_hook_configuration(
    value: Value,
) -> Result<CompiledHookConfiguration, CompileHookConfigurationError> {
    let raw: RawHookConfiguration = serde_json::from_value(value)
        .map_err(|error| CompileHookConfigurationError::InvalidStructure(error.to_string()))?;
    if raw.schema_version != 1 {
        return Err(CompileHookConfigurationError::UnsupportedSchemaVersion(
            raw.schema_version,
        ));
    }
    let settings = raw.settings.map(compile_settings_subset).transpose()?;
    let hook = compile_hook_descriptor(raw.hook)?;

    Ok(CompiledHookConfiguration {
        schema_version: 1,
        settings,
        hook,
    })
}

/// Compiles the Settings member by delegating to the shared Settings-only declaration compiler.
fn compile_settings_subset(
    settings: Value,
) -> Result<CompiledDeclaration, CompileHookConfigurationError> {
    // Reserved spec types are rejected up front so the author reads the phase-one policy.
    if let Value::Object(entries) = &settings {
        for (setting_id, declaration) in entries {
            if let Some(found) = declaration.get("type").and_then(Value::as_str)
                && matches!(found, "secret" | "file" | "directory")
            {
                return Err(CompileHookConfigurationError::UnsupportedSettingType {
                    setting_id: setting_id.clone(),
                    found: found.to_owned(),
                });
            }
        }
    }
    let wrapped = serde_json::json!({
        "schemaVersion": 1,
        "settings": settings,
    });
    Ok(compile_declaration_from_value(wrapped)?)
}

/// Compiles the Hook Protocol descriptor fields in declaration order.
fn compile_hook_descriptor(
    raw: RawHookDescriptor,
) -> Result<HookDescriptor, CompileHookConfigurationError> {
    let protocol = HookProtocol::parse(&raw.protocol)?;
    let executable = PortableRelativePath::parse(&raw.executable).map_err(|error| {
        CompileHookConfigurationError::InvalidDescriptor {
            field: "hook.executable".to_string(),
            reason: format!("executable must be a safe relative path: {error}"),
        }
    })?;
    let command = HookCommand::parse(&raw.command)?;
    let tool_version = Version::parse(&raw.tool_version).map_err(|error| {
        CompileHookConfigurationError::InvalidDescriptor {
            field: "hook.toolVersion".to_string(),
            reason: format!("toolVersion must be a semantic version: {error}"),
        }
    })?;

    Ok(HookDescriptor {
        protocol,
        executable,
        command,
        tool_version,
    })
}

/// Compiles one Hook-shaped `assets/config.json` payload.
pub fn compile_hook_configuration_from_bytes(
    source: &[u8],
) -> Result<CompiledHookConfiguration, CompileHookConfigurationError> {
    if source.len() > MAX_DECLARATION_BYTES {
        return Err(CompileDeclarationError::TooLarge.into());
    }
    let value = parse_strict_json(source).map_err(CompileHookConfigurationError::Declaration)?;
    compile_hook_configuration(value)
}
