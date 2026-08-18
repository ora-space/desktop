use crate::RepositoryError;
use ora_domain::{PluginEnabledState, PluginId, PluginState};

/// Persists user-controlled plugin eligibility independently from filesystem discovery.
///
/// Implementations must preserve the original creation timestamp when an existing row is
/// updated and must not infer plugin identity from storage rows.
pub trait PluginStateRepository {
    /// Loads the optional durable state for one plugin identifier.
    fn find_plugin_state(
        &self,
        plugin_id: &PluginId,
    ) -> Result<Option<PluginState>, RepositoryError>;

    /// Lists every durable plugin-state row in stable identifier order.
    fn list_plugin_states(&self) -> Result<Vec<PluginState>, RepositoryError>;

    /// Creates or updates the durable eligibility gate and returns the complete row.
    fn set_plugin_enabled(
        &self,
        plugin_id: &PluginId,
        enabled: PluginEnabledState,
        now: i64,
    ) -> Result<PluginState, RepositoryError>;

    /// Physically removes one state row and reports whether it existed.
    fn delete_plugin_state(&self, plugin_id: &PluginId) -> Result<bool, RepositoryError>;
}
