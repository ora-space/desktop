mod bootstrap;
mod error;
mod legacy_effect_cleanup;
mod location;
mod migration;
mod repository;
mod time;

#[cfg(test)]
mod durability_tests;
#[cfg(test)]
mod git_cleanup_tests;
#[cfg(test)]
mod repository_tests;
#[cfg(test)]
mod settings_upgrade_tests;
#[cfg(test)]
mod sqlite_engine_tests;
#[cfg(test)]
mod test_clock;
#[cfg(test)]
mod tests;

pub use bootstrap::{BootstrappedDatabase, DatabaseBootstrapper, reconcile_migration_history};
pub use error::{DatabaseError, MigrationDirection};
pub use legacy_effect_cleanup::{
    LegacyManagedSkillRow, cleanup_legacy_managed_skills, read_legacy_managed_skills,
};
pub use location::DatabaseLocation;
pub use migration::{
    AppliedMigration, DatabaseOrigin, Migration, MigrationCatalog, default_migration_catalog,
};
pub use repository::{
    CascadeDeleteOutcome, PackInstallationMemberRecord, PackInstallationRecord,
    PackMemberOwnership, PluginMarketplaceSourceRecord, PluginSkillProjection, RepositoryPool,
    SqliteAgentDefinitionRepository, SqliteCascadeRepository, SqliteGitCleanupJobRepository,
    SqlitePackInstallationRepository, SqlitePluginMarketplaceSourceRepository,
    SqlitePluginSourceNamespaceRepository, SqliteProjectRepository, SqliteScheduledRunRepository,
    SqliteScheduledTaskRepository, SqliteSessionRepository, SqliteSkillRepository,
    SqliteTaskRepository, SqliteTaskWorkspaceRepository, SqliteUserConfigRepository,
    SqliteWorkflowDocumentSourceRepository, SqliteWorkflowRepository,
    SqliteWorkflowRunEngineRepository, SqliteWorkflowRunRepository, SqliteWorkspaceRepository,
    SqliteWorktreeProvisioningLeaseRepository, SqliteWorktreeRepository,
};
pub use time::{LocalTimestampSource, SystemTimestampSource, TimestampSource};
