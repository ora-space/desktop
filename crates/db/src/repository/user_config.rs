use ora_application::RepositoryError;
use ora_user_config::UserConfigRepository;
use rusqlite::{OptionalExtension, params};

use crate::repository::RepositoryPool;

/// Adapts SQLite user_config rows to the generic key/value persistence seam.
#[derive(Clone, Debug)]
pub struct SqliteUserConfigRepository {
    pool: RepositoryPool,
}

impl SqliteUserConfigRepository {
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }
}

impl UserConfigRepository for SqliteUserConfigRepository {
    type Error = RepositoryError;

    fn get_pair(&self, first: &str, second: &str) -> Result<[Option<String>; 2], Self::Error> {
        self.pool
            .with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT (SELECT value FROM user_config WHERE key = ?1),
                        (SELECT value FROM user_config WHERE key = ?2)",
                        params![first, second],
                        |row| Ok([row.get(0)?, row.get(1)?]),
                    )
                    .map_err(crate::DatabaseError::from)
            })
            .map_err(repository_error_from_database)
    }

    fn set_pair(&self, first: (&str, &str), second: (&str, &str)) -> Result<(), Self::Error> {
        self.pool
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO user_config (key, value) VALUES (?1, ?2), (?3, ?4)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![first.0, first.1, second.0, second.1],
                )?;
                Ok(())
            })
            .map_err(repository_error_from_database)
    }

    fn get_value(&self, key: &str) -> Result<Option<String>, Self::Error> {
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
            .map_err(repository_error_from_database)
    }

    fn set_value(&self, key: &str, value: &str) -> Result<(), Self::Error> {
        self.pool
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO user_config (key, value) VALUES (?1, ?2)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![key, value],
                )?;
                Ok(())
            })
            .map_err(repository_error_from_database)
    }

    fn delete_value(&self, key: &str) -> Result<(), Self::Error> {
        self.pool
            .with_connection(|connection| {
                connection.execute("DELETE FROM user_config WHERE key = ?1", params![key])?;
                Ok(())
            })
            .map_err(repository_error_from_database)
    }
}

fn repository_error_from_database(error: crate::DatabaseError) -> RepositoryError {
    RepositoryError::new(error)
}

#[cfg(test)]
mod tests {
    use ora_application::{
        NetworkProxyMode, NetworkProxyPreferences, NetworkProxySettings, UserConfigService,
    };
    use ora_user_config::UserConfigRepository;
    use pretty_assertions::assert_eq;
    use rusqlite::Connection;
    use tempfile::TempDir;

    use super::SqliteUserConfigRepository;
    use crate::{DatabaseBootstrapper, DatabaseLocation, default_migration_catalog};

    fn repository() -> (TempDir, SqliteUserConfigRepository) {
        let temporary = TempDir::new().expect("create user-config temp directory");
        let database_path = temporary.path().join("ora.sqlite3");
        let pool = DatabaseBootstrapper::new(crate::test_clock::TestClock::new(1))
            .bootstrap_repository_pool(
                &DatabaseLocation::path(database_path),
                &default_migration_catalog().unwrap(),
            )
            .expect("bootstrap user-config repository");
        (temporary, SqliteUserConfigRepository::new(pool))
    }

    #[test]
    fn missing_key_does_not_materialize_a_row() {
        let (temporary, repository) = repository();
        assert_eq!(repository.get_value("missing").unwrap(), None);

        let connection = Connection::open(temporary.path().join("ora.sqlite3")).unwrap();
        let row_count = connection
            .query_row("SELECT COUNT(*) FROM user_config", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        assert_eq!(row_count, 0);
    }

    #[test]
    fn upserts_only_the_requested_key() {
        let (_temporary, repository) = repository();
        repository.set_value("first", "one").unwrap();
        repository.set_value("second", "two").unwrap();
        repository.set_value("first", "updated").unwrap();

        assert_eq!(
            repository.get_value("first").unwrap().as_deref(),
            Some("updated")
        );
        assert_eq!(
            repository.get_value("second").unwrap().as_deref(),
            Some("two")
        );
    }

    #[test]
    fn deletes_only_the_requested_key() {
        let (_temporary, repository) = repository();
        repository.set_value("first", "one").unwrap();
        repository.set_value("second", "two").unwrap();

        repository.delete_value("first").unwrap();

        assert_eq!(repository.get_value("first").unwrap(), None);
        assert_eq!(
            repository.get_value("second").unwrap().as_deref(),
            Some("two")
        );
    }

    /// Covers the persisted mode independently of the endpoint that an older build still reads.
    #[test]
    fn proxy_modes_preserve_the_legacy_endpoint() {
        let (_temporary, repository) = repository();
        let service = UserConfigService::new(repository.clone());
        let manual = NetworkProxySettings {
            host: "legacy.example.com".into(),
            port: 8080,
            username: None,
            password: None,
        };

        assert_eq!(
            service.network_proxy_preferences().unwrap(),
            NetworkProxyPreferences {
                mode: NetworkProxyMode::System,
                manual: None,
            }
        );
        assert_eq!(repository.get_value("network_proxy_mode").unwrap(), None);

        service.set_network_proxy_settings(manual.clone()).unwrap();
        assert_eq!(
            service.network_proxy_preferences().unwrap(),
            NetworkProxyPreferences {
                mode: NetworkProxyMode::Manual,
                manual: Some(manual.clone()),
            }
        );

        for mode in [NetworkProxyMode::System, NetworkProxyMode::Direct] {
            service.set_network_proxy_preferences(mode, None).unwrap();
            let reopened = UserConfigService::new(repository.clone());
            assert_eq!(
                reopened.network_proxy_preferences().unwrap(),
                NetworkProxyPreferences {
                    mode,
                    manual: Some(manual.clone()),
                }
            );
            assert_eq!(
                reopened.network_proxy_settings().unwrap(),
                Some(manual.clone())
            );
        }
    }

    /// Rejects partial writes if the endpoint row cannot be stored.
    #[test]
    fn proxy_mode_and_endpoint_are_committed_together() {
        let (temporary, repository) = repository();
        let service = UserConfigService::new(repository);
        service
            .set_network_proxy_preferences(NetworkProxyMode::Direct, None)
            .unwrap();
        let connection = Connection::open(temporary.path().join("ora.sqlite3")).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER reject_manual_proxy BEFORE INSERT ON user_config
             WHEN NEW.key = 'network_proxy_settings'
             BEGIN SELECT RAISE(ABORT, 'reject manual proxy'); END;",
            )
            .unwrap();
        let manual = NetworkProxySettings {
            host: "proxy.example.com".into(),
            port: 8080,
            username: None,
            password: None,
        };
        assert!(
            service
                .set_network_proxy_preferences(NetworkProxyMode::Manual, Some(manual))
                .is_err()
        );
        assert_eq!(
            service.network_proxy_preferences().unwrap(),
            NetworkProxyPreferences {
                mode: NetworkProxyMode::Direct,
                manual: None,
            }
        );
    }

    /// Treats an unknown value as an error without replacing persisted preferences.
    #[test]
    fn unknown_proxy_mode_is_not_treated_as_missing() {
        let (_temporary, repository) = repository();
        repository
            .set_value("network_proxy_mode", "future-mode")
            .unwrap();
        let service = UserConfigService::new(repository.clone());
        assert!(service.network_proxy_preferences().is_err());
        assert_eq!(
            repository
                .get_value("network_proxy_mode")
                .unwrap()
                .as_deref(),
            Some("future-mode")
        );
    }

    #[test]
    fn round_trips_network_proxy_settings_through_the_generic_store() {
        let (_temporary, repository) = repository();
        let service = UserConfigService::new(repository);
        let settings = NetworkProxySettings {
            host: "proxy.example.com".to_owned(),
            port: 8080,
            username: Some("ora".to_owned()),
            password: Some("secret".to_owned()),
        };

        assert_eq!(service.network_proxy_settings().unwrap(), None);
        assert_eq!(
            service
                .set_network_proxy_settings(settings.clone())
                .unwrap(),
            settings
        );
        assert_eq!(service.network_proxy_settings().unwrap(), Some(settings));

        service.clear_network_proxy_settings().unwrap();
        assert_eq!(service.network_proxy_settings().unwrap(), None);
    }
}
