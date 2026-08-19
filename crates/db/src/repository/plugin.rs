use ora_application::{PluginStateRepository, RepositoryError};
use ora_domain::{PluginEnabledState, PluginId, PluginState};
use rusqlite::{Row, params};

use crate::repository::RepositoryPool;

/// Persists the durable eligibility gate for discovered plugins in SQLite.
#[derive(Clone, Debug)]
pub struct SqlitePluginStateRepository {
    pool: RepositoryPool,
}

impl SqlitePluginStateRepository {
    /// Builds a plugin-state repository from the shared repository pool.
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }
}

impl PluginStateRepository for SqlitePluginStateRepository {
    /// Loads one plugin-state row without treating missing state as an enabled default.
    fn find_plugin_state(
        &self,
        plugin_id: &PluginId,
    ) -> Result<Option<PluginState>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT plugin_id, enabled, created_at, updated_at
                     FROM plugin_state
                     WHERE plugin_id = ?1",
                )?;
                let mut rows = statement.query(params![plugin_id.as_ref()])?;

                match rows.next()? {
                    Some(row) => Ok(Some(map_plugin_state_row(row)?)),
                    None => Ok(None),
                }
            })
            .map_err(plugin_repository_error_from_database)
    }

    /// Lists durable state in identifier order so reconciliation remains deterministic.
    fn list_plugin_states(&self) -> Result<Vec<PluginState>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT plugin_id, enabled, created_at, updated_at
                     FROM plugin_state
                     ORDER BY plugin_id",
                )?;
                let mut rows = statement.query([])?;
                let mut states = Vec::new();

                while let Some(row) = rows.next()? {
                    states.push(map_plugin_state_row(row)?);
                }

                Ok(states)
            })
            .map_err(plugin_repository_error_from_database)
    }

    /// Upserts only lifecycle intent so package metadata remains filesystem-derived.
    fn set_plugin_enabled(
        &self,
        plugin_id: &PluginId,
        enabled: PluginEnabledState,
        now: i64,
    ) -> Result<PluginState, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "INSERT INTO plugin_state (plugin_id, enabled, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?3)
                     ON CONFLICT(plugin_id) DO UPDATE SET
                         enabled = excluded.enabled,
                         updated_at = excluded.updated_at
                     RETURNING plugin_id, enabled, created_at, updated_at",
                )?;
                let mut rows =
                    statement.query(params![plugin_id.as_ref(), enabled.database_value(), now])?;

                match rows.next()? {
                    Some(row) => map_plugin_state_row(row),
                    None => Err(crate::DatabaseError::Sqlite(
                        rusqlite::Error::QueryReturnedNoRows,
                    )),
                }
            })
            .map_err(plugin_repository_error_from_database)
    }

    /// Physically removes state because missing packages must not leave lifecycle tombstones.
    fn delete_plugin_state(&self, plugin_id: &PluginId) -> Result<bool, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let deleted_rows = connection.execute(
                    "DELETE FROM plugin_state WHERE plugin_id = ?1",
                    params![plugin_id.as_ref()],
                )?;

                Ok(deleted_rows > 0)
            })
            .map_err(plugin_repository_error_from_database)
    }
}

/// Restores the durable enum so corrupt integer values cannot enter lifecycle orchestration.
fn map_plugin_state_row(row: &Row<'_>) -> Result<PluginState, crate::DatabaseError> {
    Ok(PluginState::new(
        PluginId::new(row.get::<_, String>("plugin_id")?),
        PluginEnabledState::from_database_value(row.get("enabled")?)?,
        row.get("created_at")?,
        row.get("updated_at")?,
    ))
}

/// Preserves the concrete database failure behind the application-owned repository error.
fn plugin_repository_error_from_database(error: crate::DatabaseError) -> RepositoryError {
    RepositoryError::new(error)
}
