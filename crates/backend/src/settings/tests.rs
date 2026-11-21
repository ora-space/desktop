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
                settings
                    .network_proxy_settings()
                    .await
                    .expect("default proxy"),
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
                .await
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
                reopened
                    .network_proxy_settings()
                    .await
                    .expect("reopened proxy"),
            ),
            (DeveloperMode::Enabled, LogLevel::Debug, Some(proxy))
        );
        reopened
            .clear_network_proxy_settings()
            .await
            .expect("clear proxy");
        assert_eq!(
            reopened
                .network_proxy_settings()
                .await
                .expect("cleared proxy"),
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

/// A rejected proxy replacement preserves every persisted field, including after reopening.
#[test]
fn failed_proxy_write_preserves_the_stored_value_after_reopening() {
    run_test(async {
        let temporary = TempDir::new().expect("settings fixture");
        let path = temporary.path().join("ora.sqlite3");
        let (pool, settings) = open_settings(&path);
        let original = NetworkProxySettings {
            host: "proxy.example.test".to_string(),
            port: 8080,
            username: Some("fixture".to_string()),
            password: Some("fixture-secret".to_string()),
        };
        settings
            .set_network_proxy_settings(original.clone())
            .await
            .expect("seed proxy");
        rusqlite::Connection::open(&path)
            .expect("fixture fault-injection connection")
            .execute_batch(
                "CREATE TRIGGER reject_proxy_write BEFORE INSERT ON user_config
                 WHEN NEW.key = 'network_proxy_settings'
                 BEGIN SELECT RAISE(FAIL, 'fixture storage failure'); END;",
            )
            .expect("inject proxy write failure in fixture database");
        assert_storage_failure(
            settings
                .set_network_proxy_settings(NetworkProxySettings {
                    host: "replacement.example.test".to_string(),
                    port: 9090,
                    username: None,
                    password: None,
                })
                .await
                .expect_err("proxy write must fail"),
        );
        assert_eq!(
            settings
                .network_proxy_settings()
                .await
                .expect("unchanged proxy"),
            Some(original.clone())
        );
        drop(settings);
        drop(pool);

        let (_pool, reopened) = open_settings(&path);
        assert_eq!(
            reopened
                .network_proxy_settings()
                .await
                .expect("reopened proxy"),
            Some(original)
        );
    });
}

/// A rejected proxy deletion cannot erase the durable settings or hide the storage error.
#[test]
fn failed_proxy_clear_preserves_the_stored_value_after_reopening() {
    run_test(async {
        let temporary = TempDir::new().expect("settings fixture");
        let path = temporary.path().join("ora.sqlite3");
        let (pool, settings) = open_settings(&path);
        let original = NetworkProxySettings {
            host: "proxy.example.test".to_string(),
            port: 8080,
            username: Some("fixture".to_string()),
            password: Some("fixture-secret".to_string()),
        };
        settings
            .set_network_proxy_settings(original.clone())
            .await
            .expect("seed proxy");
        rusqlite::Connection::open(&path)
            .expect("fixture fault-injection connection")
            .execute_batch(
                "CREATE TRIGGER reject_proxy_clear BEFORE DELETE ON user_config
                 WHEN OLD.key = 'network_proxy_settings'
                 BEGIN SELECT RAISE(FAIL, 'fixture storage failure'); END;",
            )
            .expect("inject proxy clear failure in fixture database");
        assert_storage_failure(
            settings
                .clear_network_proxy_settings()
                .await
                .expect_err("proxy clear must fail"),
        );
        assert_eq!(
            settings
                .network_proxy_settings()
                .await
                .expect("unchanged proxy"),
            Some(original.clone())
        );
        drop(settings);
        drop(pool);

        let (_pool, reopened) = open_settings(&path);
        assert_eq!(
            reopened
                .network_proxy_settings()
                .await
                .expect("reopened proxy"),
            Some(original)
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
                .await
                .expect_err("proxy read fails"),
        );
    });
}

/// Holds the sole pooled connection until an async heartbeat releases it.
async fn while_connection_held<T>(pool: RepositoryPool, operation: impl Future<Output = T>) -> T {
    let (release, wait_for_release) = std::sync::mpsc::channel();
    let (acquired, wait_for_acquisition) = tokio::sync::oneshot::channel();
    let lock_holder = std::thread::spawn(move || {
        with_trace_logging(|| {
            pool.with_held_connection(|| {
                acquired.send(()).expect("notify connection acquired");
                // A blocking regression must fail instead of hanging the test runtime forever.
                wait_for_release
                    .recv_timeout(std::time::Duration::from_secs(/*secs*/ 3))
                    .is_ok()
            })
            .expect("hold pooled connection")
        })
    });
    wait_for_acquisition.await.expect("connection acquired");
    let mut operation = std::pin::pin!(operation);
    let initially_pending = std::future::poll_fn(|context| {
        std::task::Poll::Ready(operation.as_mut().poll(context).is_pending())
    })
    .await;
    // On a current-thread runtime this timer cannot advance if the settings call blocks it.
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_millis(/*millis*/ 50)).await;
    })
    .await
    .expect("independent async heartbeat");
    let still_pending = if initially_pending {
        std::future::poll_fn(|context| {
            std::task::Poll::Ready(operation.as_mut().poll(context).is_pending())
        })
        .await
    } else {
        false
    };
    let released_by_heartbeat = release.send(()).is_ok();
    let heartbeat_progressed = lock_holder.join().expect("lock holder");
    assert_eq!(
        (
            initially_pending,
            still_pending,
            released_by_heartbeat,
            heartbeat_progressed
        ),
        (true, true, true, true),
        "settings must yield while SQLite waits for the async heartbeat"
    );
    operation.await
}

/// Proxy reads yield the only async worker while SQLite waits, then return the stored value.
#[test]
fn proxy_read_does_not_block_async_progress() {
    run_test(async {
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::in_memory(),
                &default_migration_catalog().expect("migration catalog"),
            )
            .expect("single-connection database");
        let settings = Settings::new(pool.clone());
        assert_eq!(
            while_connection_held(pool.clone(), settings.network_proxy_settings())
                .await
                .expect("read after lock release"),
            None
        );
    });
}

/// Proxy writes yield during database contention and retain their authoritative response.
#[test]
fn proxy_write_does_not_block_async_progress() {
    run_test(async {
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::in_memory(),
                &default_migration_catalog().expect("migration catalog"),
            )
            .expect("single-connection database");
        let settings = Settings::new(pool.clone());
        let proxy = NetworkProxySettings {
            host: "proxy.example.test".to_string(),
            port: 8080,
            username: None,
            password: None,
        };
        assert_eq!(
            while_connection_held(
                pool.clone(),
                settings.set_network_proxy_settings(proxy.clone())
            )
            .await
            .expect("write after lock release"),
            proxy
        );
        assert_eq!(
            settings
                .network_proxy_settings()
                .await
                .expect("stored proxy"),
            Some(proxy)
        );
    });
}

/// Proxy deletion yields during database contention and removes the durable preference.
#[test]
fn proxy_clear_does_not_block_async_progress() {
    run_test(async {
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::in_memory(),
                &default_migration_catalog().expect("migration catalog"),
            )
            .expect("single-connection database");
        let settings = Settings::new(pool.clone());
        settings
            .set_network_proxy_settings(NetworkProxySettings {
                host: "proxy.example.test".to_string(),
                port: 8080,
                username: None,
                password: None,
            })
            .await
            .expect("seed proxy");
        while_connection_held(pool.clone(), settings.clear_network_proxy_settings())
            .await
            .expect("clear after lock release");
        assert_eq!(
            settings
                .network_proxy_settings()
                .await
                .expect("cleared proxy"),
            None
        );
    });
}

/// The Cloud server origin survives reopening and can be forgotten again.
#[test]
fn cloud_server_round_trips_and_clears() {
    run_test(async {
        let temporary = TempDir::new().expect("settings fixture");
        let path = temporary.path().join("ora.sqlite3");
        let (_pool, settings) = open_settings(&path);
        assert_eq!(settings.cloud_server().await.expect("read empty"), None);
        settings
            .set_cloud_server("https://cloud.example".to_owned())
            .await
            .expect("save origin");
        let (_reopened_pool, reopened) = open_settings(&path);
        assert_eq!(
            reopened.cloud_server().await.expect("read saved"),
            Some("https://cloud.example".to_owned())
        );
        reopened.clear_cloud_server().await.expect("clear origin");
        assert_eq!(reopened.cloud_server().await.expect("read cleared"), None);
    });
}
