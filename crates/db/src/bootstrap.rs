use ora_logging::{ora_error, ora_info};
use rusqlite::Connection;

use crate::{
    DatabaseError, DatabaseLocation, MigrationCatalog, RepositoryPool, SystemTimestampSource,
    TimestampSource, migration,
};

/// Owns a SQLite connection that has already been reconciled with the active migration target.
#[derive(Debug)]
pub struct Database {
    connection: Connection,
}

impl Database {
    /// Exposes the managed SQLite connection for query and repository work.
    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// Transfers ownership of the managed connection to callers that need direct control.
    pub fn into_connection(self) -> Connection {
        self.connection
    }
}

/// Coordinates opening SQLite connections and reconciling them with the migration catalog.
#[derive(Debug)]
pub struct DatabaseBootstrapper<T> {
    timestamp_source: T,
}

impl DatabaseBootstrapper<SystemTimestampSource> {
    /// Builds a bootstrapper that timestamps applied migrations from the system clock.
    pub fn system() -> Self {
        Self::new(SystemTimestampSource)
    }
}

impl<T> DatabaseBootstrapper<T>
where
    T: TimestampSource,
{
    /// Builds a bootstrapper around a caller-provided timestamp source for deterministic tests.
    pub fn new(timestamp_source: T) -> Self {
        Self { timestamp_source }
    }

    /// Opens a database location, reconciles it with the target migration prefix, and returns the ready connection.
    pub fn bootstrap(
        &self,
        location: &DatabaseLocation,
        catalog: &MigrationCatalog,
    ) -> Result<Database, DatabaseError> {
        ora_info!(
            message = "opening database",
            operation = "database_open",
            location = location.logging_label()
        );

        let mut connection = match location.open() {
            Ok(connection) => connection,
            Err(error) => {
                ora_error!(
                    message = "failed to open database",
                    operation = "database_open",
                    location = location.logging_label(),
                    error.kind = "database_open",
                    error.message = error.to_string()
                );
                return Err(error);
            }
        };

        ora_info!(
            message = "opened database",
            operation = "database_open",
            location = location.logging_label()
        );

        if let Err(error) =
            migration::reconcile_database(&mut connection, catalog, &self.timestamp_source)
        {
            ora_error!(
                message = "database bootstrap failed",
                operation = "database_bootstrap",
                location = location.logging_label(),
                error.kind = "database_bootstrap",
                error.message = error.to_string()
            );
            return Err(error);
        }

        ora_info!(
            message = "database bootstrap complete",
            operation = "database_bootstrap",
            location = location.logging_label()
        );

        Ok(Database { connection })
    }

    /// Reconciles the database schema and returns the configured repository pool for later use.
    pub fn bootstrap_repository_pool(
        &self,
        location: &DatabaseLocation,
        catalog: &MigrationCatalog,
    ) -> Result<RepositoryPool, DatabaseError> {
        self.bootstrap(location, catalog)?;

        RepositoryPool::new(location)
    }
}
