use super::Settings;
use crate::BackendError;
use ora_application::{
    DeveloperMode, NetworkProxyMode, NetworkProxyPreferences, NetworkProxySettings,
};
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
                .network_proxy_check_url()
                .await
                .expect_err("probe URL read fails"),
        );
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

/// A new supported mode can overwrite unreadable preferences without touching the endpoint.
#[test]
fn saving_non_manual_proxy_recovers_from_unreadable_preferences() {
    run_test(async {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("ora.sqlite3");
        let (_pool, settings) = open_settings(&path);
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute(
                "INSERT INTO user_config (key, value) VALUES ('network_proxy_settings', 'broken-json')",
                [],
            )
            .unwrap();
        for (previous, mode) in [
            ("future-mode", NetworkProxyMode::System),
            ("manual", NetworkProxyMode::Direct),
        ] {
            connection
                .execute(
                    "INSERT INTO user_config (key, value) VALUES ('network_proxy_mode', ?1)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    [previous],
                )
                .unwrap();
            assert!(settings.network_proxy_preferences().await.is_err());
            let expected = NetworkProxyPreferences { mode, manual: None };
            assert_eq!(
                settings
                    .set_network_proxy_preferences(expected.clone())
                    .await
                    .unwrap(),
                expected,
            );
            assert_eq!(
                settings.network_proxy_preferences().await.unwrap(),
                expected,
            );
            assert!(settings.network_proxy_settings().await.is_err());
            let raw: String = connection
                .query_row(
                    "SELECT value FROM user_config WHERE key = 'network_proxy_settings'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(raw, "broken-json");
        }
    });
}

/// Legacy prefixed settings still resolve, and saving canonicalizes without partial writes.
#[test]
fn manual_proxy_normalizes_on_save_and_preserves_the_old_value_on_invalid_input() {
    run_test(async {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("ora.sqlite3");
        let (_pool, settings) = open_settings(&path);
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute(
                "INSERT INTO user_config (key, value) VALUES ('network_proxy_settings', ?1)",
                [r#"{"host":"http://127.0.0.1","port":7890,"username":null,"password":null}"#],
            )
            .unwrap();
        let target = url::Url::parse("https://origin.example/").unwrap();
        let legacy = settings.network_proxy_policy().await.unwrap();
        assert_eq!(
            legacy
                .route(&target)
                .unwrap()
                .endpoint
                .as_ref()
                .map(url::Url::as_str),
            Some("http://127.0.0.1:7890/")
        );

        for (input, canonical) in [
            (" http://127.0.0.1:80/ ", "127.0.0.1"),
            (" HTTPS://PROXY.EXAMPLE:443/ ", "https://proxy.example"),
            ("socks5://[::1]:1080/", "socks5://[::1]"),
            ("socks5h://proxy.example/", "socks5h://proxy.example"),
        ] {
            let mut endpoint = NetworkProxySettings {
                host: input.to_owned(),
                port: 7890,
                username: None,
                password: None,
            };
            let manual = |endpoint| NetworkProxyPreferences {
                mode: NetworkProxyMode::Manual,
                manual: Some(endpoint),
            };
            let request = manual(endpoint.clone());
            let before_save = Settings::network_proxy_policy_for(&request).unwrap();
            endpoint.host = canonical.to_owned();
            let expected = manual(endpoint.clone());
            assert_eq!(
                settings
                    .set_network_proxy_preferences(request)
                    .await
                    .unwrap(),
                expected
            );
            assert_eq!(
                settings.network_proxy_settings().await.unwrap(),
                Some(endpoint.clone())
            );
            let after_save = settings.network_proxy_policy().await.unwrap();
            assert_eq!(
                before_save.route(&target).unwrap().endpoint,
                after_save.route(&target).unwrap().endpoint
            );
            endpoint.host = "https://proxy.example/path".to_owned();
            assert!(
                settings
                    .set_network_proxy_preferences(manual(endpoint))
                    .await
                    .is_err()
            );
            assert_eq!(
                settings.network_proxy_preferences().await.unwrap(),
                expected
            );
        }
    });
}

/// Invalid endpoints leave saved settings intact and fail closed when read from storage.
#[test]
fn invalid_manual_proxy_endpoints_are_rejected_on_save_and_route_selection() {
    run_test(async {
        for host in ["ftp://h", "http://u:p@h", "h/path"] {
            let temporary = TempDir::new().unwrap();
            let path = temporary.path().join("ora.sqlite3");
            let (_pool, settings) = open_settings(&path);
            let original = NetworkProxyPreferences {
                mode: NetworkProxyMode::Manual,
                manual: Some(NetworkProxySettings {
                    host: "proxy.example".to_owned(),
                    port: 7890,
                    username: None,
                    password: None,
                }),
            };
            settings
                .set_network_proxy_preferences(original.clone())
                .await
                .unwrap();

            let invalid = NetworkProxySettings {
                host: host.to_owned(),
                port: 7890,
                username: None,
                password: None,
            };
            assert!(
                settings
                    .set_network_proxy_preferences(NetworkProxyPreferences {
                        mode: NetworkProxyMode::Manual,
                        manual: Some(invalid.clone()),
                    })
                    .await
                    .is_err(),
                "saving {host} should fail"
            );
            assert_eq!(
                settings.network_proxy_preferences().await.unwrap(),
                original
            );

            rusqlite::Connection::open(&path)
                .unwrap()
                .execute(
                    "UPDATE user_config SET value = ?1 WHERE key = 'network_proxy_settings'",
                    [serde_json::to_string(&invalid).unwrap()],
                )
                .unwrap();
            assert_eq!(
                settings.network_proxy_settings().await.unwrap(),
                Some(invalid)
            );
            assert!(
                settings.network_proxy_policy().await.is_err(),
                "routing through {host} should fail"
            );
        }
    });
}

/// Probe targets persist independently, and invalid input or failed writes preserve the old value.
#[test]
fn proxy_check_url_survives_reopen_and_rejects_destructive_fallbacks() {
    run_test(async {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("ora.sqlite3");
        let (_pool, settings) = open_settings(&path);
        assert_eq!(
            settings.network_proxy_check_url().await.unwrap(),
            "https://google.com"
        );
        let connection = rusqlite::Connection::open(&path).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM user_config", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            settings
                .set_network_proxy_check_url(" https://custom.example/probe?q=1 ".to_owned())
                .await
                .unwrap(),
            "https://custom.example/probe?q=1"
        );
        for invalid in ["", "file:///secret", "not a URL", "https://"] {
            assert!(
                settings
                    .set_network_proxy_check_url(invalid.to_owned())
                    .await
                    .is_err()
            );
        }
        connection.execute_batch("CREATE TRIGGER reject_probe_url BEFORE INSERT ON user_config WHEN NEW.key = 'network_proxy_check_url' BEGIN SELECT RAISE(ABORT, 'fixture write failure'); END;").unwrap();
        assert_storage_failure(
            settings
                .set_network_proxy_check_url("https://replacement.example".to_owned())
                .await
                .unwrap_err(),
        );
        let (_pool, reopened) = open_settings(&path);
        assert_eq!(
            (
                reopened.network_proxy_check_url().await.unwrap(),
                reopened.network_proxy_settings().await.unwrap()
            ),
            ("https://custom.example/probe?q=1".to_owned(), None)
        );
        connection.execute("UPDATE user_config SET value = 'unsupported target' WHERE key = 'network_proxy_check_url'", []).unwrap();
        assert!(reopened.network_proxy_check_url().await.is_err());
        assert_eq!(
            connection
                .query_row(
                    "SELECT value FROM user_config WHERE key = 'network_proxy_check_url'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "unsupported target"
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
