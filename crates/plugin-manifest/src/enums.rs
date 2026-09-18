use std::{fmt, str::FromStr};
use thiserror::Error;

/// Identifies the closed set of plugin kinds supported by resolver version 1.
///
/// `Hook` is a processless contribution: its package carries one immutable Hook Configuration
/// and one package-contained executable, but the host never starts a Deno runtime for it. An
/// installed Hook is globally available; its lifecycle runtime stays `stopped`. `Pack` is a
/// marketplace-only listing that names a set of member plugins; it never becomes an installed
/// package and never carries a release of its own.
///
/// `Workflow` is processless and declares no section of its own. Its package is a delivery
/// vehicle for workflow documents under `assets/workflows/`, which the host imports into the
/// workflow library as user data rather than executing as plugin code. Because an imported
/// workflow outlives the package that carried it, uninstalling the plugin never removes it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PluginKind {
    Workbench,
    Agent,
    Webview,
    Skill,
    Mcp,
    Hook,
    Pack,
    Workflow,
}

impl PluginKind {
    /// Reports whether a package of this kind may ship a target-specific native executable.
    ///
    /// Only these kinds may declare `[[targets]]` on a release or `[artifact]` on an installed
    /// package, because only they contain a binary whose host compatibility the host must check
    /// before download. A Hook *is* that binary; an Agent may bundle the CLI it drives instead of
    /// requiring the user to install one, which is why the section stays optional for an Agent
    /// while a Hook cannot prove compatibility without it.
    pub fn may_ship_targeted_artifact(self) -> bool {
        match self {
            Self::Hook | Self::Agent => true,
            Self::Workbench
            | Self::Webview
            | Self::Skill
            | Self::Mcp
            | Self::Pack
            | Self::Workflow => false,
        }
    }

    /// Returns the manifest spelling of this plugin kind.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Workbench => "workbench",
            Self::Agent => "agent",
            Self::Webview => "webview",
            Self::Skill => "skill",
            Self::Mcp => "mcp",
            Self::Hook => "hook",
            Self::Pack => "pack",
            Self::Workflow => "workflow",
        }
    }
}

impl fmt::Display for PluginKind {
    /// Writes the manifest spelling of this plugin kind.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for PluginKind {
    type Err = PluginKindError;

    /// Parses a plugin kind without accepting future values under resolver version 1.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "workbench" => Ok(Self::Workbench),
            "agent" => Ok(Self::Agent),
            "webview" => Ok(Self::Webview),
            "skill" => Ok(Self::Skill),
            "mcp" => Ok(Self::Mcp),
            "hook" => Ok(Self::Hook),
            "pack" => Ok(Self::Pack),
            "workflow" => Ok(Self::Workflow),
            found => Err(PluginKindError::Unsupported {
                found: found.to_owned(),
            }),
        }
    }
}

/// Reports an unsupported plugin kind spelling.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum PluginKindError {
    #[error("unsupported plugin kind {found:?}")]
    Unsupported { found: String },
}
