use ora_application::{DeveloperMode, NetworkProxySettings, RepositoryError, UserConfigRepository};
use ora_logging::LogLevel;
use rusqlite::{OptionalExtension, params};

use crate::repository::RepositoryPool;

const DEVELOPER_MODE_KEY: &str = "developer_mode";
const LOG_LEVEL_KEY: &str = "log_level";
const NETWORK_PROXY_SETTINGS_KEY: &str = "network_proxy_settings";

/// Persists the supported shared user preferences in SQLite's `user_config` table.
#[derive(Clone, Debug)]
pub struct SqliteUserConfigRepository {
    pool: RepositoryPool,
}

impl SqliteUserConfigRepository {
    /// Builds a typed user-configuration repository from the shared pool.
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }

    /// Loads one raw preference without materializing a default database row.
    fn load_value(&self, key: &'static str) -> Result<Option<String>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT value FROM user_config WHERE key = ?1",
                        params![key],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(crate::DatabaseError::from)
            })
            .map_err(user_config_repository_error_from_database)
    }

    /// Upserts one key so writes cannot modify unrelated preference rows.
    fn save_value(&self, key: &'static str, value: &str) -> Result<(), RepositoryError> {
        self.pool
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO user_config (key, value) VALUES (?1, ?2)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![key, value],
                )?;
                Ok(())
            })
            .map_err(user_config_repository_error_from_database)
    }
}

impl UserConfigRepository for SqliteUserConfigRepository {
    /// Loads the canonical developer-mode text or returns the disabled default.
    fn load_developer_mode(&self) -> Result<DeveloperMode, RepositoryError> {
        match self.load_value(DEVELOPER_MODE_KEY)?.as_deref() {
            None | Some("false") => Ok(DeveloperMode::Disabled),
            Some("true") => Ok(DeveloperMode::Enabled),
            Some(_) => Err(corrupt_value_error(DEVELOPER_MODE_KEY)),
        }
    }

    /// Stores the canonical lower-case developer-mode text.
    fn save_developer_mode(&self, mode: DeveloperMode) -> Result<(), RepositoryError> {
        let value = match mode {
            DeveloperMode::Enabled => "true",
            DeveloperMode::Disabled => "false",
        };
        self.save_value(DEVELOPER_MODE_KEY, value)
    }

    /// Loads the canonical log level or returns the info default.
    fn load_preferred_log_level(&self) -> Result<LogLevel, RepositoryError> {
        match self.load_value(LOG_LEVEL_KEY)?.as_deref() {
            None => Ok(LogLevel::Info),
            Some("trace") => Ok(LogLevel::Trace),
            Some("debug") => Ok(LogLevel::Debug),
            Some("info") => Ok(LogLevel::Info),
            Some("warn") => Ok(LogLevel::Warn),
            Some("error") => Ok(LogLevel::Error),
            Some(_) => Err(corrupt_value_error(LOG_LEVEL_KEY)),
        }
    }

    /// Stores the canonical lower-case log-level text.
    fn save_preferred_log_level(&self, level: LogLevel) -> Result<(), RepositoryError> {
        self.save_value(LOG_LEVEL_KEY, level.as_str())
    }

    /// Loads the optional network proxy settings as JSON, treating a missing row as unset.
    fn load_network_proxy_settings(&self) -> Result<Option<NetworkProxySettings>, RepositoryError> {
        match self.load_value(NETWORK_PROXY_SETTINGS_KEY)? {
            None => Ok(None),
            Some(value) => serde_json::from_str(&value)
                .map(Some)
                .map_err(|_| corrupt_value_error(NETWORK_PROXY_SETTINGS_KEY)),
        }
    }

    /// Stores the network proxy settings as a single JSON value.
    fn save_network_proxy_settings(
        &self,
        settings: &NetworkProxySettings,
    ) -> Result<(), RepositoryError> {
        let value = serde_json::to_string(settings)
            .map_err(|_| corrupt_value_error(NETWORK_PROXY_SETTINGS_KEY))?;
        self.save_value(NETWORK_PROXY_SETTINGS_KEY, &value)
    }
}

/// Converts shared database failures into the application-owned repository boundary.
fn user_config_repository_error_from_database(error: crate::DatabaseError) -> RepositoryError {
    RepositoryError::new(error)
}

/// Builds a value-corruption error without exposing arbitrary stored text.
fn corrupt_value_error(key: &'static str) -> RepositoryError {
    user_config_repository_error_from_database(crate::DatabaseError::CorruptUserConfigValue { key })
}

#[cfg(test)]
mod tests {
    use super::{DEVELOPER_MODE_KEY, LOG_LEVEL_KEY, SqliteUserConfigRepository};
    use ora_application::{DeveloperMode, UserConfigRepository};
    use ora_logging::LogLevel;
    use pretty_assertions::assert_eq;
    use rusqlite::{Connection, params};
    use tempfile::TempDir;

    use crate::{DatabaseBootstrapper, DatabaseLocation, default_migration_catalog};

    /// Creates a migrated repository and retains its temporary database directory.
    fn repository() -> (TempDir, SqliteUserConfigRepository) {
        let temporary = TempDir::new().expect("create user-config temp directory");
        let database_path = temporary.path().join("ora.sqlite3");
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(&database_path),
                &default_migration_catalog().expect("build migration catalog"),
            )
            .expect("bootstrap user-config database");
        (temporary, SqliteUserConfigRepository::new(pool))
    }

    /// Verifies missing rows remain implicit while typed reads expose documented defaults.
    #[test]
    fn loads_defaults_without_materializing_rows() {
        let (temporary, repository) = repository();

        assert_eq!(
            (
                repository.load_developer_mode().unwrap(),
                repository.load_preferred_log_level().unwrap(),
            ),
            (DeveloperMode::Disabled, LogLevel::Info)
        );
        let connection = Connection::open(temporary.path().join("ora.sqlite3")).unwrap();
        let row_count = connection
            .query_row("SELECT COUNT(*) FROM user_config", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        assert_eq!(row_count, 0);
    }

    /// Verifies typed upserts use canonical text and preserve unrelated rows.
    #[test]
    fn upserts_canonical_values_without_replacing_other_preferences() {
        let (temporary, repository) = repository();

        repository
            .save_developer_mode(DeveloperMode::Enabled)
            .unwrap();
        repository
            .save_preferred_log_level(LogLevel::Trace)
            .unwrap();
        repository
            .save_developer_mode(DeveloperMode::Disabled)
            .unwrap();

        assert_eq!(
            (
                repository.load_developer_mode().unwrap(),
                repository.load_preferred_log_level().unwrap(),
            ),
            (DeveloperMode::Disabled, LogLevel::Trace)
        );
        let connection = Connection::open(temporary.path().join("ora.sqlite3")).unwrap();
        let stored = [DEVELOPER_MODE_KEY, LOG_LEVEL_KEY].map(|key| {
            connection
                .query_row(
                    "SELECT value FROM user_config WHERE key = ?1",
                    params![key],
                    |row| row.get::<_, String>(0),
                )
                .unwrap()
        });
        assert_eq!(stored, ["false".to_string(), "trace".to_string()]);
    }

    /// Verifies malformed supported rows are reported rather than silently reset.
    #[test]
    fn rejects_malformed_typed_values() {
        let (temporary, repository) = repository();
        let connection = Connection::open(temporary.path().join("ora.sqlite3")).unwrap();
        connection
            .execute(
                "INSERT INTO user_config (key, value) VALUES (?1, ?2), (?3, ?4)",
                params![DEVELOPER_MODE_KEY, "yes", LOG_LEVEL_KEY, "verbose"],
            )
            .unwrap();

        assert_eq!(
            repository.load_developer_mode().unwrap_err().to_string(),
            "repository operation failed"
        );
        assert_eq!(
            repository
                .load_preferred_log_level()
                .unwrap_err()
                .to_string(),
            "repository operation failed"
        );

        for non_canonical_value in [" DEBUG ", "Warn"] {
            connection
                .execute(
                    "UPDATE user_config SET value = ?1 WHERE key = ?2",
                    params![non_canonical_value, LOG_LEVEL_KEY],
                )
                .unwrap();
            assert_eq!(
                repository
                    .load_preferred_log_level()
                    .unwrap_err()
                    .to_string(),
                "repository operation failed"
            );
        }
    }
}
