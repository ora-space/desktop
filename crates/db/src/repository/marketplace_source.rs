use rusqlite::{Row, params};

use crate::DatabaseError;
use crate::repository::{RepositoryPool, connection::bool_to_sqlite};

/// One durable plugin marketplace source row, ordered by the user-visible position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginMarketplaceSourceRecord {
    /// HTTPS Git repository URL. Duplicate-free primary key.
    pub url: String,
    /// Short branch name tracked by the source.
    pub branch: String,
    /// Whether network operations for this source should use the configured proxy.
    pub use_proxy: bool,
    /// Stable ordering position used to resolve duplicate plugin ids across sources.
    pub position: i64,
}

/// Persists the user-editable plugin marketplace source list in SQLite.
#[derive(Clone, Debug)]
pub struct SqlitePluginMarketplaceSourceRepository {
    pool: RepositoryPool,
}

impl SqlitePluginMarketplaceSourceRepository {
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }

    /// Lists every configured source in precedence order.
    pub fn list_sources(&self) -> Result<Vec<PluginMarketplaceSourceRecord>, DatabaseError> {
        self.pool.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT url, branch, use_proxy, position
                 FROM plugin_marketplace_source
                 ORDER BY position",
            )?;
            let mut rows = statement.query([])?;
            let mut sources = Vec::new();

            while let Some(row) = rows.next()? {
                sources.push(map_marketplace_source_row(row)?);
            }

            Ok(sources)
        })
    }

    /// Inserts one source at the supplied precedence position.
    pub fn insert_source(
        &self,
        record: &PluginMarketplaceSourceRecord,
        now_ms: i64,
    ) -> Result<(), DatabaseError> {
        self.pool.with_connection(|connection| {
            connection.execute(
                "INSERT INTO plugin_marketplace_source (
                    url, branch, use_proxy, position, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                params![
                    record.url.as_str(),
                    record.branch.as_str(),
                    bool_to_sqlite(record.use_proxy),
                    record.position,
                    now_ms,
                ],
            )?;
            Ok(())
        })
    }

    /// Updates only the proxy policy for one source and returns whether the row existed.
    pub fn set_use_proxy(
        &self,
        url: &str,
        use_proxy: bool,
        now_ms: i64,
    ) -> Result<bool, DatabaseError> {
        self.pool.with_connection(|connection| {
            let updated = connection.execute(
                "UPDATE plugin_marketplace_source
                 SET use_proxy = ?1, updated_at = ?2
                 WHERE url = ?3",
                params![bool_to_sqlite(use_proxy), now_ms, url],
            )?;
            Ok(updated > 0)
        })
    }

    /// Removes one source by URL and returns whether the row existed.
    pub fn delete_source(&self, url: &str) -> Result<bool, DatabaseError> {
        self.pool.with_connection(|connection| {
            let deleted = connection.execute(
                "DELETE FROM plugin_marketplace_source WHERE url = ?1",
                params![url],
            )?;
            Ok(deleted > 0)
        })
    }
}

fn map_marketplace_source_row(
    row: &Row<'_>,
) -> Result<PluginMarketplaceSourceRecord, DatabaseError> {
    Ok(PluginMarketplaceSourceRecord {
        url: row.get("url")?,
        branch: row.get("branch")?,
        use_proxy: row.get::<_, i64>("use_proxy")? != 0,
        position: row.get("position")?,
    })
}
