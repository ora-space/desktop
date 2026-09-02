use rusqlite::params;

use crate::DatabaseError;
use crate::repository::RepositoryPool;

/// Persists the immutable namespace bound to each marketplace source's canonical URL.
///
/// There is deliberately no update and no delete. A namespace is frozen into the install path,
/// private data directory, `skills` rows, and Effect Consumer identity of every plugin installed
/// from that source, and none of those can be rewritten in place without stranding the rows the
/// old identity owns. So the binding is written once and only ever read afterwards: removing a
/// source leaves its binding behind, and adding the same repository back later resolves to the
/// identity its already-installed plugins still answer to.
#[derive(Clone, Debug)]
pub struct SqlitePluginSourceNamespaceRepository {
    pool: RepositoryPool,
}

impl SqlitePluginSourceNamespaceRepository {
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }

    /// Returns the namespace already bound to `canonical_url`, if one was ever bound.
    pub fn namespace_for(&self, canonical_url: &str) -> Result<Option<String>, DatabaseError> {
        self.pool.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT namespace FROM plugin_source_namespace WHERE canonical_url = ?1",
            )?;
            let mut rows = statement.query(params![canonical_url])?;
            match rows.next()? {
                Some(row) => Ok(Some(row.get("namespace")?)),
                None => Ok(None),
            }
        })
    }

    /// Binds `namespace` to `canonical_url` unless a binding already exists, and returns the
    /// namespace that is now in force.
    ///
    /// The conflicting insert rewrites the namespace to itself so the statement returns the row
    /// that survived, which is what makes the binding immutable under concurrency as well as over
    /// time: a second caller racing to bind the same URL reads back the winner's namespace in one
    /// atomic step instead of overwriting it or observing a gap between insert and read.
    pub fn bind(
        &self,
        canonical_url: &str,
        namespace: &str,
        now_ms: i64,
    ) -> Result<String, DatabaseError> {
        self.pool.with_connection(|connection| {
            Ok(connection.query_row(
                "INSERT INTO plugin_source_namespace (canonical_url, namespace, created_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(canonical_url) DO UPDATE SET namespace = namespace
                 RETURNING namespace",
                params![canonical_url, namespace, now_ms],
                |row| row.get("namespace"),
            )?)
        })
    }
}
