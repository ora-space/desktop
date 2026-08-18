mod bootstrap;
mod error;
mod location;
mod migration;
mod repository;
mod time;

#[cfg(test)]
mod git_cleanup_tests;
#[cfg(test)]
mod plugin_repository_tests;
#[cfg(test)]
mod repository_tests;
#[cfg(test)]
mod tests;

pub use bootstrap::{Database, DatabaseBootstrapper};
pub use error::{DatabaseError, MigrationDirection};
pub use location::DatabaseLocation;
pub use migration::{AppliedMigration, Migration, MigrationCatalog, default_migration_catalog};
pub use repository::{
    CascadeDeleteOutcome, RepositoryPool, SqliteAgentDefinitionRepository, SqliteCascadeRepository,
    SqliteGitCleanupJobRepository, SqlitePluginStateRepository, SqliteProjectRepository,
    SqliteSessionRepository, SqliteSkillRepository, SqliteTaskDiffCommentRepository,
    SqliteTaskRepository, SqliteTaskWorkspaceRepository, SqliteWorkflowRepository,
    SqliteWorkflowRunEngineRepository, SqliteWorkflowRunRepository,
    SqliteWorktreeProvisioningLeaseRepository, SqliteWorktreeRepository,
};
pub use time::{SystemTimestampSource, TimestampSource};
