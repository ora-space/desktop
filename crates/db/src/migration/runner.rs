use ora_logging::{ora_error, ora_info};
use rusqlite::{Connection, Transaction, params};

use crate::{DatabaseError, MigrationCatalog, MigrationDirection, TimestampSource};

use super::AppliedMigration;

const CREATE_MIGRATIONS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS migrations (
    version TEXT PRIMARY KEY,
    up_sql TEXT NOT NULL,
    down_sql TEXT NOT NULL,
    executed_at INTEGER NOT NULL
);
"#;

/// Reconciles a SQLite connection with the catalog's target prefix by applying or rolling back migrations.
pub fn reconcile_database<T>(
    connection: &mut Connection,
    catalog: &MigrationCatalog,
    timestamp_source: &T,
) -> Result<(), DatabaseError>
where
    T: TimestampSource,
{
    ensure_migrations_table(connection)?;

    let applied_migrations = load_applied_migrations(connection)?;
    let target_versions = catalog.target_versions();
    validate_applied_history(&applied_migrations, catalog)?;
    let reconciliation_start =
        first_reconciliation_position(&applied_migrations, target_versions, catalog);
    let pending_up_count = target_versions.len().saturating_sub(reconciliation_start);
    let pending_down_count = applied_migrations
        .len()
        .saturating_sub(reconciliation_start);

    ora_info!(
        message = "evaluated migration reconciliation",
        operation = "migration_reconciliation",
        applied_migration_count = applied_migrations.len(),
        target_migration_count = target_versions.len(),
        pending_up_count,
        pending_down_count
    );

    if pending_down_count > 0 {
        ora_info!(
            message = "rolling back migration suffix",
            operation = "migration_reconciliation",
            rollback_count = pending_down_count
        );

        for applied_migration in applied_migrations.iter().skip(reconciliation_start).rev() {
            execute_migration_step(
                connection,
                &applied_migration.version,
                &applied_migration.down_sql,
                MigrationDirection::Down,
                |transaction| {
                    transaction.execute(
                        "DELETE FROM migrations WHERE version = ?1",
                        params![applied_migration.version],
                    )?;

                    Ok(())
                },
            )?;
        }
    }

    if pending_up_count > 0 {
        ora_info!(
            message = "applying pending migrations",
            operation = "migration_reconciliation",
            apply_count = pending_up_count
        );

        for target_version in target_versions.iter().skip(reconciliation_start) {
            let migration = catalog.migration(target_version).ok_or_else(|| {
                ora_error!(
                    message = "target migration version is missing from the catalog",
                    operation = "migration_reconciliation",
                    migration_version = (*target_version).to_string(),
                    error.kind = "unknown_applied_migration_version",
                    error.message = format!(
                        "target migration version {} is missing from the catalog",
                        target_version
                    )
                );

                DatabaseError::UnknownAppliedMigrationVersion {
                    version: (*target_version).to_string(),
                }
            })?;

            let up_sql = migration.up_sql();
            let down_sql = migration.down_sql();
            execute_migration_step(
                connection,
                migration.version(),
                &up_sql,
                MigrationDirection::Up,
                |transaction| {
                    transaction.execute(
                        "INSERT INTO migrations (version, up_sql, down_sql, executed_at) VALUES (?1, ?2, ?3, ?4)",
                        params![
                            migration.version(),
                            up_sql,
                            down_sql,
                            timestamp_source.current_timestamp_millis()
                        ],
                    )?;

                    Ok(())
                },
            )?;
        }
    }

    if pending_up_count == 0 && pending_down_count == 0 {
        ora_info!(
            message = "database schema already matches the target migration prefix",
            operation = "migration_reconciliation"
        );
    }

    Ok(())
}

/// Ensures the bookkeeping table exists before reconciliation starts reading or mutating migration state.
fn ensure_migrations_table(connection: &Connection) -> Result<(), DatabaseError> {
    connection.execute_batch(CREATE_MIGRATIONS_TABLE_SQL)?;
    Ok(())
}

/// Loads applied migration rows in ascending version order so prefix comparison stays deterministic.
fn load_applied_migrations(
    connection: &Connection,
) -> Result<Vec<AppliedMigration>, DatabaseError> {
    let mut statement = connection.prepare(
        "SELECT version, up_sql, down_sql, executed_at FROM migrations ORDER BY version ASC",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(AppliedMigration::new(
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(DatabaseError::from)
}

/// Rejects unknown, missing, or reordered applied versions before schema mutation begins.
fn validate_applied_history(
    applied_migrations: &[AppliedMigration],
    catalog: &MigrationCatalog,
) -> Result<(), DatabaseError> {
    for (position, applied) in applied_migrations.iter().enumerate() {
        if catalog.migration(&applied.version).is_none() {
            ora_error!(
                message = "encountered unknown applied migration version",
                operation = "migration_reconciliation",
                migration_version = applied.version.clone(),
                error.kind = "unknown_applied_migration_version",
                error.message = format!(
                    "database contains unknown applied migration version {}",
                    applied.version
                )
            );
            return Err(DatabaseError::UnknownAppliedMigrationVersion {
                version: applied.version.clone(),
            });
        }

        let expected = catalog.version_at(position).unwrap_or_default();
        if applied.version != expected {
            ora_error!(
                message = "migration history diverged",
                operation = "migration_reconciliation",
                migration_position = position,
                error.kind = "diverged_migration_history",
                error.message = format!(
                    "expected migration version {expected}, found {}",
                    applied.version
                )
            );
            return Err(DatabaseError::DivergedMigrationHistory {
                position,
                expected: expected.to_string(),
                found: applied.version.clone(),
            });
        }
    }

    Ok(())
}

/// Finds the first SQL drift or missing/trailing migration that requires reconciliation.
fn first_reconciliation_position(
    applied_migrations: &[AppliedMigration],
    target_versions: &[&str],
    catalog: &MigrationCatalog,
) -> usize {
    applied_migrations
        .iter()
        .zip(target_versions)
        .position(
            |(applied, target_version)| match catalog.migration(target_version) {
                Some(migration) => {
                    applied.up_sql != migration.up_sql() || applied.down_sql != migration.down_sql()
                }
                None => true,
            },
        )
        .unwrap_or_else(|| applied_migrations.len().min(target_versions.len()))
}

/// Executes one migration direction inside a transaction so SQL changes and bookkeeping updates succeed together.
fn execute_migration_step<F>(
    connection: &mut Connection,
    version: &str,
    sql: &str,
    direction: MigrationDirection,
    finalize: F,
) -> Result<(), DatabaseError>
where
    F: FnOnce(&Transaction<'_>) -> Result<(), rusqlite::Error>,
{
    ora_info!(
        message = "executing migration step",
        operation = "migration_execute",
        migration_version = version.to_string(),
        direction = direction.to_string()
    );

    let transaction = connection.transaction()?;

    transaction.execute_batch(sql).map_err(|source| {
        ora_error!(
            message = "migration step failed",
            operation = "migration_execute",
            migration_version = version.to_string(),
            direction = direction.to_string(),
            error.kind = "migration_step_failed",
            error.message = source.to_string()
        );

        DatabaseError::MigrationStepFailed {
            version: version.to_string(),
            direction,
            source,
        }
    })?;

    finalize(&transaction).map_err(|source| {
        ora_error!(
            message = "migration bookkeeping failed",
            operation = "migration_execute",
            migration_version = version.to_string(),
            direction = direction.to_string(),
            error.kind = "migration_step_failed",
            error.message = source.to_string()
        );

        DatabaseError::MigrationStepFailed {
            version: version.to_string(),
            direction,
            source,
        }
    })?;
    transaction.commit()?;

    ora_info!(
        message = "executed migration step",
        operation = "migration_execute",
        migration_version = version.to_string(),
        direction = direction.to_string()
    );

    Ok(())
}
