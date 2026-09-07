use super::Settings;
use crate::BackendError;
use ora_application::{DeveloperMode, NetworkProxySettings};
use ora_contracts::{ContractError, EmptyErrorParams, PublicError, RequestId};
use ora_db::{DatabaseBootstrapper, DatabaseLocation, RepositoryPool, default_migration_catalog};
use ora_logging::{LogLevel, with_trace_logging};
use ora_runtime_settings::PreferredLogLevelStore;
use pretty_assertions::assert_eq;
use std::future::Future;
use std::path::Path;
use tempfile::TempDir;

/// Opens only the settings storage, without plugin discovery, a scheduler, or agent supervisors.
fn open_settings(path: &Path) -> (RepositoryPool, Settings) {
    let pool = DatabaseBootstrapper::system()
        .bootstrap_repository_pool(
            &DatabaseLocation::path(path),
            &default_migration_catalog().expect("migration catalog"),
        )
        .expect("settings database");
    let settings = Settings::new(pool.clone());
    (pool, settings)
}

/// Keeps fixture setup and asynchronous settings calls inside the same test-scoped subscriber.
fn run_test(test: impl Future<Output = ()>) {
    with_trace_logging(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(test);
    });
}

/// Pins the full public projection without depending on SQLite's diagnostic text.
fn assert_storage_failure(error: BackendError) {
    let request_id = RequestId::new_v4();
    assert_eq!(
        error.contract_error(request_id),
        ContractError {
            request_id,
            error: PublicError::InternalError(EmptyErrorParams {}),
        }
    );
    assert!(std::error::Error::source(&error).is_some());
}

/// Exercises defaults, writes, the restricted logging capability, and persistence after reopening.
#[test]
fn preferences_round_trip_without_opening_the_application_runtime() {
    run_test(async {
        let temporary = TempDir::new().expect("settings fixture");
        let path = temporary.path().join("ora.sqlite3");
        let (pool, settings) = open_settings(&path);
        assert_eq!(
            (
                settings
                    .developer_mode()
                    .await
                    .expect("default developer mode"),
                settings
                    .preferred_log_level()
                    .await
                    .expect("default log level"),
                settings.network_proxy_settings().expect("default proxy"),
            ),
            (DeveloperMode::Disabled, LogLevel::Info, None)
        );
        assert_eq!(
            settings
                .set_developer_mode(DeveloperMode::Enabled)
                .await
                .expect("save mode"),
            DeveloperMode::Enabled
        );
        let logging = settings.preferred_log_level_store();
        logging
            .save_preferred_level(LogLevel::Debug)
            .await
            .expect("save preferred level");
        assert_eq!(
            logging
                .load_preferred_level()
                .await
                .expect("load preferred level"),
            LogLevel::Debug
        );
        let proxy = NetworkProxySettings {
            host: "proxy.example.test".to_string(),
            port: 8080,
            username: Some("fixture".to_string()),
            password: Some("fixture-secret".to_string()),
        };
        assert_eq!(
            settings
                .set_network_proxy_settings(proxy.clone())
                .expect("save proxy"),
            proxy
        );
        drop(logging);
        drop(settings);
        drop(pool);

        let (_pool, reopened) = open_settings(&path);
        assert_eq!(
            (
                reopened
                    .developer_mode()
                    .await
                    .expect("reopened developer mode"),
                reopened
                    .preferred_log_level()
                    .await
                    .expect("reopened log level"),
                reopened.network_proxy_settings().expect("reopened proxy"),
            ),
            (DeveloperMode::Enabled, LogLevel::Debug, Some(proxy))
        );
        reopened
            .clear_network_proxy_settings()
            .expect("clear proxy");
        assert_eq!(
            reopened.network_proxy_settings().expect("cleared proxy"),
            None
        );
    });
}

/// A real SQLite write failure stays visible and cannot replace the last durable preference.
#[test]
fn failed_preference_write_preserves_the_stored_value() {
    run_test(async {
        let temporary = TempDir::new().expect("settings fixture");
        let path = temporary.path().join("ora.sqlite3");
        let (_pool, settings) = open_settings(&path);
        settings
            .set_developer_mode(DeveloperMode::Enabled)
            .await
            .expect("seed preference");
        rusqlite::Connection::open(&path)
            .expect("fixture fault-injection connection")
            .execute_batch(
                "CREATE TRIGGER reject_settings_write BEFORE INSERT ON user_config
                 BEGIN SELECT RAISE(FAIL, 'fixture storage failure'); END;",
            )
            .expect("inject write failure in fixture database");
        assert_storage_failure(
            settings
                .set_developer_mode(DeveloperMode::Disabled)
                .await
                .expect_err("write must fail"),
        );
        assert_storage_failure(
            settings
                .preferred_log_level_store()
                .save_preferred_level(LogLevel::Error)
                .await
                .expect_err("restricted store must propagate failure"),
        );
        assert_eq!(
            (
                settings
                    .developer_mode()
                    .await
                    .expect("unchanged developer mode"),
                settings
                    .preferred_log_level()
                    .await
                    .expect("unchanged log level"),
            ),
            (DeveloperMode::Enabled, LogLevel::Info)
        );
    });
}

/// Storage faults do not silently turn into defaults or empty proxy settings.
#[test]
fn failed_reads_are_not_reported_as_default_preferences() {
    run_test(async {
        let temporary = TempDir::new().expect("settings fixture");
        let path = temporary.path().join("ora.sqlite3");
        let (_pool, settings) = open_settings(&path);
        rusqlite::Connection::open(&path)
            .expect("fixture fault-injection connection")
            .execute_batch("DROP TABLE user_config")
            .expect("remove only the fixture table to simulate a read fault");
        assert_storage_failure(
            settings
                .developer_mode()
                .await
                .expect_err("developer read fails"),
        );
        assert_storage_failure(
            settings
                .preferred_log_level()
                .await
                .expect_err("log level read fails"),
        );
        assert_storage_failure(
            settings
                .network_proxy_settings()
                .expect_err("proxy read fails"),
        );
    });
}
