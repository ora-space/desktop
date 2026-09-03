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
    use ora_application::{NetworkProxySettings, UserConfigService};
    use ora_user_config::UserConfigRepository;
    use pretty_assertions::assert_eq;
    use rusqlite::Connection;
    use tempfile::TempDir;

    use super::SqliteUserConfigRepository;
    use crate::{DatabaseBootstrapper, DatabaseLocation, default_migration_catalog};

    fn repository() -> (TempDir, SqliteUserConfigRepository) {
        let temporary = TempDir::new().expect("create user-config temp directory");
        let database_path = temporary.path().join("ora.sqlite3");
        let pool = DatabaseBootstrapper::system()
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
