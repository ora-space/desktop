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
