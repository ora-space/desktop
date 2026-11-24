//! Exercises persisted preferences through the production startup migration boundary.

use ora_logging::with_trace_logging;
use ora_user_config::UserConfigRepository;
use pretty_assertions::assert_eq;

use crate::{
    DatabaseBootstrapper, DatabaseLocation, MigrationCatalog, SqliteUserConfigRepository,
    default_migration_catalog,
};

/// Every supported schema prefix must retain all raw preferences, including future keys.
#[test]
fn settings_survive_upgrade_from_every_schema_prefix_and_restart() {
    with_trace_logging(|| {
        let latest = default_migration_catalog().unwrap();
        let values = [
            (
                "network_proxy_settings",
                r#"{"host":"127.0.0.1","port":7890,"username":"user","password":"secret"}"#,
            ),
            ("developer_mode", "true"),
            ("log_level", "debug"),
            ("worktree_root", "C:/custom/worktrees"),
            ("cloud_server", "https://cloud.example"),
            ("future_preference", "preserve me"),
        ];
        for count in 1..=latest.target_versions().len() {
            let directory = tempfile::tempdir().unwrap();
            let location = DatabaseLocation::path(directory.path().join("ora.sqlite3"));
            let versions = latest.target_versions()[..count].to_vec();
            let migrations = latest
                .target_versions()
                .iter()
                .map(|version| latest.migration(version).unwrap().clone())
                .collect();
            let previous = MigrationCatalog::with_target_versions(migrations, versions).unwrap();
            let pool = DatabaseBootstrapper::system()
                .bootstrap_repository_pool(&location, &previous)
                .unwrap();
            if count >= 5 {
                pool.with_connection(|connection| {
                    connection.execute("INSERT INTO plugin_marketplace_source (url, branch, use_proxy, position, created_at, updated_at) VALUES ('https://example.com/custom-market', 'private-branch', 1, 0, 1, 1)", [])?;
                    Ok(())
                }).unwrap();
            }
            let repository = SqliteUserConfigRepository::new(pool);
            for (key, value) in values {
                repository.set_value(key, value).unwrap();
            }
            drop(repository);
            for _ in 0..2 {
                let pool = DatabaseBootstrapper::system()
                    .bootstrap_repository_pool(&location, &latest)
                    .unwrap();
                if count >= 5 {
                    let source = pool.with_connection(|connection| {
                        connection.query_row("SELECT branch, use_proxy FROM plugin_marketplace_source WHERE url = 'https://example.com/custom-market'", [], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))).map_err(Into::into)
                    }).unwrap();
                    assert_eq!(
                        source,
                        ("private-branch".to_owned(), 1),
                        "schema prefix {count}"
                    );
                }
                let repository = SqliteUserConfigRepository::new(pool);
                let actual =
                    values.map(|(key, _)| (key, repository.get_value(key).unwrap().unwrap()));
                assert_eq!(
                    actual,
                    values.map(|(key, value)| (key, value.to_owned())),
                    "schema prefix {count}"
                );
            }
        }
    });
}

/// A different data root looks like a first install; it does not migrate or delete the old values.
#[test]
fn changed_database_path_exposes_empty_settings_without_touching_original() {
    with_trace_logging(|| {
        let directory = tempfile::tempdir().unwrap();
        let catalog = default_migration_catalog().unwrap();
        let old_location = DatabaseLocation::path(directory.path().join("old.sqlite3"));
        let new_location = DatabaseLocation::path(directory.path().join("new.sqlite3"));
        let old_pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(&old_location, &catalog)
            .unwrap();
        let old_repository = SqliteUserConfigRepository::new(old_pool);
        old_repository
            .set_value("network_proxy_settings", "original")
            .unwrap();
        let new_pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(&new_location, &catalog)
            .unwrap();
        let new_repository = SqliteUserConfigRepository::new(new_pool);
        assert_eq!(
            new_repository.get_value("network_proxy_settings").unwrap(),
            None
        );
        assert_eq!(
            old_repository.get_value("network_proxy_settings").unwrap(),
            Some("original".to_owned())
        );
    });
}
