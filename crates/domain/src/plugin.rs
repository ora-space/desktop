use serde::{Deserialize, Serialize};

use crate::{DomainModelError, PluginId};

/// Represents the persisted eligibility gate for an installed plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginEnabledState {
    Enabled,
    Disabled,
}

impl PluginEnabledState {
    /// Returns whether lifecycle policy permits the plugin to be activated.
    pub fn is_enabled(self) -> bool {
        match self {
            Self::Enabled => true,
            Self::Disabled => false,
        }
    }

    /// Restores the enum from the constrained integer stored by SQLite.
    pub fn from_database_value(value: i64) -> Result<Self, DomainModelError> {
        match value {
            0 => Ok(Self::Disabled),
            1 => Ok(Self::Enabled),
            value => Err(DomainModelError::InvalidPluginEnabledState(value)),
        }
    }

    /// Converts the enum into the constrained integer stored by SQLite.
    pub fn database_value(self) -> i64 {
        match self {
            Self::Disabled => 0,
            Self::Enabled => 1,
        }
    }
}

/// Holds the durable user intent associated with one discovered plugin identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginState {
    pub plugin_id: PluginId,
    pub enabled: PluginEnabledState,
    pub created_at: i64,
    pub updated_at: i64,
}

impl PluginState {
    /// Reconstructs one complete durable plugin-state row.
    pub fn new(
        plugin_id: PluginId,
        enabled: PluginEnabledState,
        created_at: i64,
        updated_at: i64,
    ) -> Self {
        Self {
            plugin_id,
            enabled,
            created_at,
            updated_at,
        }
    }
}

/// Describes which environment variables a plugin process may read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum EnvPermission {
    /// The process cannot read any environment variable.
    #[default]
    Denied,
    /// The process may read exactly these variables (`--allow-env=A,B`).
    Variables(Vec<String>),
    /// The process may read the whole environment (`--allow-env`).
    All,
}

/// Holds the Deno permissions a plugin declares in its manifest and the host grants at launch.
///
/// Declared permissions are the only ones a plugin process receives; the host no longer hands
/// every plugin the same fixed set. A missing declaration means the plugin runs fully sandboxed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PluginPermissions {
    /// `--allow-run`: spawn child processes, such as an external agent CLI.
    pub run: bool,
    /// `--allow-read`: read the filesystem beyond the module graph Deno already loads.
    pub read: bool,
    /// Environment variables the process may read.
    pub env: EnvPermission,
    /// `--allow-net`: open network connections from the plugin process itself.
    pub net: bool,
}

impl PluginPermissions {
    /// Renders the declared permissions as Deno CLI flags, in a stable order.
    pub fn deno_flags(&self) -> Vec<String> {
        let mut flags = Vec::new();
        if self.run {
            flags.push("--allow-run".to_string());
        }
        if self.read {
            flags.push("--allow-read".to_string());
        }
        match &self.env {
            EnvPermission::Denied => {}
            EnvPermission::Variables(names) if names.is_empty() => {}
            EnvPermission::Variables(names) => {
                flags.push(format!("--allow-env={}", names.join(",")));
            }
            EnvPermission::All => flags.push("--allow-env".to_string()),
        }
        if self.net {
            flags.push("--allow-net".to_string());
        }
        flags
    }
}
