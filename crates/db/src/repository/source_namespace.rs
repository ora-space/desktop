use rusqlite::params;

use crate::DatabaseError;
use crate::repository::RepositoryPool;

/// One persisted pairing of a marketplace source's canonical URL with its namespace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginSourceNamespaceBinding {
    pub canonical_url: String,
    pub namespace: String,
}

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

    /// Lists every binding ever recorded, including those whose source configuration was removed.
    ///
    /// Display labels are disambiguated against this whole set rather than the configured sources,
    /// because a plugin installed from a removed source is still listed and must keep a label that
    /// differs from its namesakes.
    pub fn list_bindings(&self) -> Result<Vec<PluginSourceNamespaceBinding>, DatabaseError> {
        self.pool.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT canonical_url, namespace FROM plugin_source_namespace
                 ORDER BY canonical_url",
            )?;
            let bindings = statement
                .query_map([], |row| {
                    Ok(PluginSourceNamespaceBinding {
                        canonical_url: row.get("canonical_url")?,
                        namespace: row.get("namespace")?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(bindings)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{PluginSourceNamespaceBinding, SqlitePluginSourceNamespaceRepository};
    use crate::{
        DatabaseBootstrapper, DatabaseLocation, default_migration_catalog, test_clock::TestClock,
    };
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    /// Opens a migrated repository in a temporary database.
    fn repository() -> (TempDir, SqlitePluginSourceNamespaceRepository) {
        let temp_dir = TempDir::new().expect("create temp directory");
        let pool = DatabaseBootstrapper::new(TestClock::new(1))
            .bootstrap_repository_pool(
                &DatabaseLocation::path(temp_dir.path().join("namespaces.sqlite3")),
                &default_migration_catalog().expect("build migration catalog"),
            )
            .expect("bootstrap database");
        (temp_dir, SqlitePluginSourceNamespaceRepository::new(pool))
    }

    /// Every binding is listed in a stable order, and a repeated bind adds no second row.
    #[test]
    fn lists_every_binding_once_in_url_order() {
        let (_temp_dir, repository) = repository();
        repository
            .bind("https://gitlab.com/acme/tools", "tools.22222222", 10)
            .expect("bind");
        repository
            .bind("https://github.com/acme/tools", "tools.11111111", 11)
            .expect("bind");
        repository
            .bind("https://gitlab.com/acme/tools", "tools.33333333", 12)
            .expect("rebind");

        assert_eq!(
            repository.list_bindings().expect("list"),
            vec![
                PluginSourceNamespaceBinding {
                    canonical_url: "https://github.com/acme/tools".to_string(),
                    namespace: "tools.11111111".to_string(),
                },
                PluginSourceNamespaceBinding {
                    canonical_url: "https://gitlab.com/acme/tools".to_string(),
                    namespace: "tools.22222222".to_string(),
                },
            ],
        );
    }
}
