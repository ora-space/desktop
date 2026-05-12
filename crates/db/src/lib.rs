mod bootstrap;
mod error;
mod location;
mod migration;
mod repository;
mod time;

#[cfg(test)]
mod repository_tests;
#[cfg(test)]
mod tests;

pub use bootstrap::{Database, DatabaseBootstrapper};
pub use error::{DatabaseError, MigrationDirection};
pub use location::DatabaseLocation;
pub use migration::{AppliedMigration, Migration, MigrationCatalog, default_migration_catalog};
pub use repository::{
    RepositoryPool, SqliteProjectRepository, SqliteSessionRepository, SqliteTaskRepository,
    SqliteWorktreeRepository,
};
pub use time::{SystemTimestampSource, TimestampSource};
