use std::time::Duration;

use r2d2::{ManageConnection, Pool};
use rusqlite::Connection;

use crate::{DatabaseError, DatabaseLocation};

const BUSY_TIMEOUT_MILLIS: u64 = 5_000;

/// Shares configured SQLite connections across repository adapters.
#[derive(Clone, Debug)]
pub struct RepositoryPool {
    inner: Pool<SqliteConnectionManager>,
}

impl RepositoryPool {
    /// Builds a repository pool for a file-backed SQLite database location.
    pub fn new(location: &DatabaseLocation) -> Result<Self, DatabaseError> {
        let path = location.pooled_path()?.to_path_buf();
        let manager = SqliteConnectionManager::new(path);
        let inner = Pool::builder().build(manager)?;

        Ok(Self { inner })
    }

    /// Runs one repository operation with a configured pooled SQLite connection.
    pub(crate) fn with_connection<T>(
        &self,
        operation: impl FnOnce(&Connection) -> Result<T, crate::DatabaseError>,
    ) -> Result<T, DatabaseError> {
        let connection = self.inner.get()?;

        operation(&connection)
    }
}

/// Opens and validates SQLite connections for the repository pool.
#[derive(Clone, Debug)]
struct SqliteConnectionManager {
    path: std::path::PathBuf,
}

impl SqliteConnectionManager {
    /// Captures the file-backed database path used for pooled connections.
    fn new(path: std::path::PathBuf) -> Self {
        Self { path }
    }
}

impl ManageConnection for SqliteConnectionManager {
    type Connection = Connection;
    type Error = rusqlite::Error;

    /// Opens a SQLite connection and applies the shared repository PRAGMAs.
    fn connect(&self) -> Result<Self::Connection, Self::Error> {
        let connection = Connection::open(&self.path)?;

        configure_repository_connection(&connection)?;

        Ok(connection)
    }

    /// Verifies pooled SQLite connections can still execute a trivial query.
    fn is_valid(&self, connection: &mut Self::Connection) -> Result<(), Self::Error> {
        connection.execute_batch("SELECT 1;")
    }

    /// Treats pooled SQLite connections as healthy unless checkout already failed.
    fn has_broken(&self, _connection: &mut Self::Connection) -> bool {
        false
    }
}

/// Applies the SQLite runtime settings required by the repository adapters.
fn configure_repository_connection(connection: &Connection) -> Result<(), rusqlite::Error> {
    // These PRAGMAs are centralized here so every pooled connection uses the same
    // concurrency and durability profile instead of relying on repository call sites.
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MILLIS))?;
    connection.pragma_update(None, "synchronous", "NORMAL")?;

    Ok(())
}

/// Encodes a Rust boolean into the integer representation used by the schema.
pub(crate) fn bool_to_sqlite(value: bool) -> i64 {
    i64::from(value)
}
