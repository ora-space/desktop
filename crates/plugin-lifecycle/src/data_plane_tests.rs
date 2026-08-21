//! Tests for the plugin data plane: connections, notification pump, contract validation, and the
//! surface-closer ordering around stop, disable, and uninstall.

use crate::tests::{
    FixedClock, NoopNotificationSink, RecordingStatusPublisher, trace_logging_guard,
    write_plugin_package,
};
use crate::{
    ConnectionError, InboundNotification, LaunchedRuntime, PluginCallError, PluginGeneration,
    PluginLaunchRequest, PluginLifecycle, PluginLifecycleConfig, PluginNotification,
    PluginNotificationSink, PluginRegistration, PluginRuntime, PluginRuntimeExit,
    PluginRuntimeFailure, PluginRuntimeLauncher, SurfaceCloser,
};
use ora_contracts::{
    ActivatePluginRequest, DisablePluginRequest, EnablePluginRequest, PluginRuntimeStatus,
    StopPluginRequest, UninstallPluginRequest,
};
use ora_db::{
    DatabaseBootstrapper, DatabaseLocation, SqlitePluginStateRepository, default_migration_catalog,
};
use ora_domain::PluginId;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::fs;
use std::future::{Future, pending};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::{mpsc, oneshot};

/// Shared, ordered record of the observable side effects a test cares about.
type EffectLog = Arc<Mutex<Vec<String>>>;

/// Appends one effect to the shared log.
fn record(log: &EffectLog, effect: &str) {
    log.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(effect.to_string());
}

/// Launches scripted runtimes and hands each launch's notification sender to the test.
#[derive(Clone)]
struct ScriptedLauncher {
    registration: PluginRegistration,
    log: EffectLog,
    notification_senders: mpsc::UnboundedSender<mpsc::UnboundedSender<PluginNotification>>,
    launch_failure: Option<String>,
    release: Arc<Mutex<Option<oneshot::Receiver<()>>>>,
}

impl ScriptedLauncher {
    /// Builds a launcher that completes immediately with the given registration.
    fn new(
        registration: PluginRegistration,
    ) -> (
        Self,
        EffectLog,
        mpsc::UnboundedReceiver<mpsc::UnboundedSender<PluginNotification>>,
    ) {
        let (senders_tx, senders_rx) = mpsc::unbounded_channel();
        let log = EffectLog::default();
        (
            Self {
                registration,
                log: Arc::clone(&log),
                notification_senders: senders_tx,
                launch_failure: None,
                release: Arc::new(Mutex::new(None)),
            },
            log,
            senders_rx,
        )
    }

    /// Holds the next launch open until the returned sender fires.
    fn gated(mut self) -> (Self, oneshot::Sender<()>) {
        let (release_tx, release_rx) = oneshot::channel();
        self.release = Arc::new(Mutex::new(Some(release_rx)));
        (self, release_tx)
    }
}

impl PluginRuntimeLauncher for ScriptedLauncher {
    type Runtime = ScriptedRuntime;

    /// Optionally waits for the gate, then returns a runtime plus a live notification stream.
    fn launch(
        &self,
        _request: PluginLaunchRequest,
    ) -> impl Future<Output = Result<LaunchedRuntime<Self::Runtime>, PluginRuntimeFailure>> + Send
    {
        let this = self.clone();
        let release = this
            .release
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        async move {
            if let Some(release) = release {
                let _ = release.await;
            }
            if let Some(reason) = this.launch_failure {
                return Err(PluginRuntimeFailure::new(reason));
            }
            let (sender, notifications) = mpsc::unbounded_channel();
            let _ = this.notification_senders.send(sender);
            Ok(LaunchedRuntime {
                runtime: ScriptedRuntime {
                    registration: this.registration,
                    log: this.log,
                },
                notifications,
            })
        }
    }
}

/// Records stops, never exits on its own, and echoes invocations.
#[derive(Clone)]
struct ScriptedRuntime {
    registration: PluginRegistration,
    log: EffectLog,
}

impl PluginRuntime for ScriptedRuntime {
    /// Records the stop so tests can assert ordering against surface closing.
    fn stop(&self) -> impl Future<Output = Result<(), PluginRuntimeFailure>> + Send {
        record(&self.log, "stop");
        async { Ok(()) }
    }

    /// Never exits; the notification pump must therefore decide on its own.
    fn wait_for_exit(&self) -> impl Future<Output = PluginRuntimeExit> + Send + 'static {
        pending()
    }

    /// Returns the scripted registration.
    fn registration(&self) -> PluginRegistration {
        self.registration.clone()
    }

    /// Echoes the method and params so connection tests can see the call went through.
    fn invoke(
        &self,
        method: &str,
        params: Value,
    ) -> impl Future<Output = Result<Value, PluginCallError>> + Send {
        let result = json!({ "method": method, "params": params });
        async move { Ok(result) }
    }

    /// Accepts every notification.
    fn notify(
        &self,
        _method: &str,
        _params: Value,
    ) -> impl Future<Output = Result<(), PluginCallError>> + Send {
        async { Ok(()) }
    }
}

/// Records every inbound notification for deep-equality assertions.
#[derive(Clone)]
struct RecordingSink {
    notifications: mpsc::UnboundedSender<InboundNotification>,
}

impl PluginNotificationSink for RecordingSink {
    /// Forwards the notification to the test.
    fn on_notification(&self, notification: InboundNotification) {
        let _ = self.notifications.send(notification);
    }
}

/// Records surface closing in the shared effect log.
struct RecordingCloser {
    log: EffectLog,
}

impl SurfaceCloser for RecordingCloser {
    /// Records the close so its position relative to `stop` can be asserted.
    fn close_all(&self, plugin_id: &PluginId) -> impl Future<Output = ()> + Send {
        record(&self.log, &format!("close_all:{plugin_id}"));
        async {}
    }
}

type TestLifecycle<Sink> = PluginLifecycle<
    SqlitePluginStateRepository,
    FixedClock,
    ScriptedLauncher,
    RecordingStatusPublisher,
    Sink,
>;

/// Opens a lifecycle over a fresh database in `temp_dir` with the given launcher and sink.
fn open_lifecycle<Sink: PluginNotificationSink>(
    temp_dir: &Path,
    launcher: ScriptedLauncher,
    sink: Sink,
) -> (TestLifecycle<Sink>, mpsc::UnboundedReceiver<PluginId>) {
    let pool = DatabaseBootstrapper::system()
        .bootstrap_repository_pool(
            &DatabaseLocation::path(temp_dir.join("ora.sqlite3")),
            &default_migration_catalog().expect("build migration catalog"),
        )
        .expect("bootstrap plugin lifecycle database");
    let (publisher, events) = RecordingStatusPublisher::new();
    let lifecycle = PluginLifecycle::open(
        PluginLifecycleConfig {
            data_directory: temp_dir.to_path_buf(),
            deno_path: PathBuf::from("deno"),
        },
        SqlitePluginStateRepository::new(pool),
        FixedClock,
        launcher,
        publisher,
        sink,
    )
    .expect("open plugin lifecycle");
    (lifecycle, events)
}

/// Enables `ora.example` and drains the resulting status event.
async fn enable_example<Sink: PluginNotificationSink>(
    lifecycle: &TestLifecycle<Sink>,
    events: &mut mpsc::UnboundedReceiver<PluginId>,
) {
    lifecycle
        .enable_plugin(EnablePluginRequest {
            plugin_id: "official/ora.example".to_string(),
        })
        .await
        .expect("enable plugin");
    assert_eq!(
        events.recv().await,
        Some(PluginId::new("official", "ora.example").expect("plugin id"))
    );
}

/// Returns the runtime status the lifecycle currently reports for `ora.example`.
fn example_status<Sink: PluginNotificationSink>(
    lifecycle: &TestLifecycle<Sink>,
) -> PluginRuntimeStatus {
    lifecycle
        .list_installed_plugins()
        .plugins
        .into_iter()
        .find(|plugin| plugin.id == "official/ora.example")
        .map(|plugin| plugin.runtime)
        .expect("example plugin is installed")
}

/// Writes one remote-site ui package into the flat installed layout.
fn write_ui_plugin_package(data_dir: &Path, name: &str) {
    let package_root = data_dir.join("plugins").join("installed").join(name);
    fs::create_dir_all(&package_root).expect("create plugin package");
    fs::write(package_root.join("main.js"), "export {};\n").expect("write plugin entrypoint");
    fs::write(
        package_root.join("orax.toml"),
        format!(
            r#"resolver = 1
name = "{name}"
namespace = "official"
kind = "ui"
version = "1.0.0"
description = "Example ui plugin"

[[ui.surfaces]]
id = "market"
title = "Market"

[ui.surfaces.source]
kind = "remote_site"
entry = "https://example.com/"
hosts = ["example.com"]
"#
        ),
    )
    .expect("write plugin manifest");
}

/// `ensure_running` activates a stopped plugin and returns a connection to generation 1.
#[tokio::test]
async fn ensure_running_starts_a_stopped_plugin() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "ora.example");
    let (launcher, _log, _senders) = ScriptedLauncher::new(PluginRegistration::default());
    let (lifecycle, mut events) = open_lifecycle(temp_dir.path(), launcher, NoopNotificationSink);
    enable_example(&lifecycle, &mut events).await;
    let plugin_id = PluginId::new("official", "ora.example").expect("plugin id");

    let connection = lifecycle
        .ensure_running(&plugin_id, Duration::from_secs(5))
        .await
        .expect("ensure running");
    let echoed = connection
        .invoke("ui/ping", json!({ "n": 1 }))
        .await
        .expect("invoke");

    assert_eq!(
        (
            connection.generation(),
            connection.plugin_id().clone(),
            echoed,
            example_status(&lifecycle),
            lifecycle.connection(&plugin_id).map(|c| c.generation()),
            temp_dir
                .path()
                .join("plugins")
                .join("data")
                .join("official")
                .join("ora.example")
                .join("downloads")
                .is_dir(),
        ),
        (
            PluginGeneration(1),
            plugin_id.clone(),
            json!({ "method": "ui/ping", "params": { "n": 1 } }),
            PluginRuntimeStatus::Running,
            Ok(PluginGeneration(1)),
            true,
        ),
    );
}

/// Disabled and unknown plugins are reported without any launch attempt.
#[tokio::test]
async fn ensure_running_reports_disabled_and_not_found() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "ora.example");
    let (launcher, log, _senders) = ScriptedLauncher::new(PluginRegistration::default());
    let (lifecycle, _events) = open_lifecycle(temp_dir.path(), launcher, NoopNotificationSink);

    let disabled = lifecycle
        .ensure_running(
            &PluginId::new("official", "ora.example").expect("plugin id"),
            Duration::from_secs(1),
        )
        .await
        .map(|c| c.generation());
    let missing = lifecycle
        .ensure_running(
            &PluginId::new("official", "ora.missing").expect("plugin id"),
            Duration::from_secs(1),
        )
        .await
        .map(|c| c.generation());

    assert_eq!(
        (
            disabled,
            missing,
            lifecycle
                .connection(&PluginId::new("official", "ora.example").expect("plugin id"))
                .map(|c| c.generation()),
            log.lock().unwrap_or_else(PoisonError::into_inner).clone(),
        ),
        (
            Err(ConnectionError::Disabled),
            Err(ConnectionError::NotFound),
            Err(ConnectionError::Disabled),
            Vec::new(),
        ),
    );
}

/// A launch failure surfaces as `Failed` with the launcher's reason.
#[tokio::test]
async fn ensure_running_reports_launch_failure() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "ora.example");
    let (mut launcher, _log, _senders) = ScriptedLauncher::new(PluginRegistration::default());
    launcher.launch_failure = Some("deno exploded".to_string());
    let (lifecycle, mut events) = open_lifecycle(temp_dir.path(), launcher, NoopNotificationSink);
    enable_example(&lifecycle, &mut events).await;
    let plugin_id = PluginId::new("official", "ora.example").expect("plugin id");

    let result = lifecycle
        .ensure_running(&plugin_id, Duration::from_secs(5))
        .await
        .map(|c| c.generation());

    assert_eq!(
        (
            result,
            lifecycle.connection(&plugin_id).map(|c| c.generation()),
        ),
        (
            Err(ConnectionError::Failed("deno exploded".to_string())),
            Err(ConnectionError::Failed("deno exploded".to_string())),
        ),
    );
}

/// A launch that is still in flight yields `Timeout` from `ensure_running` and `NotReady` from
/// `connection`; a stopped plugin yields `NotRunning`.
#[tokio::test]
async fn ensure_running_times_out_while_starting() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "ora.example");
    let (launcher, _log, _senders) = ScriptedLauncher::new(PluginRegistration::default());
    let (launcher, release) = launcher.gated();
    let (lifecycle, mut events) = open_lifecycle(temp_dir.path(), launcher, NoopNotificationSink);
    enable_example(&lifecycle, &mut events).await;
    let plugin_id = PluginId::new("official", "ora.example").expect("plugin id");
    let stopped = lifecycle.connection(&plugin_id).map(|c| c.generation());

    let timed_out = lifecycle
        .ensure_running(&plugin_id, Duration::from_millis(50))
        .await
        .map(|c| c.generation());
    let starting = lifecycle.connection(&plugin_id).map(|c| c.generation());

    // Let the gated launch finish so ensure_running can now resolve to Running.
    release.send(()).expect("release launch");
    let running = lifecycle
        .ensure_running(&plugin_id, Duration::from_secs(5))
        .await
        .map(|c| c.generation());

    assert_eq!(
        (stopped, timed_out, starting, running),
        (
            Err(ConnectionError::NotRunning),
            Err(ConnectionError::Timeout),
            Err(ConnectionError::NotReady),
            Ok(PluginGeneration(1)),
        ),
    );
}

/// Notifications reach the sink tagged with the plugin and the generation that emitted them.
#[tokio::test]
async fn pump_forwards_notifications_with_generation() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "ora.example");
    let (launcher, _log, mut senders) = ScriptedLauncher::new(PluginRegistration::default());
    let (sink_tx, mut received) = mpsc::unbounded_channel();
    let sink = RecordingSink {
        notifications: sink_tx,
    };
    let (lifecycle, mut events) = open_lifecycle(temp_dir.path(), launcher, sink);
    enable_example(&lifecycle, &mut events).await;
    let plugin_id = PluginId::new("official", "ora.example").expect("plugin id");
    lifecycle
        .ensure_running(&plugin_id, Duration::from_secs(5))
        .await
        .expect("ensure running");
    let sender = senders.recv().await.expect("launch handed over a sender");

    sender
        .send(PluginNotification {
            method: "ui/progress".to_string(),
            params: json!({ "pct": 50 }),
        })
        .expect("send notification");

    assert_eq!(
        received.recv().await,
        Some(InboundNotification {
            plugin_id,
            generation: PluginGeneration(1),
            method: "ui/progress".to_string(),
            params: json!({ "pct": 50 }),
        }),
    );
}

/// A notification stream that closes while the process lives marks the attempt failed.
#[tokio::test(start_paused = true)]
async fn pump_close_under_live_process_fails_the_plugin() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "ora.example");
    let (launcher, _log, mut senders) = ScriptedLauncher::new(PluginRegistration::default());
    let (lifecycle, mut events) = open_lifecycle(temp_dir.path(), launcher, NoopNotificationSink);
    enable_example(&lifecycle, &mut events).await;
    let plugin_id = PluginId::new("official", "ora.example").expect("plugin id");
    lifecycle
        .ensure_running(&plugin_id, Duration::from_secs(5))
        .await
        .expect("ensure running");
    // Starting and Running events.
    assert_eq!(events.recv().await, Some(plugin_id.clone()));
    assert_eq!(events.recv().await, Some(plugin_id.clone()));

    drop(senders.recv().await.expect("launch handed over a sender"));

    assert_eq!(events.recv().await, Some(plugin_id.clone()));
    assert_eq!(
        example_status(&lifecycle),
        PluginRuntimeStatus::Failed {
            failure_reason: "plugin notification channel closed".to_string(),
        },
    );
}

/// A stream closing after its attempt was already stopped must not resurrect it as failed.
#[tokio::test(start_paused = true)]
async fn pump_close_after_stop_leaves_the_plugin_stopped() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "ora.example");
    let (launcher, _log, mut senders) = ScriptedLauncher::new(PluginRegistration::default());
    let (lifecycle, mut events) = open_lifecycle(temp_dir.path(), launcher, NoopNotificationSink);
    enable_example(&lifecycle, &mut events).await;
    let plugin_id = PluginId::new("official", "ora.example").expect("plugin id");
    lifecycle
        .ensure_running(&plugin_id, Duration::from_secs(5))
        .await
        .expect("ensure running");
    let sender = senders.recv().await.expect("launch handed over a sender");
    lifecycle
        .stop_plugin(StopPluginRequest {
            plugin_id: "official/ora.example".to_string(),
        })
        .await
        .expect("stop plugin");

    drop(sender);
    // Give the pump's grace period a chance to elapse under paused time.
    tokio::time::sleep(Duration::from_secs(5)).await;

    assert_eq!(example_status(&lifecycle), PluginRuntimeStatus::Stopped);
}

/// A remote-site ui plugin whose registration lacks the download handler is stopped and failed.
#[tokio::test]
async fn ui_plugin_missing_download_completed_fails_after_launch() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_ui_plugin_package(temp_dir.path(), "ora.example");
    let (launcher, log, _senders) = ScriptedLauncher::new(PluginRegistration::default());
    let (lifecycle, mut events) = open_lifecycle(temp_dir.path(), launcher, NoopNotificationSink);
    enable_example(&lifecycle, &mut events).await;
    let plugin_id = PluginId::new("official", "ora.example").expect("plugin id");

    let result = lifecycle
        .ensure_running(&plugin_id, Duration::from_secs(5))
        .await
        .map(|c| c.generation());

    assert_eq!(
        (
            result,
            log.lock().unwrap_or_else(PoisonError::into_inner).clone(),
        ),
        (
            Err(ConnectionError::Failed(
                "ui contract v1 requires method ora/ui/download_completed".to_string()
            )),
            vec!["stop".to_string()],
        ),
    );
}

/// A remote-site ui plugin that registers the download handler runs.
#[tokio::test]
async fn ui_plugin_with_download_completed_runs() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_ui_plugin_package(temp_dir.path(), "ora.example");
    let (launcher, _log, _senders) = ScriptedLauncher::new(PluginRegistration {
        methods: HashSet::from(["ora/ui/download_completed".to_string()]),
        emits: HashSet::new(),
    });
    let (lifecycle, mut events) = open_lifecycle(temp_dir.path(), launcher, NoopNotificationSink);
    enable_example(&lifecycle, &mut events).await;

    let result = lifecycle
        .ensure_running(
            &PluginId::new("official", "ora.example").expect("plugin id"),
            Duration::from_secs(5),
        )
        .await
        .map(|c| c.generation());

    assert_eq!(result, Ok(PluginGeneration(1)));
}

/// Stop, disable, and uninstall each close surfaces before stopping the runtime.
#[tokio::test]
async fn surfaces_close_before_the_runtime_stops() {
    let _logging = trace_logging_guard();
    let temp_dir = TempDir::new().expect("create plugin lifecycle directory");
    write_plugin_package(temp_dir.path(), "ora.example");
    let (launcher, log, _senders) = ScriptedLauncher::new(PluginRegistration::default());
    let (lifecycle, mut events) = open_lifecycle(temp_dir.path(), launcher, NoopNotificationSink);
    lifecycle.set_surface_closer(RecordingCloser {
        log: Arc::clone(&log),
    });
    let plugin_id = PluginId::new("official", "ora.example").expect("plugin id");
    let mut observed = Vec::new();

    enable_example(&lifecycle, &mut events).await;
    lifecycle
        .ensure_running(&plugin_id, Duration::from_secs(5))
        .await
        .expect("ensure running");
    lifecycle
        .stop_plugin(StopPluginRequest {
            plugin_id: "official/ora.example".to_string(),
        })
        .await
        .expect("stop plugin");
    observed.push(std::mem::take(
        &mut *log.lock().unwrap_or_else(PoisonError::into_inner),
    ));

    lifecycle
        .activate_plugin(ActivatePluginRequest {
            plugin_id: "official/ora.example".to_string(),
        })
        .await
        .expect("activate plugin");
    lifecycle
        .ensure_running(&plugin_id, Duration::from_secs(5))
        .await
        .expect("ensure running again");
    lifecycle
        .disable_plugin(DisablePluginRequest {
            plugin_id: "official/ora.example".to_string(),
        })
        .await
        .expect("disable plugin");
    observed.push(std::mem::take(
        &mut *log.lock().unwrap_or_else(PoisonError::into_inner),
    ));

    enable_example(&lifecycle, &mut events).await;
    lifecycle
        .ensure_running(&plugin_id, Duration::from_secs(5))
        .await
        .expect("ensure running a third time");
    lifecycle
        .uninstall_plugin(UninstallPluginRequest {
            plugin_id: "official/ora.example".to_string(),
        })
        .await
        .expect("uninstall plugin");
    observed.push(std::mem::take(
        &mut *log.lock().unwrap_or_else(PoisonError::into_inner),
    ));

    let ordered = vec![
        "close_all:official/ora.example".to_string(),
        "stop".to_string(),
    ];
    assert_eq!(
        (
            observed,
            temp_dir
                .path()
                .join("plugins")
                .join("data")
                .join("official")
                .join("ora.example")
                .exists(),
            lifecycle
                .ensure_running(&plugin_id, Duration::from_secs(1))
                .await
                .map(|c| c.generation()),
        ),
        (
            vec![ordered.clone(), ordered.clone(), ordered],
            false,
            Err(ConnectionError::NotFound),
        ),
    );
}
