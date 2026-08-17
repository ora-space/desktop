use std::{fmt, str::FromStr};
use thiserror::Error;

/// Identifies the closed set of plugin source namespaces supported by resolver version 1.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PluginNamespace {
    Official,
}

impl PluginNamespace {
    /// Returns the manifest spelling of this namespace.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Official => "official",
        }
    }
}

impl fmt::Display for PluginNamespace {
    /// Writes the manifest spelling of this namespace.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for PluginNamespace {
    type Err = PluginNamespaceError;

    /// Parses a namespace without accepting future values under resolver version 1.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "official" => Ok(Self::Official),
            found => Err(PluginNamespaceError::Unsupported {
                found: found.to_owned(),
            }),
        }
    }
}

/// Reports an unsupported plugin namespace spelling.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum PluginNamespaceError {
    #[error("unsupported plugin namespace {found:?}")]
    Unsupported { found: String },
}

/// Identifies the closed set of plugin kinds supported by resolver version 1.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PluginKind {
    Workbench,
}

impl PluginKind {
    /// Returns the manifest spelling of this plugin kind.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Workbench => "workbench",
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
