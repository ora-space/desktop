use super::{
    PluginLaunchRequest, PluginLifecycle, PluginLifecycleConfig, PluginLifecycleError,
    PluginRuntime, PluginRuntimeExit, PluginRuntimeFailure, PluginRuntimeLauncher,
    PluginStatusPublisher,
};
use ora_application::{Clock, PluginStateRepository};
use ora_contracts::{
    ActivatePluginRequest, ActivatePluginResponse, DisablePluginRequest, DisablePluginResponse,
    EnablePluginRequest, EnablePluginResponse, InstalledPlugin, InstalledPluginAgent,
    ListInstalledPluginsResponse, PluginRuntimeStatus, ScanPluginsRequest, ScanPluginsResponse,
    StopPluginRequest, StopPluginResponse, UninstallPluginRequest, UninstallPluginResponse,
};
use ora_db::{
    DatabaseBootstrapper, DatabaseLocation, SqlitePluginStateRepository, default_migration_catalog,
};
use ora_domain::{PluginEnabledState, PluginId};
use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::fs;
use std::future::{Future, pending};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use tokio::sync::{mpsc, oneshot};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::prelude::*;

/// Supplies deterministic lifecycle timestamps without mutating process-global time.
#[derive(Clone, Copy, Debug)]
struct FixedClock;

impl Clock for FixedClock {
    /// Returns one stable timestamp for lifecycle repository writes.
    fn now_timestamp_millis(&self) -> i64 {
        100
    }
}

/// Installs a test-thread TRACE subscriber for the full lifetime of an async test future.
fn trace_logging_guard() -> tracing::subscriber::DefaultGuard {
    let subscriber = tracing_subscriber::registry().with(LevelFilter::TRACE);
    tracing::subscriber::set_default(subscriber)
}

/// Opens lifecycle state for tests that never cross the external runtime boundary.
fn open_without_runtime(
    data_directory: &Path,
    repository: SqlitePluginStateRepository,
) -> PluginLifecycle<
    SqlitePluginStateRepository,
    FixedClock,
    UnusedRuntimeLauncher,
    NoopStatusPublisher,
> {
    PluginLifecycle::open(
        PluginLifecycleConfig {
            data_directory: data_directory.to_path_buf(),
            deno_path: PathBuf::from("deno"),
        },
        repository,
        FixedClock,
        UnusedRuntimeLauncher,
        NoopStatusPublisher,
    )
    .expect("open plugin lifecycle")
}

/// Rejects accidental launches in lifecycle tests that do not exercise process behavior.
#[derive(Clone)]
struct UnusedRuntimeLauncher;

impl PluginRuntimeLauncher for UnusedRuntimeLauncher {
    type Runtime = FakeRuntime;

    /// Fails visibly if a non-runtime test unexpectedly crosses the process seam.
    fn launch(
        &self,
        _request: PluginLaunchRequest,
    ) -> impl Future<Output = Result<Self::Runtime, PluginRuntimeFailure>> + Send {
        async { Err(PluginRuntimeFailure::new("runtime launch was not expected")) }
    }
}

/// Discards invalidations in tests that assert only returned lifecycle snapshots.
#[derive(Clone)]
struct NoopStatusPublisher;

impl PluginStatusPublisher for NoopStatusPublisher {
    /// Intentionally ignores an invalidation outside event-focused tests.
    fn publish_status_changed(&self, _plugin_id: &PluginId) {}
}

/// Verifies startup discovery exposes an unpersisted plugin as disabled and stopped.
#[test]
fn opens_with_discovered_plugins_disabled_and_stopped() {
    with_trace_logging(|| {
        let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
        write_plugin_package(temp_dir.path(), "example", "ora.example", "Example");
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(temp_dir.path().join("ora.sqlite3")),
                &default_migration_catalog().expect("build migration catalog"),
            )
            .expect("bootstrap plugin lifecycle database");
        let lifecycle =
            open_without_runtime(temp_dir.path(), SqlitePluginStateRepository::new(pool));

        assert_eq!(
            lifecycle.list_installed_plugins(),
            ListInstalledPluginsResponse {
                plugins: vec![expected_plugin(/*enabled*/ false)],
            },
        );
    });
}

/// Verifies startup joins a discovered package with the user's durable enabled intent.
#[test]
fn opens_with_persisted_plugin_eligibility() {
    with_trace_logging(|| {
        let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
        write_plugin_package(temp_dir.path(), "example", "ora.example", "Example");
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(temp_dir.path().join("ora.sqlite3")),
                &default_migration_catalog().expect("build migration catalog"),
            )
            .expect("bootstrap plugin lifecycle database");
        let repository = SqlitePluginStateRepository::new(pool);
        repository
            .set_plugin_enabled(
                &PluginId::new("ora.example"),
                PluginEnabledState::Enabled,
                20,
            )
            .expect("persist enabled plugin");

        let lifecycle = open_without_runtime(temp_dir.path(), repository);

        assert_eq!(
            lifecycle.list_installed_plugins(),
            ListInstalledPluginsResponse {
                plugins: vec![expected_plugin(/*enabled*/ true)],
            },
        );
    });
}

/// Verifies enabling persists eligibility and leaves the process stopped.
#[tokio::test]
async fn enables_plugin_without_activating_it() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "example", "ora.example", "Example");
    let pool = DatabaseBootstrapper::system()
        .bootstrap_repository_pool(
            &DatabaseLocation::path(temp_dir.path().join("ora.sqlite3")),
            &default_migration_catalog().expect("build migration catalog"),
        )
        .expect("bootstrap plugin lifecycle database");
    let lifecycle = open_without_runtime(temp_dir.path(), SqlitePluginStateRepository::new(pool));

    let response = lifecycle
        .enable_plugin(EnablePluginRequest {
            plugin_id: "ora.example".to_string(),
        })
        .await
        .expect("enable plugin");
    let expected_plugin = expected_plugin(/*enabled*/ true);

    assert_eq!(
        (response, lifecycle.list_installed_plugins()),
        (
            EnablePluginResponse {
                plugin: expected_plugin.clone(),
            },
            ListInstalledPluginsResponse {
                plugins: vec![expected_plugin],
            },
        ),
    );
}

/// Verifies disabling a stopped plugin persists ineligibility without changing runtime state.
#[tokio::test]
async fn disables_stopped_plugin() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "example", "ora.example", "Example");
    let pool = DatabaseBootstrapper::system()
        .bootstrap_repository_pool(
            &DatabaseLocation::path(temp_dir.path().join("ora.sqlite3")),
            &default_migration_catalog().expect("build migration catalog"),
        )
        .expect("bootstrap plugin lifecycle database");
    let repository = SqlitePluginStateRepository::new(pool);
    repository
        .set_plugin_enabled(
            &PluginId::new("ora.example"),
            PluginEnabledState::Enabled,
            20,
        )
        .expect("persist enabled plugin");
    let lifecycle = open_without_runtime(temp_dir.path(), repository);

    let response = lifecycle
        .disable_plugin(DisablePluginRequest {
            plugin_id: "ora.example".to_string(),
        })
        .await
        .expect("disable plugin");
    let expected_plugin = expected_plugin(/*enabled*/ false);

    assert_eq!(
        (response, lifecycle.list_installed_plugins()),
        (
            DisablePluginResponse {
                plugin: expected_plugin.clone(),
            },
            ListInstalledPluginsResponse {
                plugins: vec![expected_plugin],
            },
        ),
    );
}

/// Verifies disabling a never-enabled plugin preserves missing durable state as the default.
#[tokio::test]
async fn disabling_never_enabled_plugin_does_not_create_durable_state() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "example", "ora.example", "Example");
    let pool = DatabaseBootstrapper::system()
        .bootstrap_repository_pool(
            &DatabaseLocation::path(temp_dir.path().join("ora.sqlite3")),
            &default_migration_catalog().expect("build migration catalog"),
        )
        .expect("bootstrap plugin lifecycle database");
    let repository = SqlitePluginStateRepository::new(pool);
    let repository_probe = repository.clone();
    let lifecycle = open_without_runtime(temp_dir.path(), repository);

    let response = lifecycle
        .disable_plugin(DisablePluginRequest {
            plugin_id: "ora.example".to_string(),
        })
        .await
        .expect("disable never-enabled plugin");

    assert_eq!(
        (
            response,
            repository_probe
                .find_plugin_state(&PluginId::new("ora.example"))
                .expect("read durable plugin state"),
        ),
        (
            DisablePluginResponse {
                plugin: expected_plugin(/*enabled*/ false),
            },
            None,
        ),
    );
}

/// Verifies only an explicit scan observes packages added after lifecycle startup.
#[tokio::test]
async fn scans_new_packages_without_rescanning_cached_queries() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    let pool = DatabaseBootstrapper::system()
        .bootstrap_repository_pool(
            &DatabaseLocation::path(temp_dir.path().join("ora.sqlite3")),
            &default_migration_catalog().expect("build migration catalog"),
        )
        .expect("bootstrap plugin lifecycle database");
    let lifecycle = open_without_runtime(temp_dir.path(), SqlitePluginStateRepository::new(pool));
    write_plugin_package(temp_dir.path(), "example", "ora.example", "Example");

    let before_scan = lifecycle.list_installed_plugins();
    let scan_response = lifecycle
        .scan_plugins(ScanPluginsRequest {})
        .await
        .expect("scan plugins");
    let expected_plugin = expected_plugin(/*enabled*/ false);

    assert_eq!(
        (
            before_scan,
            scan_response,
            lifecycle.list_installed_plugins(),
        ),
        (
            ListInstalledPluginsResponse {
                plugins: Vec::new(),
            },
            ScanPluginsResponse {
                plugins: vec![expected_plugin.clone()],
            },
            ListInstalledPluginsResponse {
                plugins: vec![expected_plugin],
            },
        ),
    );
}

/// Verifies startup removes durable rows whose package is absent from filesystem discovery.
#[tokio::test]
async fn startup_reconciliation_deletes_orphaned_plugin_state() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    let pool = DatabaseBootstrapper::system()
        .bootstrap_repository_pool(
            &DatabaseLocation::path(temp_dir.path().join("ora.sqlite3")),
            &default_migration_catalog().expect("build migration catalog"),
        )
        .expect("bootstrap plugin lifecycle database");
    let repository = SqlitePluginStateRepository::new(pool);
    repository
        .set_plugin_enabled(
            &PluginId::new("ora.example"),
            PluginEnabledState::Enabled,
            20,
        )
        .expect("persist orphaned plugin state");
    let lifecycle = open_without_runtime(temp_dir.path(), repository);

    write_plugin_package(temp_dir.path(), "example", "ora.example", "Example");
    let response = lifecycle
        .scan_plugins(ScanPluginsRequest {})
        .await
        .expect("scan restored package");

    assert_eq!(
        response,
        ScanPluginsResponse {
            plugins: vec![expected_plugin(/*enabled*/ false)],
        },
    );
}

/// Verifies manual reconciliation removes state for packages deleted outside Ora.
#[tokio::test]
async fn scan_reconciliation_deletes_orphaned_plugin_state() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "example", "ora.example", "Example");
    let pool = DatabaseBootstrapper::system()
        .bootstrap_repository_pool(
            &DatabaseLocation::path(temp_dir.path().join("ora.sqlite3")),
            &default_migration_catalog().expect("build migration catalog"),
        )
        .expect("bootstrap plugin lifecycle database");
    let repository = SqlitePluginStateRepository::new(pool);
    repository
        .set_plugin_enabled(
            &PluginId::new("ora.example"),
            PluginEnabledState::Enabled,
            20,
        )
        .expect("persist enabled plugin");
    let lifecycle = open_without_runtime(temp_dir.path(), repository);

    fs::remove_dir_all(temp_dir.path().join("plugins").join("example"))
        .expect("remove plugin outside Ora");
    lifecycle
        .scan_plugins(ScanPluginsRequest {})
        .await
        .expect("reconcile deleted package");
    write_plugin_package(temp_dir.path(), "example", "ora.example", "Example");
    let restored = lifecycle
        .scan_plugins(ScanPluginsRequest {})
        .await
        .expect("scan restored package");

    assert_eq!(
        restored,
        ScanPluginsResponse {
            plugins: vec![expected_plugin(/*enabled*/ false)],
        },
    );
}

/// Verifies reconciliation stops an externally orphaned runtime before clearing its state.
#[tokio::test]
async fn scan_stops_runtime_for_package_deleted_outside_ora() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "example", "ora.example", "Example");
    let pool = DatabaseBootstrapper::system()
        .bootstrap_repository_pool(
            &DatabaseLocation::path(temp_dir.path().join("ora.sqlite3")),
            &default_migration_catalog().expect("build migration catalog"),
        )
        .expect("bootstrap plugin lifecycle database");
    let repository = SqlitePluginStateRepository::new(pool);
    let repository_probe = repository.clone();
    let (runtime, mut stop_started, release_stop) = ControllableStopRuntime::new();
    let lifecycle = PluginLifecycle::open(
        PluginLifecycleConfig {
            data_directory: temp_dir.path().to_path_buf(),
            deno_path: PathBuf::from("deno"),
        },
        repository,
        FixedClock,
        ImmediateRuntimeLauncher { runtime },
        NoopStatusPublisher,
    )
    .expect("open plugin lifecycle");
    lifecycle
        .enable_plugin(EnablePluginRequest {
            plugin_id: "ora.example".to_string(),
        })
        .await
        .expect("enable plugin");
    lifecycle
        .activate_plugin(ActivatePluginRequest {
            plugin_id: "ora.example".to_string(),
        })
        .await
        .expect("activate plugin");
    tokio::task::yield_now().await;
    fs::remove_dir_all(temp_dir.path().join("plugins").join("example"))
        .expect("remove plugin outside Ora");

    let scan_lifecycle = lifecycle.clone();
    let scan_task =
        tokio::spawn(async move { scan_lifecycle.scan_plugins(ScanPluginsRequest {}).await });
    assert_eq!(stop_started.recv().await, Some(()));
    assert_eq!(scan_task.is_finished(), false);
    release_stop.send(()).expect("release runtime stop");
    let response = scan_task
        .await
        .expect("join scan task")
        .expect("scan plugins");

    assert_eq!(
        (
            response,
            repository_probe
                .find_plugin_state(&PluginId::new("ora.example"))
                .expect("read reconciled plugin state"),
        ),
        (
            ScanPluginsResponse {
                plugins: Vec::new(),
            },
            None,
        ),
    );
}

/// Verifies an explicit scan reapplies durable eligibility to an existing cached package.
#[tokio::test]
async fn scan_reloads_durable_eligibility_for_existing_plugin() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "example", "ora.example", "Example");
    let pool = DatabaseBootstrapper::system()
        .bootstrap_repository_pool(
            &DatabaseLocation::path(temp_dir.path().join("ora.sqlite3")),
            &default_migration_catalog().expect("build migration catalog"),
        )
        .expect("bootstrap plugin lifecycle database");
    let repository = SqlitePluginStateRepository::new(pool);
    let repository_probe = repository.clone();
    let lifecycle = open_without_runtime(temp_dir.path(), repository);
    repository_probe
        .set_plugin_enabled(
            &PluginId::new("ora.example"),
            PluginEnabledState::Enabled,
            20,
        )
        .expect("persist external eligibility change");

    let response = lifecycle
        .scan_plugins(ScanPluginsRequest {})
        .await
        .expect("reconcile durable eligibility");

    assert_eq!(
        (response, lifecycle.list_installed_plugins()),
        (
            ScanPluginsResponse {
                plugins: vec![expected_plugin(/*enabled*/ true)],
            },
            ListInstalledPluginsResponse {
                plugins: vec![expected_plugin(/*enabled*/ true)],
            },
        ),
    );
}

/// Verifies reconciliation stops a runtime when its durable eligibility row disappears.
#[tokio::test]
async fn scan_stops_runtime_invalidated_by_missing_durable_state() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "example", "ora.example", "Example");
    let pool = DatabaseBootstrapper::system()
        .bootstrap_repository_pool(
            &DatabaseLocation::path(temp_dir.path().join("ora.sqlite3")),
            &default_migration_catalog().expect("build migration catalog"),
        )
        .expect("bootstrap plugin lifecycle database");
    let repository = SqlitePluginStateRepository::new(pool);
    let repository_probe = repository.clone();
    let (runtime, mut stop_started, release_stop) = ControllableStopRuntime::new();
    let (publisher, mut events) = RecordingStatusPublisher::new();
    let lifecycle = PluginLifecycle::open(
        PluginLifecycleConfig {
            data_directory: temp_dir.path().to_path_buf(),
            deno_path: PathBuf::from("deno"),
        },
        repository,
        FixedClock,
        ImmediateRuntimeLauncher { runtime },
        publisher,
    )
    .expect("open plugin lifecycle");
    lifecycle
        .enable_plugin(EnablePluginRequest {
            plugin_id: "ora.example".to_string(),
        })
        .await
        .expect("enable plugin");
    assert_eq!(events.recv().await, Some(PluginId::new("ora.example")));
    lifecycle
        .activate_plugin(ActivatePluginRequest {
            plugin_id: "ora.example".to_string(),
        })
        .await
        .expect("activate plugin");
    assert_eq!(events.recv().await, Some(PluginId::new("ora.example")));
    assert_eq!(events.recv().await, Some(PluginId::new("ora.example")));
    repository_probe
        .delete_plugin_state(&PluginId::new("ora.example"))
        .expect("delete durable state outside lifecycle");

    let scan_lifecycle = lifecycle.clone();
    let scan_task =
        tokio::spawn(async move { scan_lifecycle.scan_plugins(ScanPluginsRequest {}).await });
    assert_eq!(stop_started.recv().await, Some(()));
    assert_eq!(scan_task.is_finished(), false);
    release_stop.send(()).expect("release runtime stop");
    let response = scan_task
        .await
        .expect("join scan task")
        .expect("scan plugins");
    assert_eq!(events.recv().await, Some(PluginId::new("ora.example")));

    assert_eq!(
        (response, lifecycle.list_installed_plugins()),
        (
            ScanPluginsResponse {
                plugins: vec![expected_plugin(/*enabled*/ false)],
            },
            ListInstalledPluginsResponse {
                plugins: vec![expected_plugin(/*enabled*/ false)],
            },
        ),
    );
}

/// Verifies activation returns starting before asynchronously transitioning to running.
#[tokio::test]
async fn activates_enabled_plugin_and_publishes_each_transition() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "example", "ora.example", "Example");
    let pool = DatabaseBootstrapper::system()
        .bootstrap_repository_pool(
            &DatabaseLocation::path(temp_dir.path().join("ora.sqlite3")),
            &default_migration_catalog().expect("build migration catalog"),
        )
        .expect("bootstrap plugin lifecycle database");
    let (launcher, mut launched, release_launch) = ControllableRuntimeLauncher::new();
    let (publisher, mut events) = RecordingStatusPublisher::new();
    let lifecycle = PluginLifecycle::open(
        PluginLifecycleConfig {
            data_directory: temp_dir.path().to_path_buf(),
            deno_path: PathBuf::from("deno"),
        },
        SqlitePluginStateRepository::new(pool),
        FixedClock,
        launcher,
        publisher,
    )
    .expect("open plugin lifecycle");
    lifecycle
        .enable_plugin(EnablePluginRequest {
            plugin_id: "ora.example".to_string(),
        })
        .await
        .expect("enable plugin");
    assert_eq!(events.recv().await, Some(PluginId::new("ora.example")));

    let response = lifecycle
        .activate_plugin(ActivatePluginRequest {
            plugin_id: "ora.example".to_string(),
        })
        .await
        .expect("activate plugin");
    assert_eq!(
        response,
        ActivatePluginResponse {
            plugin: expected_plugin_with_runtime(
                PluginEnabledState::Enabled,
                PluginRuntimeStatus::Starting,
            ),
        },
    );
    assert_eq!(events.recv().await, Some(PluginId::new("ora.example")));
    assert_eq!(
        launched.recv().await,
        Some(PluginLaunchRequest {
            plugin_id: PluginId::new("ora.example"),
            deno_path: PathBuf::from("deno"),
            entrypoint: temp_dir
                .path()
                .join("plugins")
                .join("example")
                .join("dist")
                .join("index.js"),
        }),
    );

    release_launch.send(()).expect("release runtime launch");
    assert_eq!(events.recv().await, Some(PluginId::new("ora.example")));
    assert_eq!(
        lifecycle.list_installed_plugins(),
        ListInstalledPluginsResponse {
            plugins: vec![expected_plugin_with_runtime(
                PluginEnabledState::Enabled,
                PluginRuntimeStatus::Running,
            )],
        },
    );
}

/// Verifies explicit stop waits for process exit and preserves durable eligibility.
#[tokio::test]
async fn stops_running_plugin_without_disabling_it() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "example", "ora.example", "Example");
    let pool = DatabaseBootstrapper::system()
        .bootstrap_repository_pool(
            &DatabaseLocation::path(temp_dir.path().join("ora.sqlite3")),
            &default_migration_catalog().expect("build migration catalog"),
        )
        .expect("bootstrap plugin lifecycle database");
    let (runtime, mut stop_started, release_stop) = ControllableStopRuntime::new();
    let launcher = ImmediateRuntimeLauncher { runtime };
    let (publisher, mut events) = RecordingStatusPublisher::new();
    let lifecycle = PluginLifecycle::open(
        PluginLifecycleConfig {
            data_directory: temp_dir.path().to_path_buf(),
            deno_path: PathBuf::from("deno"),
        },
        SqlitePluginStateRepository::new(pool),
        FixedClock,
        launcher,
        publisher,
    )
    .expect("open plugin lifecycle");
    lifecycle
        .enable_plugin(EnablePluginRequest {
            plugin_id: "ora.example".to_string(),
        })
        .await
        .expect("enable plugin");
    assert_eq!(events.recv().await, Some(PluginId::new("ora.example")));
    lifecycle
        .activate_plugin(ActivatePluginRequest {
            plugin_id: "ora.example".to_string(),
        })
        .await
        .expect("activate plugin");
    assert_eq!(events.recv().await, Some(PluginId::new("ora.example")));
    assert_eq!(events.recv().await, Some(PluginId::new("ora.example")));

    let stop_lifecycle = lifecycle.clone();
    let stop_task = tokio::spawn(async move {
        stop_lifecycle
            .stop_plugin(StopPluginRequest {
                plugin_id: "ora.example".to_string(),
            })
            .await
    });
    assert_eq!(stop_started.recv().await, Some(()));
    release_stop.send(()).expect("release runtime stop");
    let response = stop_task
        .await
        .expect("join stop task")
        .expect("stop plugin");

    assert_eq!(events.recv().await, Some(PluginId::new("ora.example")));
    let expected_plugin = expected_plugin(/*enabled*/ true);
    assert_eq!(
        (response, lifecycle.list_installed_plugins()),
        (
            StopPluginResponse {
                plugin: expected_plugin.clone(),
            },
            ListInstalledPluginsResponse {
                plugins: vec![expected_plugin],
            },
        ),
    );
}

/// Verifies disabling a running plugin waits for shutdown before clearing eligibility.
#[tokio::test]
async fn disabling_running_plugin_stops_it_first() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "example", "ora.example", "Example");
    let pool = DatabaseBootstrapper::system()
        .bootstrap_repository_pool(
            &DatabaseLocation::path(temp_dir.path().join("ora.sqlite3")),
            &default_migration_catalog().expect("build migration catalog"),
        )
        .expect("bootstrap plugin lifecycle database");
    let (runtime, mut stop_started, release_stop) = ControllableStopRuntime::new();
    let launcher = ImmediateRuntimeLauncher { runtime };
    let (publisher, mut events) = RecordingStatusPublisher::new();
    let lifecycle = PluginLifecycle::open(
        PluginLifecycleConfig {
            data_directory: temp_dir.path().to_path_buf(),
            deno_path: PathBuf::from("deno"),
        },
        SqlitePluginStateRepository::new(pool),
        FixedClock,
        launcher,
        publisher,
    )
    .expect("open plugin lifecycle");
    lifecycle
        .enable_plugin(EnablePluginRequest {
            plugin_id: "ora.example".to_string(),
        })
        .await
        .expect("enable plugin");
    assert_eq!(events.recv().await, Some(PluginId::new("ora.example")));
    lifecycle
        .activate_plugin(ActivatePluginRequest {
            plugin_id: "ora.example".to_string(),
        })
        .await
        .expect("activate plugin");
    assert_eq!(events.recv().await, Some(PluginId::new("ora.example")));
    assert_eq!(events.recv().await, Some(PluginId::new("ora.example")));

    let disable_lifecycle = lifecycle.clone();
    let disable_task = tokio::spawn(async move {
        disable_lifecycle
            .disable_plugin(DisablePluginRequest {
                plugin_id: "ora.example".to_string(),
            })
            .await
    });
    assert_eq!(stop_started.recv().await, Some(()));
    release_stop.send(()).expect("release runtime stop");
    let response = disable_task
        .await
        .expect("join disable task")
        .expect("disable running plugin");

    assert_eq!(events.recv().await, Some(PluginId::new("ora.example")));
    let expected_plugin = expected_plugin(/*enabled*/ false);
    assert_eq!(
        (response, lifecycle.list_installed_plugins()),
        (
            DisablePluginResponse {
                plugin: expected_plugin.clone(),
            },
            ListInstalledPluginsResponse {
                plugins: vec![expected_plugin],
            },
        ),
    );
}

/// Verifies disable queues behind an in-flight launch and waits for the resulting runtime stop.
#[tokio::test]
async fn queues_disable_behind_starting_activation() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "example", "ora.example", "Example");
    let pool = DatabaseBootstrapper::system()
        .bootstrap_repository_pool(
            &DatabaseLocation::path(temp_dir.path().join("ora.sqlite3")),
            &default_migration_catalog().expect("build migration catalog"),
        )
        .expect("bootstrap plugin lifecycle database");
    let (runtime, mut stop_started, release_stop) = ControllableStopRuntime::new();
    let (launcher, release_launch) = QueuedRuntimeLauncher::new(runtime);
    let (publisher, _events) = RecordingStatusPublisher::new();
    let lifecycle = PluginLifecycle::open(
        PluginLifecycleConfig {
            data_directory: temp_dir.path().to_path_buf(),
            deno_path: PathBuf::from("deno"),
        },
        SqlitePluginStateRepository::new(pool),
        FixedClock,
        launcher,
        publisher,
    )
    .expect("open plugin lifecycle");
    lifecycle
        .enable_plugin(EnablePluginRequest {
            plugin_id: "ora.example".to_string(),
        })
        .await
        .expect("enable plugin");
    lifecycle
        .activate_plugin(ActivatePluginRequest {
            plugin_id: "ora.example".to_string(),
        })
        .await
        .expect("activate plugin");

    let disable_lifecycle = lifecycle.clone();
    let disable_task = tokio::spawn(async move {
        disable_lifecycle
            .disable_plugin(DisablePluginRequest {
                plugin_id: "ora.example".to_string(),
            })
            .await
    });
    tokio::task::yield_now().await;
    assert_eq!(disable_task.is_finished(), false);

    release_launch.send(()).expect("release runtime launch");
    assert_eq!(stop_started.recv().await, Some(()));
    release_stop.send(()).expect("release runtime stop");
    let response = disable_task
        .await
        .expect("join disable task")
        .expect("disable plugin after launch");

    assert_eq!(
        response,
        DisablePluginResponse {
            plugin: expected_plugin(/*enabled*/ false),
        },
    );
}

/// Verifies uninstall waits for runtime exit before deleting the package and cached identity.
#[tokio::test]
async fn uninstalls_running_plugin_after_stopping_it() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "example", "ora.example", "Example");
    let package_root = temp_dir.path().join("plugins").join("example");
    let pool = DatabaseBootstrapper::system()
        .bootstrap_repository_pool(
            &DatabaseLocation::path(temp_dir.path().join("ora.sqlite3")),
            &default_migration_catalog().expect("build migration catalog"),
        )
        .expect("bootstrap plugin lifecycle database");
    let (runtime, mut stop_started, release_stop) = ControllableStopRuntime::new();
    let launcher = ImmediateRuntimeLauncher { runtime };
    let (publisher, mut events) = RecordingStatusPublisher::new();
    let lifecycle = PluginLifecycle::open(
        PluginLifecycleConfig {
            data_directory: temp_dir.path().to_path_buf(),
            deno_path: PathBuf::from("deno"),
        },
        SqlitePluginStateRepository::new(pool),
        FixedClock,
        launcher,
        publisher,
    )
    .expect("open plugin lifecycle");
    lifecycle
        .enable_plugin(EnablePluginRequest {
            plugin_id: "ora.example".to_string(),
        })
        .await
        .expect("enable plugin");
    assert_eq!(events.recv().await, Some(PluginId::new("ora.example")));
    lifecycle
        .activate_plugin(ActivatePluginRequest {
            plugin_id: "ora.example".to_string(),
        })
        .await
        .expect("activate plugin");
    assert_eq!(events.recv().await, Some(PluginId::new("ora.example")));
    assert_eq!(events.recv().await, Some(PluginId::new("ora.example")));

    let uninstall_lifecycle = lifecycle.clone();
    let uninstall_task = tokio::spawn(async move {
        uninstall_lifecycle
            .uninstall_plugin(UninstallPluginRequest {
                plugin_id: "ora.example".to_string(),
            })
            .await
    });
    assert_eq!(stop_started.recv().await, Some(()));
    assert_eq!(package_root.is_dir(), true);
    release_stop.send(()).expect("release runtime stop");
    let response = uninstall_task
        .await
        .expect("join uninstall task")
        .expect("uninstall plugin");

    assert_eq!(
        (
            response,
            package_root.exists(),
            lifecycle.list_installed_plugins(),
        ),
        (
            UninstallPluginResponse {
                plugin_id: "ora.example".to_string(),
            },
            false,
            ListInstalledPluginsResponse {
                plugins: Vec::new(),
            },
        ),
    );
}

/// Verifies a failed package removal still leaves the stopped runtime state observable.
#[tokio::test]
async fn uninstall_records_stopped_state_before_package_removal() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "example", "ora.example", "Example");
    let package_root = temp_dir.path().join("plugins").join("example");
    let pool = DatabaseBootstrapper::system()
        .bootstrap_repository_pool(
            &DatabaseLocation::path(temp_dir.path().join("ora.sqlite3")),
            &default_migration_catalog().expect("build migration catalog"),
        )
        .expect("bootstrap plugin lifecycle database");
    let repository = SqlitePluginStateRepository::new(pool);
    let repository_probe = repository.clone();
    let (runtime, mut stop_started, release_stop) = ControllableStopRuntime::new();
    let (publisher, mut events) = RecordingStatusPublisher::new();
    let lifecycle = PluginLifecycle::open(
        PluginLifecycleConfig {
            data_directory: temp_dir.path().to_path_buf(),
            deno_path: PathBuf::from("deno"),
        },
        repository,
        FixedClock,
        ImmediateRuntimeLauncher { runtime },
        publisher,
    )
    .expect("open plugin lifecycle");
    lifecycle
        .enable_plugin(EnablePluginRequest {
            plugin_id: "ora.example".to_string(),
        })
        .await
        .expect("enable plugin");
    assert_eq!(events.recv().await, Some(PluginId::new("ora.example")));
    lifecycle
        .activate_plugin(ActivatePluginRequest {
            plugin_id: "ora.example".to_string(),
        })
        .await
        .expect("activate plugin");
    assert_eq!(events.recv().await, Some(PluginId::new("ora.example")));
    assert_eq!(events.recv().await, Some(PluginId::new("ora.example")));
    fs::remove_dir_all(&package_root).expect("remove package fixture directory");
    fs::write(&package_root, "not a directory").expect("replace package directory with a file");

    let uninstall_lifecycle = lifecycle.clone();
    let uninstall_task = tokio::spawn(async move {
        uninstall_lifecycle
            .uninstall_plugin(UninstallPluginRequest {
                plugin_id: "ora.example".to_string(),
            })
            .await
    });
    assert_eq!(stop_started.recv().await, Some(()));
    release_stop.send(()).expect("release runtime stop");
    let error = uninstall_task
        .await
        .expect("join uninstall task")
        .expect_err("package removal should fail");
    assert_eq!(events.recv().await, Some(PluginId::new("ora.example")));
    assert!(matches!(
        error,
        PluginLifecycleError::PackageRemoval { path, .. } if path == package_root
    ));

    assert_eq!(
        (
            lifecycle.list_installed_plugins(),
            repository_probe
                .find_plugin_state(&PluginId::new("ora.example"))
                .expect("read deleted durable state"),
            package_root.is_file(),
        ),
        (
            ListInstalledPluginsResponse {
                plugins: vec![expected_plugin(/*enabled*/ true)],
            },
            None,
            true,
        ),
    );
}

/// Verifies an unexpected process exit records failure without clearing durable eligibility.
#[tokio::test]
async fn records_runtime_failure_without_disabling_plugin() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "example", "ora.example", "Example");
    let pool = DatabaseBootstrapper::system()
        .bootstrap_repository_pool(
            &DatabaseLocation::path(temp_dir.path().join("ora.sqlite3")),
            &default_migration_catalog().expect("build migration catalog"),
        )
        .expect("bootstrap plugin lifecycle database");
    let (runtime, fail_runtime) = ControllableFailureRuntime::new();
    let (publisher, mut events) = RecordingStatusPublisher::new();
    let lifecycle = PluginLifecycle::open(
        PluginLifecycleConfig {
            data_directory: temp_dir.path().to_path_buf(),
            deno_path: PathBuf::from("deno"),
        },
        SqlitePluginStateRepository::new(pool),
        FixedClock,
        FailureRuntimeLauncher { runtime },
        publisher,
    )
    .expect("open plugin lifecycle");
    lifecycle
        .enable_plugin(EnablePluginRequest {
            plugin_id: "ora.example".to_string(),
        })
        .await
        .expect("enable plugin");
    assert_eq!(events.recv().await, Some(PluginId::new("ora.example")));
    lifecycle
        .activate_plugin(ActivatePluginRequest {
            plugin_id: "ora.example".to_string(),
        })
        .await
        .expect("activate plugin");
    assert_eq!(events.recv().await, Some(PluginId::new("ora.example")));
    assert_eq!(events.recv().await, Some(PluginId::new("ora.example")));

    fail_runtime
        .send(PluginRuntimeExit::Failed(PluginRuntimeFailure::new(
            "process crashed",
        )))
        .expect("fail runtime");
    assert_eq!(events.recv().await, Some(PluginId::new("ora.example")));

    assert_eq!(
        lifecycle.list_installed_plugins(),
        ListInstalledPluginsResponse {
            plugins: vec![expected_plugin_with_runtime(
                PluginEnabledState::Enabled,
                PluginRuntimeStatus::Failed {
                    failure_reason: "process crashed".to_string(),
                },
            )],
        },
    );
}

/// Builds the complete expected wire contract for the shared package fixture.
fn expected_plugin(enabled: bool) -> InstalledPlugin {
    expected_plugin_with_runtime(
        if enabled {
            PluginEnabledState::Enabled
        } else {
            PluginEnabledState::Disabled
        },
        PluginRuntimeStatus::Stopped,
    )
}

/// Builds the expected package contract with an explicit lifecycle state.
fn expected_plugin_with_runtime(
    enabled: PluginEnabledState,
    runtime: PluginRuntimeStatus,
) -> InstalledPlugin {
    InstalledPlugin {
        id: "ora.example".to_string(),
        package_name: "@ora/example".to_string(),
        display_name: "Example".to_string(),
        version: "1.0.0".to_string(),
        kind: "agent".to_string(),
        main: "dist/index.js".to_string(),
        agents: vec![InstalledPluginAgent {
            id: "example-agent".to_string(),
            display_name: "Example Agent".to_string(),
            contract_version: 1,
        }],
        enabled: enabled.is_enabled(),
        runtime,
    }
}

/// Pauses one launch until the test permits the transition to running.
#[derive(Clone)]
struct ControllableRuntimeLauncher {
    launched: mpsc::UnboundedSender<PluginLaunchRequest>,
    release: Arc<Mutex<Option<oneshot::Receiver<()>>>>,
}

impl ControllableRuntimeLauncher {
    /// Creates the launcher plus the observation and release controls used by a test.
    fn new() -> (
        Self,
        mpsc::UnboundedReceiver<PluginLaunchRequest>,
        oneshot::Sender<()>,
    ) {
        let (launched_tx, launched_rx) = mpsc::unbounded_channel();
        let (release_tx, release_rx) = oneshot::channel();
        (
            Self {
                launched: launched_tx,
                release: Arc::new(Mutex::new(Some(release_rx))),
            },
            launched_rx,
            release_tx,
        )
    }
}

impl PluginRuntimeLauncher for ControllableRuntimeLauncher {
    type Runtime = FakeRuntime;

    /// Records the launch request and waits for the test-controlled ready signal.
    fn launch(
        &self,
        request: PluginLaunchRequest,
    ) -> impl Future<Output = Result<Self::Runtime, PluginRuntimeFailure>> + Send {
        let launched = self.launched.clone();
        let release = self
            .release
            .lock()
            .expect("lock runtime launch release")
            .take()
            .expect("runtime launch is requested once");
        async move {
            launched
                .send(request)
                .map_err(|_| PluginRuntimeFailure::new("launch observer closed"))?;
            release
                .await
                .map_err(|_| PluginRuntimeFailure::new("launch release dropped"))?;
            Ok(FakeRuntime)
        }
    }
}

/// Represents a running fake whose failure future remains pending for this test.
#[derive(Clone)]
struct FakeRuntime;

impl PluginRuntime for FakeRuntime {
    /// Stops immediately because this test exercises activation rather than shutdown timing.
    fn stop(&self) -> impl Future<Output = Result<(), PluginRuntimeFailure>> + Send {
        async { Ok(()) }
    }

    /// Never fails unless a later test supplies a controllable runtime implementation.
    fn wait_for_exit(&self) -> impl Future<Output = PluginRuntimeExit> + Send + 'static {
        pending()
    }
}

/// Returns one ready runtime whose eventual exit is controlled by the test.
#[derive(Clone)]
struct FailureRuntimeLauncher {
    runtime: ControllableFailureRuntime,
}

impl PluginRuntimeLauncher for FailureRuntimeLauncher {
    type Runtime = ControllableFailureRuntime;

    /// Completes launch immediately so the test can drive the later process exit.
    fn launch(
        &self,
        _request: PluginLaunchRequest,
    ) -> impl Future<Output = Result<Self::Runtime, PluginRuntimeFailure>> + Send {
        let runtime = self.runtime.clone();
        async move { Ok(runtime) }
    }
}

/// Delivers one test-controlled intentional or failed process exit.
#[derive(Clone)]
struct ControllableFailureRuntime {
    exit: Arc<Mutex<Option<oneshot::Receiver<PluginRuntimeExit>>>>,
}

impl ControllableFailureRuntime {
    /// Creates the runtime and the sender used to complete its exit observer.
    fn new() -> (Self, oneshot::Sender<PluginRuntimeExit>) {
        let (exit_tx, exit_rx) = oneshot::channel();
        (
            Self {
                exit: Arc::new(Mutex::new(Some(exit_rx))),
            },
            exit_tx,
        )
    }
}

impl PluginRuntime for ControllableFailureRuntime {
    /// Stops immediately because the failure test never requests explicit shutdown.
    fn stop(&self) -> impl Future<Output = Result<(), PluginRuntimeFailure>> + Send {
        async { Ok(()) }
    }

    /// Waits for the exact exit classification selected by the test.
    fn wait_for_exit(&self) -> impl Future<Output = PluginRuntimeExit> + Send + 'static {
        let exit = self
            .exit
            .lock()
            .expect("lock runtime exit")
            .take()
            .expect("runtime exit is observed once");
        async move { exit.await.unwrap_or(PluginRuntimeExit::Stopped) }
    }
}

/// Returns one already-ready controllable runtime for stop behavior tests.
#[derive(Clone)]
struct ImmediateRuntimeLauncher {
    runtime: ControllableStopRuntime,
}

impl PluginRuntimeLauncher for ImmediateRuntimeLauncher {
    type Runtime = ControllableStopRuntime;

    /// Completes launch immediately so the test can focus on explicit stop behavior.
    fn launch(
        &self,
        _request: PluginLaunchRequest,
    ) -> impl Future<Output = Result<Self::Runtime, PluginRuntimeFailure>> + Send {
        let runtime = self.runtime.clone();
        async move { Ok(runtime) }
    }
}

/// Blocks stop completion until the test releases the simulated external process.
#[derive(Clone)]
struct ControllableStopRuntime {
    stop_started: mpsc::UnboundedSender<()>,
    release_stop: Arc<Mutex<Option<oneshot::Receiver<()>>>>,
}

impl ControllableStopRuntime {
    /// Creates the runtime plus observation and release controls for explicit stop.
    fn new() -> (Self, mpsc::UnboundedReceiver<()>, oneshot::Sender<()>) {
        let (stop_started_tx, stop_started_rx) = mpsc::unbounded_channel();
        let (release_stop_tx, release_stop_rx) = oneshot::channel();
        (
            Self {
                stop_started: stop_started_tx,
                release_stop: Arc::new(Mutex::new(Some(release_stop_rx))),
            },
            stop_started_rx,
            release_stop_tx,
        )
    }
}

impl PluginRuntime for ControllableStopRuntime {
    /// Records the stop request and waits until the simulated process has exited.
    fn stop(&self) -> impl Future<Output = Result<(), PluginRuntimeFailure>> + Send {
        let stop_started = self.stop_started.clone();
        let release_stop = self
            .release_stop
            .lock()
            .expect("lock runtime stop release")
            .take()
            .expect("runtime stop is requested once");
        async move {
            stop_started
                .send(())
                .map_err(|_| PluginRuntimeFailure::new("stop observer closed"))?;
            release_stop
                .await
                .map_err(|_| PluginRuntimeFailure::new("stop release dropped"))
        }
    }

    /// Never fails independently while the stop-focused test owns the runtime.
    fn wait_for_exit(&self) -> impl Future<Output = PluginRuntimeExit> + Send + 'static {
        pending()
    }
}

/// Holds one launch open while exposing a runtime whose stop is separately controllable.
#[derive(Clone)]
struct QueuedRuntimeLauncher {
    runtime: ControllableStopRuntime,
    release_launch: Arc<Mutex<Option<oneshot::Receiver<()>>>>,
}

impl QueuedRuntimeLauncher {
    /// Creates the launcher and the control that marks its runtime ready.
    fn new(runtime: ControllableStopRuntime) -> (Self, oneshot::Sender<()>) {
        let (release_launch_tx, release_launch_rx) = oneshot::channel();
        (
            Self {
                runtime,
                release_launch: Arc::new(Mutex::new(Some(release_launch_rx))),
            },
            release_launch_tx,
        )
    }
}

impl PluginRuntimeLauncher for QueuedRuntimeLauncher {
    type Runtime = ControllableStopRuntime;

    /// Waits for the test-controlled ready signal before returning the runtime handle.
    fn launch(
        &self,
        _request: PluginLaunchRequest,
    ) -> impl Future<Output = Result<Self::Runtime, PluginRuntimeFailure>> + Send {
        let runtime = self.runtime.clone();
        let release_launch = self
            .release_launch
            .lock()
            .expect("lock queued launch release")
            .take()
            .expect("queued runtime launches once");
        async move {
            release_launch
                .await
                .map_err(|_| PluginRuntimeFailure::new("launch release dropped"))?;
            Ok(runtime)
        }
    }
}

/// Records invalidation identifiers without coupling lifecycle tests to Backend's event hub.
#[derive(Clone)]
struct RecordingStatusPublisher {
    events: mpsc::UnboundedSender<PluginId>,
}

impl RecordingStatusPublisher {
    /// Creates the publisher and its receiving observation seam.
    fn new() -> (Self, mpsc::UnboundedReceiver<PluginId>) {
        let (events, receiver) = mpsc::unbounded_channel();
        (Self { events }, receiver)
    }
}

impl PluginStatusPublisher for RecordingStatusPublisher {
    /// Records one invalidation without interpreting lifecycle state.
    fn publish_status_changed(&self, plugin_id: &PluginId) {
        let _ = self.events.send(plugin_id.clone());
    }
}

/// Writes one complete package below the plugin-manager discovery root.
fn write_plugin_package(data_dir: &std::path::Path, directory: &str, id: &str, name: &str) {
    let package_root = data_dir.join("plugins").join(directory);
    fs::create_dir_all(package_root.join("dist")).expect("create plugin package");
    fs::write(package_root.join("dist").join("index.js"), "export {};\n")
        .expect("write plugin entrypoint");
    fs::write(
        package_root.join("package.json"),
        serde_json::to_vec_pretty(&json!({
            "name": "@ora/example",
            "version": "1.0.0",
            "type": "module",
            "ora": {
                "manifestVersion": 1,
                "id": id,
                "displayName": name,
                "kind": "agent",
                "main": "dist/index.js",
                "engines": {
                    "ora": ">=0.1.0 <0.2.0",
                    "pluginApi": 1,
                    "bun": ">=1.0.0 <2.0.0"
                },
                "contributes": {
                    "agents": [{
                        "id": "example-agent",
                        "displayName": "Example Agent",
                        "contractVersion": 1
                    }]
                }
            }
        }))
        .expect("serialize plugin manifest"),
    )
    .expect("write plugin manifest");
}
