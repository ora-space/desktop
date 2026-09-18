//! Compiles one Hook Plugin's `assets/config.json` — the package-contained executable, the
//! optional `supportedAgents` list, and the lifecycle commands the host may execute — into a
//! strongly typed Hook Configuration.
//!
//! The compiled value is static install-time truth only: it proves the declaration is legal and
//! names a package-relative executable, not that the executable can run or that it will find any
//! Agent on this machine. Executing a lifecycle command is a later, separate step owned by the
//! backend, which re-validates the executable against the installed package before every spawn.

#[cfg(test)]
mod tests;

use crate::declaration::{
    CompileDeclarationError, CompiledDeclaration, MAX_DECLARATION_BYTES,
    compile_declaration_from_value, parse_strict_json,
};
use ora_utils::Slug;
use ora_utils::path::PortableRelativePath;
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

/// Maximum number of arguments one lifecycle phase may declare.
const MAX_LIFECYCLE_ARGS: usize = 16;
/// Maximum byte length of one lifecycle argument.
const MAX_LIFECYCLE_ARG_BYTES: usize = 512;
/// Maximum number of Agent identifiers one Hook may advertise.
const MAX_SUPPORTED_AGENTS: usize = 16;

/// Descriptor members removed when the Hook declaration converged to executable + lifecycle.
///
/// They are reported by name rather than as unknown fields: a package written for the previous
/// shape must be repackaged, and serde's generic "unknown field" list cannot say that.
const REMOVED_DESCRIPTOR_FIELDS: [&str; 3] = ["protocol", "command", "toolVersion"];

/// Reports a Hook Configuration that cannot be compiled without ambiguity.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CompileHookConfigurationError {
    #[error(transparent)]
    Declaration(#[from] CompileDeclarationError),
    #[error("Hook configuration does not match schema version one: {0}")]
    InvalidStructure(String),
    #[error("unsupported Hook configuration schema version {0}")]
    UnsupportedSchemaVersion(u32),
    #[error(
        "Hook descriptor field `{field}` was removed: repackage the plugin with `executable`, an optional `supportedAgents`, and `lifecycle`"
    )]
    RemovedDescriptorField { field: String },
    #[error("invalid Hook descriptor `{field}`: {reason}")]
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

/// Holds the validated Hook declaration: which package-contained program to run, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookDescriptor {
    /// Package-relative executable path under `assets/`; filesystem containment is re-checked by
    /// the package validator that owns the package root, and again before every execution.
    pub executable: PortableRelativePath,
    /// Agent identifiers the author claims the tool supports. Display only: the host validates
    /// each identifier's shape and nothing else, and never matches them against Agents it knows.
    pub supported_agents: Vec<Slug>,
    pub lifecycle: HookLifecycle,
}

/// Holds the lifecycle commands a Hook Plugin declares for the host to execute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookLifecycle {
    /// Runs after install, local import and update, and on explicit user initialization.
    pub init: HookLifecycleCommand,
    /// Runs before uninstall when the tool provides it. Absent means the host executes nothing on
    /// uninstall and reports that whatever the tool wrote stays in place.
    pub deinit: Option<HookLifecycleCommand>,
}

/// Holds the validated fixed arguments of one lifecycle phase.
///
/// The host passes these arguments verbatim and never composes its own: the tool decides which
/// flag means "register" and which means "revoke", so the host needs no per-tool knowledge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookLifecycleCommand {
    args: Vec<String>,
}

impl HookLifecycleCommand {
    /// Validates one phase's declared argument list, rejecting values that cannot be passed to a
    /// process safely or would be invisible in a diagnostic record.
    pub fn parse(field: &str, args: Vec<String>) -> Result<Self, CompileHookConfigurationError> {
        if args.len() > MAX_LIFECYCLE_ARGS {
            return Err(CompileHookConfigurationError::InvalidDescriptor {
                field: field.to_string(),
                reason: format!("at most {MAX_LIFECYCLE_ARGS} arguments are allowed"),
            });
        }
        // Arguments are produced by packaging scripts and handed to the operating system without
        // a shell, so an empty or control-bearing value is always a packaging mistake. Control
        // characters in particular would corrupt the very log lines used to diagnose a failure.
        for (index, arg) in args.iter().enumerate() {
            let field = format!("{field}[{index}]");
            if arg.is_empty() {
                return Err(CompileHookConfigurationError::InvalidDescriptor {
                    field,
                    reason: "argument must not be empty".to_string(),
                });
            }
            if arg.len() > MAX_LIFECYCLE_ARG_BYTES {
                return Err(CompileHookConfigurationError::InvalidDescriptor {
                    field,
                    reason: format!("argument must be at most {MAX_LIFECYCLE_ARG_BYTES} bytes"),
                });
            }
            if arg.chars().any(char::is_control) {
                return Err(CompileHookConfigurationError::InvalidDescriptor {
                    field,
                    reason: "argument must not contain control characters".to_string(),
                });
            }
        }
        Ok(Self { args })
    }

    /// Returns the arguments in declaration order.
    pub fn args(&self) -> &[String] {
        &self.args
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
    executable: String,
    #[serde(default)]
    supported_agents: Vec<String>,
    lifecycle: RawHookLifecycle,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawHookLifecycle {
    init: RawLifecycleCommand,
    #[serde(default)]
    deinit: Option<RawLifecycleCommand>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawLifecycleCommand {
    /// Omitted arguments mean the executable is invoked with no arguments for that phase.
    #[serde(default)]
    args: Vec<String>,
}

/// Compiles one duplicate-free Hook configuration JSON value.
pub(crate) fn compile_hook_configuration(
    value: Value,
) -> Result<CompiledHookConfiguration, CompileHookConfigurationError> {
    // Removed members are detected before structural parsing so a package written for the
    // previous shape gets one actionable error instead of serde's unknown-field list.
    reject_removed_descriptor_fields(&value)?;
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

/// Rejects a Hook declaration written for the shape that this decision replaced.
///
/// The removed members are probed in a fixed order so the same package always reports the same
/// field regardless of how the author ordered their keys.
fn reject_removed_descriptor_fields(value: &Value) -> Result<(), CompileHookConfigurationError> {
    let Some(descriptor) = value.get("hook").and_then(Value::as_object) else {
        return Ok(());
    };
    for field in REMOVED_DESCRIPTOR_FIELDS {
        if descriptor.contains_key(field) {
            return Err(CompileHookConfigurationError::RemovedDescriptorField {
                field: format!("hook.{field}"),
            });
        }
    }
    Ok(())
}

/// Compiles the Hook descriptor fields in declaration order.
fn compile_hook_descriptor(
    raw: RawHookDescriptor,
) -> Result<HookDescriptor, CompileHookConfigurationError> {
    let executable = PortableRelativePath::parse(&raw.executable).map_err(|error| {
        CompileHookConfigurationError::InvalidDescriptor {
            field: "hook.executable".to_string(),
            reason: format!("executable must be a safe relative path: {error}"),
        }
    })?;
    let supported_agents = compile_supported_agents(raw.supported_agents)?;
    let lifecycle = compile_lifecycle(raw.lifecycle)?;

    Ok(HookDescriptor {
        executable,
        supported_agents,
        lifecycle,
    })
}

/// Compiles the advertised Agent identifiers, rejecting duplicates so the displayed list cannot
/// claim the same Agent twice.
fn compile_supported_agents(
    agents: Vec<String>,
) -> Result<Vec<Slug>, CompileHookConfigurationError> {
    if agents.len() > MAX_SUPPORTED_AGENTS {
        return Err(CompileHookConfigurationError::InvalidDescriptor {
            field: "hook.supportedAgents".to_string(),
            reason: format!("at most {MAX_SUPPORTED_AGENTS} identifiers are allowed"),
        });
    }
    let mut compiled = Vec::with_capacity(agents.len());
    for (index, agent) in agents.iter().enumerate() {
        let slug = Slug::parse(agent).map_err(|error| {
            CompileHookConfigurationError::InvalidDescriptor {
                field: format!("hook.supportedAgents[{index}]"),
                reason: format!("identifier must be a lowercase slug: {error}"),
            }
        })?;
        if compiled.contains(&slug) {
            return Err(CompileHookConfigurationError::InvalidDescriptor {
                field: format!("hook.supportedAgents[{index}]"),
                reason: "duplicate Agent identifier".to_string(),
            });
        }
        compiled.push(slug);
    }
    Ok(compiled)
}

/// Compiles both lifecycle phases, keeping `init` mandatory and `deinit` optional.
fn compile_lifecycle(
    raw: RawHookLifecycle,
) -> Result<HookLifecycle, CompileHookConfigurationError> {
    let init = HookLifecycleCommand::parse("hook.lifecycle.init.args", raw.init.args)?;
    let deinit = raw
        .deinit
        .map(|command| HookLifecycleCommand::parse("hook.lifecycle.deinit.args", command.args))
        .transpose()?;

    Ok(HookLifecycle { init, deinit })
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
