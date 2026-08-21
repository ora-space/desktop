//! Host-level tests driving `SurfaceService` against `tauri::test::mock_app` with a fake plugin
//! gateway, plus the pure builders for plugin call params.

use crate::surface::downloads::{bring_main_window_forward, download_completed_params};
use crate::surface::error::SurfaceError;
use crate::surface::gateway::{GatewayFailure, SurfaceConnection, SurfacePluginGateway};
use crate::surface::hooks::DownloadSink;
use crate::surface::plugin_link::{ProcessStart, Replay, session_params};
use crate::surface::{MAIN_WINDOW_LABEL, SURFACE_EVENT, SurfaceService};
use ora_domain::PluginId;
use ora_plugin_lifecycle::{
    ConnectionError, InboundNotification, PluginCallError, PluginGeneration,
};
use ora_plugin_manager::{
    HostName, InstalledSurface, InstalledSurfaceSource, InstancePolicy, PanelSource,
    RemoteSiteSource, SurfaceId, WebDataPolicy,
};
use ora_surface::{
    CompletedDownload, DownloadClock, DownloadId, MountTarget, SurfaceInstanceId, SurfaceRecord,
    SurfaceState, WebviewLabel,
};
use ora_utils::path::PortableRelativePath;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::test::{MockRuntime, mock_app};
use tauri::webview::DownloadEvent;
use tauri::{App, Listener, Manager, Url, WebviewUrl, WebviewWindowBuilder};
use time::macros::datetime;
use tokio::sync::broadcast;

pub(super) const PLUGIN: &str = "ora-space.skillhub";
/// Second fake plugin, contributing one panel surface rooted in the gateway's data root.
pub(super) const PANEL_PLUGIN: &str = "ora-space.hello-panel";

/// Records every JSON-RPC call the fake plugin process receives and answers `ui/request` with
/// the payload echoed back, so bridge tests can see the exact params the host built.
#[derive(Clone, Default)]
pub(super) struct FakeConnection {
    pub(super) calls: Arc<Mutex<Vec<(String, Value)>>>,
}

impl SurfaceConnection for FakeConnection {
    fn generation(&self) -> PluginGeneration {
        PluginGeneration(3)
    }

    async fn invoke(&self, method: &str, params: Value) -> Result<Value, PluginCallError> {
        self.calls
            .lock()
            .expect("calls")
            .push((method.to_owned(), params.clone()));
        if method == "ui/request" {
            return Ok(json!({ "payload": { "echo": params["payload"] } }));
        }
        Ok(json!({}))
    }

    async fn notify(&self, method: &str, params: Value) -> Result<(), PluginCallError> {
        self.calls
            .lock()
            .expect("calls")
            .push((method.to_owned(), params));
        Ok(())
    }
}

/// Whether the fake process is currently reachable through `connection`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FakeProcess {
    Running,
    Stopped,
}

/// Two installed ui plugins: `PLUGIN` with one remote-site surface and `PANEL_PLUGIN` with one
/// panel surface whose assets live in `<data_root>/hello-panel/ui`.
#[derive(Clone)]
pub(super) struct FakeGateway {
    pub(super) data_root: PathBuf,
    pub(super) enabled: bool,
    pub(super) process: FakeProcess,
    pub(super) connection: FakeConnection,
    pub(super) notifications: broadcast::Sender<InboundNotification>,
    /// While set, `data_directory` fails so webview creation fails before any window exists.
    pub(super) data_directory_unavailable: Arc<AtomicBool>,
}

impl FakeGateway {
    pub(super) fn new(data_root: PathBuf) -> Self {
        let (notifications, _) = broadcast::channel(16);
        Self {
            data_root,
            enabled: true,
            process: FakeProcess::Stopped,
            connection: FakeConnection::default(),
            notifications,
            data_directory_unavailable: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Directory the panel surface serves; tests populate it before opening the panel.
    pub(super) fn panel_root(&self) -> PathBuf {
        self.data_root.join("hello-panel").join("ui")
    }
}

impl SurfacePluginGateway for FakeGateway {
    type Connection = FakeConnection;

    fn installed_surfaces(&self, plugin_id: &PluginId) -> Option<Vec<InstalledSurface>> {
        match plugin_id.as_ref() {
            PLUGIN => Some(vec![InstalledSurface {
                id: SurfaceId::parse("market").expect("surface id"),
                title: "SkillHub".to_owned(),
                instance_policy: InstancePolicy::Singleton,
                source: InstalledSurfaceSource::RemoteSite(RemoteSiteSource {
                    entry_url: Url::parse("https://www.skillhub.cn").expect("url"),
                    allow_hosts: vec![HostName::parse("www.skillhub.cn").expect("host")],
                    allow_host_suffixes: vec![],
                    web_data: WebDataPolicy::EphemeralIsolated,
                }),
            }]),
            PANEL_PLUGIN => Some(vec![InstalledSurface {
                id: SurfaceId::parse("counter").expect("surface id"),
                title: "Hello Panel".to_owned(),
                instance_policy: InstancePolicy::Singleton,
                source: InstalledSurfaceSource::Panel(PanelSource {
                    asset_root: self.panel_root(),
                    entry: PortableRelativePath::parse("index.html").expect("entry"),
                }),
            }]),
            _ => None,
        }
    }

    fn plugin_enabled(&self, plugin_id: &PluginId) -> bool {
        matches!(plugin_id.as_ref(), PLUGIN | PANEL_PLUGIN) && self.enabled
    }

    fn data_directory(&self, plugin_id: &PluginId) -> Result<PathBuf, GatewayFailure> {
        if self.data_directory_unavailable.load(Ordering::SeqCst) {
            return Err(GatewayFailure::Other("plugin data unavailable".to_owned()));
        }
        let directory = self.data_root.join("plugin-data").join(plugin_id.as_ref());
        std::fs::create_dir_all(directory.join("downloads"))
            .map_err(|error| GatewayFailure::Other(error.to_string()))?;
        Ok(directory)
    }

    async fn ensure_running(
        &self,
        _plugin_id: &PluginId,
        _wait: Duration,
    ) -> Result<FakeConnection, GatewayFailure> {
        Ok(self.connection.clone())
    }

    fn connection(&self, _plugin_id: &PluginId) -> Result<FakeConnection, GatewayFailure> {
        match self.process {
            FakeProcess::Running => Ok(self.connection.clone()),
            FakeProcess::Stopped => Err(GatewayFailure::Connection(ConnectionError::NotRunning)),
        }
    }

    async fn stop_if_idle(&self, _plugin_id: &PluginId) -> Result<(), GatewayFailure> {
        Ok(())
    }

    fn subscribe_notifications(&self) -> broadcast::Receiver<InboundNotification> {
        self.notifications.subscribe()
    }
}

/// Fixed local instant so tests never touch the process-wide logging clock.
#[derive(Clone, Copy)]
pub(super) struct FixedClock;

impl DownloadClock for FixedClock {
    fn now_local(&self) -> time::OffsetDateTime {
        datetime!(2026-08-20 16:30:00 +08:00)
    }
}

pub(super) type TestService = SurfaceService<FakeGateway, MockRuntime, FixedClock>;

/// Creates a mock app with a hidden main window that records every surface event.
pub(super) fn harness(
    gateway: FakeGateway,
) -> (App<MockRuntime>, Arc<TestService>, Arc<Mutex<Vec<Value>>>) {
    let app = mock_app();
    let main = WebviewWindowBuilder::new(
        app.handle(),
        MAIN_WINDOW_LABEL,
        WebviewUrl::App("index.html".into()),
    )
    .visible(false)
    .build()
    .expect("create main window");
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    main.listen(SURFACE_EVENT, move |event| {
        let payload: Value = serde_json::from_str(event.payload()).expect("json payload");
        sink.lock().expect("events").push(payload);
    });
    let service = SurfaceService::with_clock(app.handle().clone(), gateway, FixedClock);
    (app, service, events)
}

/// Counts surface windows currently registered with the app.
fn surface_window_count(app: &App<MockRuntime>) -> usize {
    app.webview_windows()
        .keys()
        .filter(|label| label.starts_with(WebviewLabel::REMOTE_PREFIX))
        .count()
}

fn plugin() -> PluginId {
    PluginId::new(PLUGIN)
}

/// Verifies a singleton surface opened twice yields one window and the same record, with the
/// documented label shape and an `opened` event emitted once.
#[test]
fn opening_a_singleton_twice_reuses_the_instance() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (app, service, events) = harness(FakeGateway::new(temp.path().to_path_buf()));

    let first = service
        .open(&plugin(), "market", MountTarget::Windowed)
        .expect("first open");
    let second = service
        .open(&plugin(), "market", MountTarget::Windowed)
        .expect("second open");

    assert_eq!(
        (
            first.label.as_str(),
            first.state.clone(),
            second == first,
            surface_window_count(&app),
            events.lock().expect("events").clone(),
        ),
        (
            "remote-surface:ora-space_skillhub:market:0",
            SurfaceState::Windowed {
                view: ora_surface::ViewGeneration::INITIAL,
            },
            true,
            1,
            vec![json!({
                "type": "opened",
                "instance": 0,
                "pluginId": PLUGIN,
                "surfaceId": "market",
                "target": "windowed",
                "title": "SkillHub",
            })],
        )
    );
}

/// Verifies a retry on an instance whose webview creation failed rebuilds it through the
/// registry instead of reloading a webview that never existed: the instance id is kept, a
/// window now exists, and the frontend sees `failed` followed by `opened`.
#[test]
fn reload_rebuilds_a_failed_instance() {
    let temp = tempfile::tempdir().expect("tempdir");
    let gateway = FakeGateway::new(temp.path().to_path_buf());
    let unavailable = gateway.data_directory_unavailable.clone();
    let (app, service, events) = harness(gateway);

    unavailable.store(true, Ordering::SeqCst);
    let failed = service
        .open(&plugin(), "market", MountTarget::Windowed)
        .expect("open registers the instance even when creation fails");
    let windows_after_failure = surface_window_count(&app);
    unavailable.store(false, Ordering::SeqCst);
    service.reload(failed.instance).expect("retry rebuilds");
    let rebuilt = service.list();

    assert_eq!(
        (
            failed.state,
            windows_after_failure,
            rebuilt
                .iter()
                .map(|record| (record.instance, record.state.clone()))
                .collect::<Vec<_>>(),
            surface_window_count(&app),
            events
                .lock()
                .expect("events")
                .iter()
                .map(|event| event["type"].as_str().unwrap_or("").to_owned())
                .collect::<Vec<_>>(),
        ),
        (
            SurfaceState::Failed {
                target: MountTarget::Windowed,
                reason: "plugin data unavailable".to_owned(),
            },
            0,
            vec![(
                SurfaceInstanceId::new(0),
                SurfaceState::Windowed {
                    view: ora_surface::ViewGeneration::INITIAL,
                },
            )],
            1,
            vec!["failed".to_owned(), "opened".to_owned()],
        )
    );
}

/// Verifies closing removes the instance and a reopen creates a fresh one.
#[test]
fn close_then_reopen_creates_a_new_instance() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (_app, service, events) = harness(FakeGateway::new(temp.path().to_path_buf()));

    let first = service
        .open(&plugin(), "market", MountTarget::Windowed)
        .expect("open");
    service.close(first.instance).expect("close");
    let listed_after_close = service.list();
    let second = service
        .open(&plugin(), "market", MountTarget::Windowed)
        .expect("reopen");
    let stale_close = service.close(first.instance);

    assert_eq!(
        (
            listed_after_close,
            second.instance,
            stale_close,
            events
                .lock()
                .expect("events")
                .iter()
                .map(|event| event["type"].as_str().unwrap_or("").to_owned())
                .collect::<Vec<_>>(),
        ),
        (
            vec![],
            SurfaceInstanceId::new(1),
            Err(SurfaceError::InstanceNotFound(SurfaceInstanceId::new(0))),
            vec![
                "opened".to_owned(),
                "closed".to_owned(),
                "opened".to_owned()
            ],
        )
    );
}

/// Verifies a disabled plugin and an unknown surface are refused before any window exists, and
/// that an embedded request degrades to windowed on a build without child webviews.
#[test]
fn refuses_disabled_plugins_and_unknown_surfaces() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut gateway = FakeGateway::new(temp.path().to_path_buf());
    gateway.enabled = false;
    let (app, service, _) = harness(gateway);

    let disabled = service.open(&plugin(), "market", MountTarget::Embedded);
    let unknown_plugin = service.open(
        &PluginId::new("acme.tools"),
        "market",
        MountTarget::Windowed,
    );

    assert_eq!(
        (disabled, unknown_plugin, surface_window_count(&app)),
        (
            Err(SurfaceError::PluginDisabled(plugin())),
            Err(SurfaceError::PluginNotFound(PluginId::new("acme.tools"))),
            0
        )
    );
}

/// Verifies `close_all` (the lifecycle's `SurfaceCloser` path) closes the plugin's instances
/// and that an unknown surface id is reported.
#[test]
fn close_all_closes_every_instance_of_the_plugin() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (_app, service, _) = harness(FakeGateway::new(temp.path().to_path_buf()));
    service
        .open(&plugin(), "market", MountTarget::Windowed)
        .expect("open");
    let unknown_surface = service.open(&plugin(), "docs", MountTarget::Windowed);

    service.close_all(&plugin());

    assert_eq!(
        (service.list(), unknown_surface),
        (
            vec![],
            Err(SurfaceError::SurfaceNotFound {
                plugin_id: plugin(),
                surface_id: "docs".to_owned(),
            })
        )
    );
}

/// Drives the download hook through a full transfer and returns what the coordinator did.
fn run_download(success: bool) -> (PathBuf, PathBuf, bool, bool, Vec<Value>) {
    let temp = tempfile::tempdir().expect("tempdir");
    let (_app, service, events) = harness(FakeGateway::new(temp.path().to_path_buf()));
    let record = service
        .open(&plugin(), "market", MountTarget::Windowed)
        .expect("open");
    let url = Url::parse("https://cdn.skillhub.cn/abc.zip").expect("url");
    let mut destination = temp.path().join("browser-default").join("abc.zip");
    let accepted = service.downloads.handle(
        &record.label,
        Some(Url::parse("https://www.skillhub.cn/skills/abc").expect("url")),
        DownloadEvent::Requested {
            url: url.clone(),
            destination: &mut destination,
        },
    );
    std::fs::write(&destination, b"zip bytes").expect("write part file");
    let finished = service.downloads.handle(
        &record.label,
        None,
        DownloadEvent::Finished {
            url,
            path: None,
            success,
        },
    );
    let events = events.lock().expect("events").clone();
    (destination, temp.keep(), accepted, finished, events)
}

/// Verifies a download is reserved as `.part` inside the plugin's `downloads/` directory,
/// promoted on success, and announced to the frontend.
#[test]
fn download_hook_writes_part_file_and_promotes_it() {
    let (part_path, root, accepted, finished, events) = run_download(true);
    let final_path = root
        .join("plugin-data")
        .join(PLUGIN)
        .join("downloads")
        .join("abc.zip");

    assert_eq!(
        (
            part_path.clone(),
            accepted,
            finished,
            part_path.exists(),
            std::fs::read(&final_path).ok(),
            events,
        ),
        (
            final_path.with_file_name("abc.zip.part"),
            true,
            true,
            false,
            Some(b"zip bytes".to_vec()),
            vec![
                json!({
                    "type": "opened",
                    "instance": 0,
                    "pluginId": PLUGIN,
                    "surfaceId": "market",
                    "target": "windowed",
                    "title": "SkillHub",
                }),
                json!({
                    "type": "downloadStarted",
                    "instance": 0,
                    "pluginId": PLUGIN,
                    "fileName": "abc.zip",
                }),
                json!({
                    "type": "downloadCompleted",
                    "instance": 0,
                    "pluginId": PLUGIN,
                    "fileName": "abc.zip",
                    "path": final_path.display().to_string(),
                }),
            ]
        )
    );
    let _ = std::fs::remove_dir_all(root);
}

/// Verifies a failed transfer removes the `.part` file and reports `downloadFailed`.
#[test]
fn failed_download_removes_part_file() {
    let (part_path, root, accepted, finished, events) = run_download(false);
    let final_path = root
        .join("plugin-data")
        .join(PLUGIN)
        .join("downloads")
        .join("abc.zip");

    assert_eq!(
        (
            accepted,
            finished,
            part_path.exists(),
            final_path.exists(),
            events.last().cloned(),
        ),
        (
            true,
            true,
            false,
            false,
            Some(json!({
                "type": "downloadFailed",
                "instance": 0,
                "pluginId": PLUGIN,
                "fileName": "abc.zip",
                "reason": "the browser engine reported a failed transfer",
            }))
        )
    );
    let _ = std::fs::remove_dir_all(root);
}

/// Builds a record without a mock app for the pure param builders.
fn record() -> SurfaceRecord {
    let registry = ora_surface::SurfaceRegistry::default();
    let gateway = FakeGateway::new(PathBuf::from("/unused"));
    let surface = gateway
        .installed_surfaces(&plugin())
        .expect("installed")
        .remove(0);
    let (record, _) = registry
        .open(
            ora_surface::SurfaceDefinition::from_installed(&plugin(), &surface),
            MountTarget::Windowed,
        )
        .expect("open");
    SurfaceRecord {
        instance: SurfaceInstanceId::new(7),
        ..record
    }
}

/// Pins the `ui/downloadCompleted` params shape from 05 §4 with a fixed local timestamp.
#[test]
fn download_completed_params_match_the_contract() {
    let download = CompletedDownload {
        id: DownloadId::new(12),
        page_url: Some(Url::parse("https://www.skillhub.cn/skills/abc").expect("url")),
        source_url: Url::parse("https://cdn.skillhub.cn/abc.zip").expect("url"),
        file_name: "abc.zip".to_owned(),
        path: PathBuf::from("/home/u/plugin-data/ora-space.skillhub/downloads/abc.zip"),
        size_bytes: 10240,
        completed_at: datetime!(2026-08-20 16:30:00 +08:00),
    };

    assert_eq!(
        download_completed_params(&record(), 3, &download),
        json!({
            "surfaceId": "market",
            "instanceId": 7,
            "generation": 3,
            "download": {
                "id": 12,
                "pageUrl": "https://www.skillhub.cn/skills/abc",
                "sourceUrl": "https://cdn.skillhub.cn/abc.zip",
                "fileName": "abc.zip",
                "path": "/home/u/plugin-data/ora-space.skillhub/downloads/abc.zip",
                "sizeBytes": 10240,
                "completedAt": "2026-08-20T16:30:00+08:00",
            },
        })
    );
}

/// Pins the `ui/surfaceOpened` / `ui/surfaceClosed` params shape from 06 §1.2.
#[test]
fn session_params_match_the_contract() {
    assert_eq!(
        session_params(&record(), 3),
        json!({ "surfaceId": "market", "instanceId": 7, "generation": 3 })
    );
}

/// Verifies a completed download can reveal a hidden main window before the toast is shown.
#[test]
fn brings_the_main_window_forward_for_completed_downloads() {
    let app = mock_app();
    let handle = app.handle().clone();
    let main_window = WebviewWindowBuilder::new(
        &handle,
        MAIN_WINDOW_LABEL,
        WebviewUrl::App("index.html".into()),
    )
    .visible(false)
    .build()
    .expect("create hidden main window");

    bring_main_window_forward(&handle);

    assert_eq!(main_window.is_visible().expect("read visibility"), true);
}

/// Two process-start waiters resolving on the same Starting -> Running transition (a
/// `Replay::All` one from the first open while the process was stopped and a `Replay::Only`
/// one from a second open while it was starting) announce each instance exactly once.
#[test]
fn concurrent_process_starts_announce_each_instance_once() {
    let registry = Arc::new(ora_surface::SurfaceRegistry::default());
    let gateway = FakeGateway::new(PathBuf::from("/unused"));
    let surface = gateway
        .installed_surfaces(&plugin())
        .expect("installed")
        .remove(0);
    let mut instances = Vec::new();
    for surface_id in ["market", "second"] {
        let mut definition = ora_surface::SurfaceDefinition::from_installed(&plugin(), &surface);
        definition.id.surface_id = SurfaceId::parse(surface_id).expect("surface id");
        let (record, effects) = registry
            .open(definition, MountTarget::Windowed)
            .expect("open");
        let ora_surface::SurfaceEffect::CreateWebview { operation, .. } = effects[0] else {
            panic!("expected create effect");
        };
        registry
            .complete(
                record.instance,
                ora_surface::SurfaceCompletion::Opened {
                    operation,
                    outcome: Ok(MountTarget::Windowed),
                },
            )
            .expect("complete");
        instances.push(record.instance.value());
    }
    let waiters = [
        ProcessStart::<FakeConnection>::Await {
            plugin_id: plugin(),
            replay: Replay::All,
            registry: registry.clone(),
        },
        ProcessStart::Await {
            plugin_id: plugin(),
            replay: Replay::Only(instances[1]),
            registry: registry.clone(),
        },
    ];

    tauri::async_runtime::block_on(async {
        for waiter in waiters {
            waiter.run(&gateway).await;
        }
    });

    let calls = gateway.connection.calls.lock().expect("calls").clone();
    assert_eq!(
        calls,
        vec![
            (
                "ui/surfaceOpened".to_owned(),
                json!({ "surfaceId": "market", "instanceId": instances[0], "generation": 3 })
            ),
            (
                "ui/surfaceOpened".to_owned(),
                json!({ "surfaceId": "second", "instanceId": instances[1], "generation": 3 })
            ),
        ]
    );
}
