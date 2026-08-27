use super::{
    DenoPermission, InboundNotification, LaunchedRuntime, PluginCallError, PluginLaunchRequest,
    PluginLifecycle, PluginLifecycleConfig, PluginLifecycleError, PluginNotificationSink,
    PluginRegistration, PluginRuntime, PluginRuntimeExit, PluginRuntimeFailure,
    PluginRuntimeLauncher, PluginStatusPublisher, ReadScope,
};
use ora_contracts::{
    ActivatePluginRequest, ActivatePluginResponse, InstalledPlugin, InstalledPluginContribution,
    ListInstalledPluginsResponse, PluginConfigurationSummary, PluginDataDisposition,
    PluginInstallationValidity, PluginRuntimeStatus, ScanPluginsRequest, ScanPluginsResponse,
    StopPluginRequest, StopPluginResponse, UninstallPluginRequest, UninstallPluginResponse,
};
use ora_domain::PluginId;
use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;
use std::fs;
use std::future::{Future, pending};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use tokio::sync::{mpsc, oneshot};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::prelude::*;

/// Installs a test-thread TRACE subscriber for the full lifetime of an async test future.
pub(super) fn trace_logging_guard() -> tracing::subscriber::DefaultGuard {
    let subscriber = tracing_subscriber::registry().with(LevelFilter::TRACE);
    tracing::subscriber::set_default(subscriber)
}

/// Opens lifecycle state for tests that never cross the external runtime boundary.
fn open_without_runtime(
    data_directory: &Path,
) -> PluginLifecycle<UnusedRuntimeLauncher, NoopStatusPublisher, NoopNotificationSink> {
    PluginLifecycle::open(
        PluginLifecycleConfig {
            data_directory: data_directory.to_path_buf(),
            deno_path: PathBuf::from("deno"),
        },
        UnusedRuntimeLauncher,
        NoopStatusPublisher,
        NoopNotificationSink,
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
    ) -> impl Future<Output = Result<LaunchedRuntime<Self::Runtime>, PluginRuntimeFailure>> + Send
    {
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

/// Discards plugin notifications in tests that do not observe the data plane.
#[derive(Clone)]
pub(super) struct NoopNotificationSink;

impl PluginNotificationSink for NoopNotificationSink {
    /// Intentionally ignores notifications outside data-plane tests.
    fn on_notification(&self, _notification: InboundNotification) {}
}

/// Pairs a fake runtime with a notification channel whose sender stays open for the test.
///
/// The sender is leaked on purpose: dropping it would close the stream and make the lifecycle
/// treat every fake launch as a dead reader.
fn launched_runtime<Runtime>(runtime: Runtime) -> LaunchedRuntime<Runtime> {
    let (sender, notifications) = mpsc::unbounded_channel();
    std::mem::forget(sender);
    LaunchedRuntime {
        runtime,
        notifications,
    }
}

/// Verifies startup discovery exposes an installed plugin as available and stopped.
#[test]
fn opens_with_discovered_plugins_stopped() {
    with_trace_logging(|| {
        let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
        write_plugin_package(temp_dir.path(), "ora.example");
        let lifecycle = open_without_runtime(temp_dir.path());

        assert_eq!(
            lifecycle.list_installed_plugins(),
            ListInstalledPluginsResponse {
                plugins: vec![expected_plugin()],
            },
        );
    });
}

/// Verifies a static Skill plugin can be listed and managed but never activated.
#[tokio::test]
async fn manages_static_skill_plugin_without_a_runtime() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_skill_plugin_package(temp_dir.path(), "ora.skill-pack");
    let lifecycle = open_without_runtime(temp_dir.path());
    let plugin_id = "official/ora.skill-pack".to_string();

    assert_eq!(
        lifecycle.list_installed_plugins(),
        ListInstalledPluginsResponse {
            plugins: vec![expected_skill_plugin()],
        }
    );
    assert!(matches!(
        lifecycle
            .activate_plugin(ActivatePluginRequest {
                plugin_id: plugin_id.clone(),
            })
            .await,
        Err(PluginLifecycleError::NoProcess { plugin_id: found }) if found == plugin_id
    ));
    assert_eq!(
        lifecycle
            .uninstall_plugin(UninstallPluginRequest {
                plugin_id: plugin_id.clone(),
                data_disposition: PluginDataDisposition::Delete,
            })
            .await
            .expect("uninstall Skill plugin"),
        UninstallPluginResponse { plugin_id }
    );
    assert_eq!(
        lifecycle.list_installed_plugins(),
        ListInstalledPluginsResponse {
            plugins: Vec::new(),
        }
    );
    assert!(!package_version_root(temp_dir.path(), "ora.skill-pack").exists());
}

/// Verifies a processless Hook plugin can be listed and uninstalled but never activated.
#[tokio::test]
async fn manages_static_hook_plugin_without_a_runtime() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_hook_plugin_package(temp_dir.path(), "rtk-ai.rtk");
    let lifecycle = open_without_runtime(temp_dir.path());
    let plugin_id = "official/rtk-ai.rtk".to_string();

    assert_eq!(
        lifecycle.list_installed_plugins(),
        ListInstalledPluginsResponse {
            plugins: vec![expected_hook_plugin()],
        }
    );
    assert!(matches!(
        lifecycle
            .activate_plugin(ActivatePluginRequest {
                plugin_id: plugin_id.clone(),
            })
            .await,
        Err(PluginLifecycleError::NoProcess { plugin_id: found }) if found == plugin_id
    ));
    assert_eq!(
        lifecycle
            .uninstall_plugin(UninstallPluginRequest {
                plugin_id: plugin_id.clone(),
                data_disposition: PluginDataDisposition::Delete,
            })
            .await
            .expect("uninstall Hook plugin"),
        UninstallPluginResponse { plugin_id }
    );
    assert_eq!(
        lifecycle.list_installed_plugins(),
        ListInstalledPluginsResponse {
            plugins: Vec::new(),
        }
    );
    assert!(!package_version_root(temp_dir.path(), "rtk-ai.rtk").exists());
}

/// Verifies an installed process plugin can start directly.
#[tokio::test]
async fn activates_plugin_and_starts_its_runtime() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "ora.example");
    let (runtime, _stop_started, _release_stop) = ControllableStopRuntime::new();
    let (publisher, mut events) = RecordingStatusPublisher::new();
    let lifecycle = PluginLifecycle::open(
        PluginLifecycleConfig {
            data_directory: temp_dir.path().to_path_buf(),
            deno_path: PathBuf::from("deno"),
        },
        ImmediateRuntimeLauncher { runtime },
        publisher,
        NoopNotificationSink,
    )
    .expect("open plugin lifecycle");

    let response = lifecycle
        .activate_plugin(ActivatePluginRequest {
            plugin_id: "official/ora.example".to_string(),
        })
        .await
        .expect("activate plugin");
    assert_eq!(
        response,
        ActivatePluginResponse {
            plugin: expected_plugin_with_runtime(PluginRuntimeStatus::Starting),
        },
    );
    assert_eq!(
        events.recv().await,
        Some(PluginId::new("official", "ora.example").expect("plugin id"))
    );
    assert_eq!(
        events.recv().await,
        Some(PluginId::new("official", "ora.example").expect("plugin id"))
    );

    assert_eq!(
        lifecycle.list_installed_plugins(),
        ListInstalledPluginsResponse {
            plugins: vec![expected_plugin_with_runtime(PluginRuntimeStatus::Running)],
        },
    );
}

/// Verifies only an explicit scan observes packages added after lifecycle startup.
#[tokio::test]
async fn scans_new_packages_without_rescanning_cached_queries() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    let lifecycle = open_without_runtime(temp_dir.path());
    write_plugin_package(temp_dir.path(), "ora.example");

    let before_scan = lifecycle.list_installed_plugins();
    let scan_response = lifecycle
        .scan_plugins(ScanPluginsRequest {})
        .await
        .expect("scan plugins");
    let expected_plugin = expected_plugin();

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

/// Verifies a package discovered after startup is immediately available and stopped.
#[tokio::test]
async fn package_discovered_after_startup_is_available() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    let lifecycle = open_without_runtime(temp_dir.path());

    write_plugin_package(temp_dir.path(), "ora.example");
    let response = lifecycle
        .scan_plugins(ScanPluginsRequest {})
        .await
        .expect("scan restored package");

    assert_eq!(
        response,
        ScanPluginsResponse {
            plugins: vec![expected_plugin()],
        },
    );
}

/// Verifies removing and restoring a package makes it available again.
#[tokio::test]
async fn scan_restores_removed_packages_as_available() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "ora.example");
    let lifecycle = open_without_runtime(temp_dir.path());
    fs::remove_dir_all(example_package_root(temp_dir.path())).expect("remove plugin outside Ora");
    lifecycle
        .scan_plugins(ScanPluginsRequest {})
        .await
        .expect("reconcile deleted package");
    write_plugin_package(temp_dir.path(), "ora.example");
    let restored = lifecycle
        .scan_plugins(ScanPluginsRequest {})
        .await
        .expect("scan restored package");

    assert_eq!(
        restored,
        ScanPluginsResponse {
            plugins: vec![expected_plugin()],
        },
    );
}

/// Verifies reconciliation stops an externally orphaned runtime before clearing its state.
#[tokio::test]
async fn scan_stops_runtime_for_package_deleted_outside_ora() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "ora.example");
    let (runtime, mut stop_started, release_stop) = ControllableStopRuntime::new();
    let lifecycle = PluginLifecycle::open(
        PluginLifecycleConfig {
            data_directory: temp_dir.path().to_path_buf(),
            deno_path: PathBuf::from("deno"),
        },
        ImmediateRuntimeLauncher { runtime },
        NoopStatusPublisher,
        NoopNotificationSink,
    )
    .expect("open plugin lifecycle");
    let activation = lifecycle
        .activate_plugin(ActivatePluginRequest {
            plugin_id: "official/ora.example".to_string(),
        })
        .await
        .expect("activate plugin");
    assert_eq!(
        activation,
        ActivatePluginResponse {
            plugin: expected_plugin_with_runtime(PluginRuntimeStatus::Starting),
        },
    );
    tokio::task::yield_now().await;
    fs::remove_dir_all(example_package_root(temp_dir.path())).expect("remove plugin outside Ora");

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
        response,
        ScanPluginsResponse {
            plugins: Vec::new()
        }
    );
}

/// Verifies an explicit scan retains an existing installed package.
#[tokio::test]
async fn scan_retains_existing_plugin() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_skill_plugin_package(temp_dir.path(), "ora.skill-pack");
    let lifecycle = open_without_runtime(temp_dir.path());
    let response = lifecycle
        .scan_plugins(ScanPluginsRequest {})
        .await
        .expect("scan plugins");

    assert_eq!(
        (response, lifecycle.list_installed_plugins()),
        (
            ScanPluginsResponse {
                plugins: vec![expected_skill_plugin()],
            },
            ListInstalledPluginsResponse {
                plugins: vec![expected_skill_plugin()],
            },
        ),
    );
}

/// Verifies activation returns starting, launches with agent permissions, then reports running.
#[tokio::test]
async fn activation_launches_the_plugin_and_publishes_each_transition() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "ora.example");
    let (launcher, mut launched, release_launch) = ControllableRuntimeLauncher::new();
    let (publisher, mut events) = RecordingStatusPublisher::new();
    let lifecycle = PluginLifecycle::open(
        PluginLifecycleConfig {
            data_directory: temp_dir.path().to_path_buf(),
            deno_path: PathBuf::from("deno"),
        },
        launcher,
        publisher,
        NoopNotificationSink,
    )
    .expect("open plugin lifecycle");
    let response = lifecycle
        .activate_plugin(ActivatePluginRequest {
            plugin_id: "official/ora.example".to_string(),
        })
        .await
        .expect("activate plugin");
    assert_eq!(
        response,
        ActivatePluginResponse {
            plugin: expected_plugin_with_runtime(PluginRuntimeStatus::Starting),
        },
    );
    assert_eq!(
        events.recv().await,
        Some(PluginId::new("official", "ora.example").expect("plugin id"))
    );
    assert_eq!(
        launched.recv().await,
        Some(PluginLaunchRequest {
            plugin_id: PluginId::new("official", "ora.example").expect("plugin id"),
            deno_path: PathBuf::from("deno"),
            entrypoint: example_package_root(temp_dir.path()).join("main.js"),
            package_root: example_package_root(temp_dir.path()),
            permissions: vec![
                DenoPermission::AllowRun,
                DenoPermission::AllowRead(ReadScope::Everything),
                DenoPermission::AllowEnv,
                DenoPermission::AllowNet,
            ],
            data_dir: temp_dir
                .path()
                .join("plugins")
                .join("data")
                .join("official")
                .join("ora.example"),
        }),
    );

    release_launch.send(()).expect("release runtime launch");
    assert_eq!(
        events.recv().await,
        Some(PluginId::new("official", "ora.example").expect("plugin id"))
    );
    // Activation of an already running plugin is a read of the live state, never a second launch:
    // the launcher would panic if it were asked to start the same plugin twice.
    let activation = lifecycle
        .activate_plugin(ActivatePluginRequest {
            plugin_id: "official/ora.example".to_string(),
        })
        .await
        .expect("activate plugin");
    assert_eq!(
        (activation, lifecycle.list_installed_plugins()),
        (
            ActivatePluginResponse {
                plugin: expected_plugin_with_runtime(PluginRuntimeStatus::Running),
            },
            ListInstalledPluginsResponse {
                plugins: vec![expected_plugin_with_runtime(PluginRuntimeStatus::Running)],
            },
        ),
    );
}

/// Verifies explicit stop waits for process exit and leaves the plugin available.
#[tokio::test]
async fn stops_running_plugin() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "ora.example");
    let (runtime, mut stop_started, release_stop) = ControllableStopRuntime::new();
    let launcher = ImmediateRuntimeLauncher { runtime };
    let (publisher, mut events) = RecordingStatusPublisher::new();
    let lifecycle = PluginLifecycle::open(
        PluginLifecycleConfig {
            data_directory: temp_dir.path().to_path_buf(),
            deno_path: PathBuf::from("deno"),
        },
        launcher,
        publisher,
        NoopNotificationSink,
    )
    .expect("open plugin lifecycle");
    lifecycle
        .activate_plugin(ActivatePluginRequest {
            plugin_id: "official/ora.example".to_string(),
        })
        .await
        .expect("activate plugin");
    assert_eq!(
        events.recv().await,
        Some(PluginId::new("official", "ora.example").expect("plugin id"))
    );
    assert_eq!(
        events.recv().await,
        Some(PluginId::new("official", "ora.example").expect("plugin id"))
    );

    let stop_lifecycle = lifecycle.clone();
    let stop_task = tokio::spawn(async move {
        stop_lifecycle
            .stop_plugin(StopPluginRequest {
                plugin_id: "official/ora.example".to_string(),
            })
            .await
    });
    assert_eq!(stop_started.recv().await, Some(()));
    release_stop.send(()).expect("release runtime stop");
    let response = stop_task
        .await
        .expect("join stop task")
        .expect("stop plugin");

    assert_eq!(
        events.recv().await,
        Some(PluginId::new("official", "ora.example").expect("plugin id"))
    );
    let expected_plugin = expected_plugin();
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

/// Verifies stop queues behind an in-flight launch and waits for the resulting runtime stop.
#[tokio::test]
async fn queues_stop_behind_an_in_flight_launch() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "ora.example");
    let (runtime, mut stop_started, release_stop) = ControllableStopRuntime::new();
    let (launcher, release_launch) = QueuedRuntimeLauncher::new(runtime);
    let (publisher, _events) = RecordingStatusPublisher::new();
    let lifecycle = PluginLifecycle::open(
        PluginLifecycleConfig {
            data_directory: temp_dir.path().to_path_buf(),
            deno_path: PathBuf::from("deno"),
        },
        launcher,
        publisher,
        NoopNotificationSink,
    )
    .expect("open plugin lifecycle");
    lifecycle
        .activate_plugin(ActivatePluginRequest {
            plugin_id: "official/ora.example".to_string(),
        })
        .await
        .expect("activate plugin");

    let stop_lifecycle = lifecycle.clone();
    let stop_task = tokio::spawn(async move {
        stop_lifecycle
            .stop_plugin(StopPluginRequest {
                plugin_id: "official/ora.example".to_string(),
            })
            .await
    });
    tokio::task::yield_now().await;
    assert_eq!(stop_task.is_finished(), false);

    release_launch.send(()).expect("release runtime launch");
    assert_eq!(stop_started.recv().await, Some(()));
    release_stop.send(()).expect("release runtime stop");
    let response = stop_task
        .await
        .expect("join stop task")
        .expect("stop plugin after launch");

    assert_eq!(
        response,
        StopPluginResponse {
            plugin: expected_plugin(),
        },
    );
}

/// Verifies uninstall waits for runtime exit before deleting the package and cached identity.
#[tokio::test]
async fn uninstalls_running_plugin_after_stopping_it() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "ora.example");
    let package_root = example_package_root(temp_dir.path());
    let namespace_root = temp_dir
        .path()
        .join("plugins")
        .join("installed")
        .join("official");
    let (runtime, mut stop_started, release_stop) = ControllableStopRuntime::new();
    let launcher = ImmediateRuntimeLauncher { runtime };
    let (publisher, mut events) = RecordingStatusPublisher::new();
    let lifecycle = PluginLifecycle::open(
        PluginLifecycleConfig {
            data_directory: temp_dir.path().to_path_buf(),
            deno_path: PathBuf::from("deno"),
        },
        launcher,
        publisher,
        NoopNotificationSink,
    )
    .expect("open plugin lifecycle");
    lifecycle
        .activate_plugin(ActivatePluginRequest {
            plugin_id: "official/ora.example".to_string(),
        })
        .await
        .expect("activate plugin");
    assert_eq!(
        events.recv().await,
        Some(PluginId::new("official", "ora.example").expect("plugin id"))
    );
    assert_eq!(
        events.recv().await,
        Some(PluginId::new("official", "ora.example").expect("plugin id"))
    );

    let uninstall_lifecycle = lifecycle.clone();
    let uninstall_task = tokio::spawn(async move {
        uninstall_lifecycle
            .uninstall_plugin(UninstallPluginRequest {
                plugin_id: "official/ora.example".to_string(),
                data_disposition: PluginDataDisposition::Delete,
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
            namespace_root.exists(),
            lifecycle.list_installed_plugins(),
        ),
        (
            UninstallPluginResponse {
                plugin_id: "official/ora.example".to_string(),
            },
            false,
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
    write_plugin_package(temp_dir.path(), "ora.example");
    // Uninstall removes the whole `<namespace>/<name>` tree, so that is the path made unremovable.
    let package_root = example_package_root(temp_dir.path())
        .parent()
        .expect("version directory has a package name parent")
        .to_path_buf();
    let (runtime, mut stop_started, release_stop) = ControllableStopRuntime::new();
    let (publisher, mut events) = RecordingStatusPublisher::new();
    let lifecycle = PluginLifecycle::open(
        PluginLifecycleConfig {
            data_directory: temp_dir.path().to_path_buf(),
            deno_path: PathBuf::from("deno"),
        },
        ImmediateRuntimeLauncher { runtime },
        publisher,
        NoopNotificationSink,
    )
    .expect("open plugin lifecycle");
    lifecycle
        .activate_plugin(ActivatePluginRequest {
            plugin_id: "official/ora.example".to_string(),
        })
        .await
        .expect("activate plugin");
    assert_eq!(
        events.recv().await,
        Some(PluginId::new("official", "ora.example").expect("plugin id"))
    );
    assert_eq!(
        events.recv().await,
        Some(PluginId::new("official", "ora.example").expect("plugin id"))
    );
    fs::remove_dir_all(&package_root).expect("remove package fixture directory");
    fs::write(&package_root, "not a directory").expect("replace package directory with a file");

    let uninstall_lifecycle = lifecycle.clone();
    let uninstall_task = tokio::spawn(async move {
        uninstall_lifecycle
            .uninstall_plugin(UninstallPluginRequest {
                plugin_id: "official/ora.example".to_string(),
                data_disposition: PluginDataDisposition::Delete,
            })
            .await
    });
    assert_eq!(stop_started.recv().await, Some(()));
    release_stop.send(()).expect("release runtime stop");
    let error = uninstall_task
        .await
        .expect("join uninstall task")
        .expect_err("package removal should fail");
    assert_eq!(
        events.recv().await,
        Some(PluginId::new("official", "ora.example").expect("plugin id"))
    );
    assert!(matches!(
        error,
        PluginLifecycleError::UninstallStaging { path, .. } if path == package_root
    ));

    assert_eq!(
        (lifecycle.list_installed_plugins(), package_root.is_file()),
        (
            ListInstalledPluginsResponse {
                plugins: vec![expected_plugin()],
            },
            true,
        ),
    );
}

/// Verifies an unexpected process exit records failure while leaving the plugin available.
#[tokio::test]
async fn records_runtime_failure() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "ora.example");
    let (runtime, fail_runtime) = ControllableFailureRuntime::new();
    let (publisher, mut events) = RecordingStatusPublisher::new();
    let lifecycle = PluginLifecycle::open(
        PluginLifecycleConfig {
            data_directory: temp_dir.path().to_path_buf(),
            deno_path: PathBuf::from("deno"),
        },
        FailureRuntimeLauncher { runtime },
        publisher,
        NoopNotificationSink,
    )
    .expect("open plugin lifecycle");
    lifecycle
        .activate_plugin(ActivatePluginRequest {
            plugin_id: "official/ora.example".to_string(),
        })
        .await
        .expect("activate plugin");
    assert_eq!(
        events.recv().await,
        Some(PluginId::new("official", "ora.example").expect("plugin id"))
    );
    assert_eq!(
        events.recv().await,
        Some(PluginId::new("official", "ora.example").expect("plugin id"))
    );

    fail_runtime
        .send(PluginRuntimeExit::Failed(PluginRuntimeFailure::new(
            "process crashed",
        )))
        .expect("fail runtime");
    assert_eq!(
        events.recv().await,
        Some(PluginId::new("official", "ora.example").expect("plugin id"))
    );

    assert_eq!(
        lifecycle.list_installed_plugins(),
        ListInstalledPluginsResponse {
            plugins: vec![expected_plugin_with_runtime(PluginRuntimeStatus::Failed {
                failure_reason: "process crashed".to_string(),
            })],
        },
    );
}

/// Builds the complete expected wire contract for the shared package fixture.
fn expected_plugin() -> InstalledPlugin {
    expected_plugin_with_runtime(PluginRuntimeStatus::Stopped)
}

/// Builds the expected wire contract for the static Skill package fixture.
fn expected_skill_plugin() -> InstalledPlugin {
    InstalledPlugin {
        id: "official/ora.skill-pack".to_string(),
        namespace: "official".to_string(),
        name: "ora.skill-pack".to_string(),
        display_name: "ora.skill-pack".to_string(),
        version: "1.0.0".to_string(),
        description: "Example Skill plugin".to_string(),
        homepage: None,
        license: None,
        contribution: InstalledPluginContribution::Skill,
        logo: Some(PACKAGE_LOGO.to_string()),
        installation_validity: PluginInstallationValidity::Valid,
        configuration: PluginConfigurationSummary::NotDeclared,
        runtime: PluginRuntimeStatus::Stopped,
    }
}

/// Builds the expected wire contract for the static Hook package fixture.
fn expected_hook_plugin() -> InstalledPlugin {
    InstalledPlugin {
        id: "official/rtk-ai.rtk".to_string(),
        namespace: "official".to_string(),
        name: "rtk-ai.rtk".to_string(),
        display_name: "rtk-ai.rtk".to_string(),
        version: "1.0.0".to_string(),
        description: "Example Hook plugin".to_string(),
        homepage: None,
        license: None,
        contribution: InstalledPluginContribution::Hook {
            protocol: "rtk-rewrite-v1".to_string(),
            command: "rtk".to_string(),
            target: Some("x86_64-pc-windows-msvc".to_string()),
            tool_version: "0.45.0".to_string(),
        },
        logo: Some(PACKAGE_LOGO.to_string()),
        installation_validity: PluginInstallationValidity::Valid,
        configuration: PluginConfigurationSummary::NotDeclared,
        runtime: PluginRuntimeStatus::Stopped,
    }
}

/// Builds the expected package contract with an explicit lifecycle state.
fn expected_plugin_with_runtime(runtime: PluginRuntimeStatus) -> InstalledPlugin {
    InstalledPlugin {
        id: "official/ora.example".to_string(),
        namespace: "official".to_string(),
        name: "ora.example".to_string(),
        display_name: "ora.example".to_string(),
        version: "1.0.0".to_string(),
        description: "Example plugin".to_string(),
        homepage: None,
        license: None,
        contribution: InstalledPluginContribution::Agent {
            agent_display_name: "ora.example".to_string(),
        },
        logo: Some(PACKAGE_LOGO.to_string()),
        installation_validity: PluginInstallationValidity::Valid,
        configuration: PluginConfigurationSummary::NotDeclared,
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
    ) -> impl Future<Output = Result<LaunchedRuntime<Self::Runtime>, PluginRuntimeFailure>> + Send
    {
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
            Ok(launched_runtime(FakeRuntime))
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

    /// Reports an empty registration; these tests exercise agent packages, which skip validation.
    fn registration(&self) -> PluginRegistration {
        PluginRegistration::default()
    }

    /// Calls are not exercised by this double.
    fn invoke(
        &self,
        _method: &str,
        _params: serde_json::Value,
    ) -> impl Future<Output = Result<serde_json::Value, PluginCallError>> + Send {
        async { Err(PluginCallError::Unavailable) }
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
    ) -> impl Future<Output = Result<LaunchedRuntime<Self::Runtime>, PluginRuntimeFailure>> + Send
    {
        let runtime = self.runtime.clone();
        async move { Ok(launched_runtime(runtime)) }
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

    /// Reports an empty registration; these tests exercise agent packages, which skip validation.
    fn registration(&self) -> PluginRegistration {
        PluginRegistration::default()
    }

    /// Calls are not exercised by this double.
    fn invoke(
        &self,
        _method: &str,
        _params: serde_json::Value,
    ) -> impl Future<Output = Result<serde_json::Value, PluginCallError>> + Send {
        async { Err(PluginCallError::Unavailable) }
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
    ) -> impl Future<Output = Result<LaunchedRuntime<Self::Runtime>, PluginRuntimeFailure>> + Send
    {
        let runtime = self.runtime.clone();
        async move { Ok(launched_runtime(runtime)) }
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

    /// Reports an empty registration; these tests exercise agent packages, which skip validation.
    fn registration(&self) -> PluginRegistration {
        PluginRegistration::default()
    }

    /// Calls are not exercised by this double.
    fn invoke(
        &self,
        _method: &str,
        _params: serde_json::Value,
    ) -> impl Future<Output = Result<serde_json::Value, PluginCallError>> + Send {
        async { Err(PluginCallError::Unavailable) }
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
    ) -> impl Future<Output = Result<LaunchedRuntime<Self::Runtime>, PluginRuntimeFailure>> + Send
    {
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
            Ok(launched_runtime(runtime))
        }
    }
}

/// Records invalidation identifiers without coupling lifecycle tests to Backend's event hub.
#[derive(Clone)]
pub(super) struct RecordingStatusPublisher {
    events: mpsc::UnboundedSender<PluginId>,
}

impl RecordingStatusPublisher {
    /// Creates the publisher and its receiving observation seam.
    pub(super) fn new() -> (Self, mpsc::UnboundedReceiver<PluginId>) {
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

/// The icon shipped by the shared package fixture, which proves a discovered logo reaches the
/// wire contract unchanged.
const PACKAGE_LOGO: &str = r#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="8"/></svg>"#;

/// Returns the installed package directory of the shared `official/ora.example` fixture.
pub(super) fn example_package_root(data_dir: &std::path::Path) -> std::path::PathBuf {
    package_version_root(data_dir, "ora.example")
}

/// Returns the versioned package directory the fixture writer uses for `name`.
pub(super) fn package_version_root(data_dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    data_dir
        .join("plugins")
        .join("installed")
        .join("official")
        .join(name)
        .join("1.0.0")
}

/// Writes one static Skill package without a process entrypoint.
fn write_skill_plugin_package(data_dir: &std::path::Path, name: &str) {
    let package_root = package_version_root(data_dir, name);
    fs::create_dir_all(package_root.join("assets/review")).expect("create Skill plugin assets");
    fs::write(
        package_root.join("assets/review/SKILL.md"),
        "---\nname: review\ndescription: Reviews code\n---\n",
    )
    .expect("write bundled Skill manifest");
    fs::write(package_root.join("logo.svg"), PACKAGE_LOGO).expect("write plugin logo");
    fs::write(
        package_root.join("orax.toml"),
        format!(
            r#"identifier = "{name}"
namespace = "official"
kind = "skill"
version = "1.0.0"
description = "Example Skill plugin"
"#
        ),
    )
    .expect("write Skill plugin manifest");
}

/// Writes one processless Hook package with a contained executable and no `main.js`.
fn write_hook_plugin_package(data_dir: &std::path::Path, name: &str) {
    let package_root = package_version_root(data_dir, name);
    fs::create_dir_all(package_root.join("assets")).expect("create Hook plugin assets");
    fs::write(package_root.join("logo.svg"), PACKAGE_LOGO).expect("write plugin logo");
    fs::write(
        package_root.join("assets").join("config.json"),
        r#"{
            "schemaVersion": 1,
            "hook": {
                "protocol": "rtk-rewrite-v1",
                "executable": "assets/rtk.exe",
                "command": "rtk",
                "toolVersion": "0.45.0"
            }
        }"#,
    )
    .expect("write Hook configuration");
    fs::write(package_root.join("assets").join("rtk.exe"), b"MZdummy")
        .expect("write Hook executable");
    fs::write(
        package_root.join("orax.toml"),
        format!(
            r#"identifier = "{name}"
namespace = "official"
kind = "hook"
version = "1.0.0"
description = "Example Hook plugin"

[artifact]
target = "x86_64-pc-windows-msvc"
"#
        ),
    )
    .expect("write Hook plugin manifest");
}

/// Writes one complete agent package into the versioned installed layout.
pub(super) fn write_plugin_package(data_dir: &std::path::Path, name: &str) {
    let package_root = package_version_root(data_dir, name);
    fs::create_dir_all(&package_root).expect("create plugin package");
    fs::write(package_root.join("logo.svg"), PACKAGE_LOGO).expect("write plugin logo");
    fs::write(package_root.join("main.js"), "export {};\n").expect("write plugin entrypoint");
    fs::write(
        package_root.join("orax.toml"),
        format!(
            r#"resolver = 1
identifier = "{name}"
namespace = "official"
kind = "agent"
version = "1.0.0"
description = "Example plugin"
"#
        ),
    )
    .expect("write plugin manifest");
}
