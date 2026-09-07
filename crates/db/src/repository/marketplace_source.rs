use std::fmt;

use rusqlite::{Row, params};

use crate::DatabaseError;
use crate::repository::{RepositoryPool, connection::bool_to_sqlite};

/// One durable plugin marketplace source row, ordered by the user-visible position.
#[derive(Clone, PartialEq, Eq)]
pub struct PluginMarketplaceSourceRecord {
    /// HTTPS Git repository URL. Duplicate-free primary key.
    pub url: String,
    /// Short branch name tracked by the source.
    pub branch: String,
    /// Whether network operations for this source should use the configured proxy.
    pub use_proxy: bool,
    /// Whether this source participates in marketplace sync, listing, and install.
    pub enabled: bool,
    /// Tagged JSON describing direct HTTPS or source-scoped S3 SigV4 artifact retrieval.
    pub artifact_retrieval: String,
    /// Stable ordering position used to resolve duplicate plugin ids across sources.
    pub position: i64,
}

impl fmt::Debug for PluginMarketplaceSourceRecord {
    /// Redacts the retrieval JSON because its S3 variant contains persisted credentials.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PluginMarketplaceSourceRecord")
            .field("url", &self.url)
            .field("branch", &self.branch)
            .field("use_proxy", &self.use_proxy)
            .field("enabled", &self.enabled)
            .field("artifact_retrieval", &"[redacted]")
            .field("position", &self.position)
            .finish()
    }
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
                "SELECT url, branch, use_proxy, enabled, artifact_retrieval, position
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
                    url, branch, use_proxy, enabled, artifact_retrieval, position, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![
                    record.url.as_str(),
                    record.branch.as_str(),
                    bool_to_sqlite(record.use_proxy),
                    bool_to_sqlite(record.enabled),
                    record.artifact_retrieval.as_str(),
                    record.position,
                    now_ms,
                ],
            )?;
            Ok(())
        })
    }

    /// Replaces the editable fields of one source and returns whether the row existed.
    ///
    /// `url` identifies the current row. `record.url` may differ when the user edits the Git
    /// address; SQLite updates the primary key in place so position and identity stay on the row.
    pub fn update_source(
        &self,
        url: &str,
        record: &PluginMarketplaceSourceRecord,
        now_ms: i64,
    ) -> Result<bool, DatabaseError> {
        self.pool.with_connection(|connection| {
            let updated = connection.execute(
                "UPDATE plugin_marketplace_source
                 SET url = ?1, branch = ?2, use_proxy = ?3, enabled = ?4,
                     artifact_retrieval = ?5, updated_at = ?6
                 WHERE url = ?7",
                params![
                    record.url.as_str(),
                    record.branch.as_str(),
                    bool_to_sqlite(record.use_proxy),
                    bool_to_sqlite(record.enabled),
                    record.artifact_retrieval.as_str(),
                    now_ms,
                    url,
                ],
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
        enabled: row.get::<_, i64>("enabled")? != 0,
        artifact_retrieval: row.get("artifact_retrieval")?,
        position: row.get("position")?,
    })
}
