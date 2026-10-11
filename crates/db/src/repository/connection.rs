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
    /// Builds a repository pool for the selected SQLite storage mode.
    pub fn new(location: &DatabaseLocation) -> Result<Self, DatabaseError> {
        let manager = SqliteConnectionManager::new(location.clone());
        let builder = Pool::builder();
        // SQLite's anonymous in-memory databases belong to one connection. A single pooled
        // connection preserves the schema and data while still exercising the repository pool.
        let inner = if matches!(location, DatabaseLocation::InMemory) {
            builder.max_size(1).build(manager)?
        } else {
            builder.build(manager)?
        };

        Ok(Self { inner })
    }

    /// Holds a real pooled connection while a test controls when contention ends.
    #[cfg(feature = "test-support")]
    pub fn with_held_connection<T>(
        &self,
        operation: impl FnOnce() -> T,
    ) -> Result<T, DatabaseError> {
        self.with_connection(|_connection| Ok(operation()))
    }

    /// Runs one repository operation with a configured pooled SQLite connection.
    pub(crate) fn with_connection<T>(
        &self,
        operation: impl FnOnce(&Connection) -> Result<T, crate::DatabaseError>,
    ) -> Result<T, DatabaseError> {
        let connection = self.inner.get()?;

        operation(&connection)
    }

    /// Runs one repository operation that requires exclusive access to a pooled connection.
    ///
    /// Explicit transactions use a mutable borrow so rusqlite can reject nested transactions
    /// at compile time instead of relying on SQLite to detect them at runtime.
    pub(crate) fn with_connection_mut<T>(
        &self,
        operation: impl FnOnce(&mut Connection) -> Result<T, crate::DatabaseError>,
    ) -> Result<T, DatabaseError> {
        let mut connection = self.inner.get()?;

        operation(&mut connection)
    }
}

/// Opens and validates SQLite connections for the repository pool.
#[derive(Clone, Debug)]
struct SqliteConnectionManager {
    location: DatabaseLocation,
}

impl SqliteConnectionManager {
    /// Captures the SQLite location used for pooled connections.
    fn new(location: DatabaseLocation) -> Self {
        Self { location }
    }
}

impl ManageConnection for SqliteConnectionManager {
    type Connection = Connection;
    type Error = rusqlite::Error;

    /// Opens a SQLite connection and applies the shared repository PRAGMAs.
    fn connect(&self) -> Result<Self::Connection, Self::Error> {
        let connection = self.location.open()?;

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
    // WAL defers durability decisions to the `synchronous` setting: with NORMAL a
    // COMMIT only appends write-ahead frames to the OS page cache, so a power cut
    // rolls the database back to the last WAL sync — which, because auto-checkpoint
    // rarely triggers at Ora's write volume and the WAL is only reset on a clean
    // close, can span the whole session. Users experienced exactly that as projects
    // disappearing from the sidebar and every task/worktree of the surviving
    // projects vanishing after a power outage. FULL makes each COMMIT fsync the WAL
    // before reporting success, so power loss can lose at most the in-flight
    // transaction, never acknowledged writes. This matches the durability policy
    // every other SQLite owner in the workspace (node-db, controller, and the
    // process-runtime state journal) already applies.
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.pragma_update(None, "foreign_keys", "ON")?;

    Ok(())
}

/// Encodes a Rust boolean into the integer representation used by the schema.
pub(crate) fn bool_to_sqlite(value: bool) -> i64 {
    i64::from(value)
}
