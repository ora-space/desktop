//! Host-level tests driving `SurfaceService` against `tauri::test::mock_app` with a fake plugin
//! gateway, covering the surface lifecycle, the workbench asset protocol and bridge, and the
//! webview download disposition.

use crate::surface::download_actions::DownloadActionHost;
use crate::surface::gateway::{GatewayFailure, SurfaceConnection, SurfacePluginGateway};
use crate::surface::workbench_assets::{AssetOutcome, resolve_asset};
use crate::surface::workbench_bridge::{HostErrorCode, WorkbenchInvokeRequest};
use crate::surface::{MAIN_WINDOW_LABEL, SURFACE_EVENT, SurfaceService};
use ora_domain::PluginId;
use ora_plugin_lifecycle::{PluginCallError, PluginGenerationKey};
use ora_plugin_manifest::{
    DownloadAction, DownloadDisposition, DownloadPolicy, DownloadRule, MethodName, Origin,
    PageMatcher, PathPrefix, StartUrl,
};
use ora_surface::{
    MountTarget, RemoteSiteDefinition, SurfaceDefinition, SurfaceInstanceId, SurfaceSource,
    WebviewLabel, WorkbenchDefinition,
};
use ora_utils::path::PortableRelativePath;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::test::{MockRuntime, mock_app};
use tauri::{App, Listener, Manager, Url, WebviewUrl, WebviewWindowBuilder};

const WEBVIEW_PLUGIN: &str = "acme.hub";
const WORKBENCH_PLUGIN: &str = "acme.panel";

/// Records every JSON-RPC call the fake plugin process receives and echoes the input back.
#[derive(Clone)]
struct FakeConnection {
    calls: Arc<Mutex<Vec<(String, Value)>>>,
    registered: HashSet<String>,
    /// Mutable so tests can simulate a plugin process restart between two bridge calls.
    generation: Arc<AtomicU64>,
}

impl Default for FakeConnection {
    fn default() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            registered: HashSet::from(["counter/get".to_owned(), "counter/increment".to_owned()]),
            generation: Arc::new(AtomicU64::new(3)),
        }
    }
}

impl SurfaceConnection for FakeConnection {
    fn key(&self) -> PluginGenerationKey {
        PluginGenerationKey(self.generation.load(Ordering::SeqCst))
    }

    fn registered_methods(&self) -> HashSet<String> {
        self.registered.clone()
    }

    async fn invoke(&self, method: &str, params: Value) -> Result<Value, PluginCallError> {
        self.calls
            .lock()
            .expect("calls")
            .push((method.to_owned(), params.clone()));
        // Echo the envelope's input back as the result so tests can inspect what the host built.
        Ok(params.get("input").cloned().unwrap_or(Value::Null))
    }
}

/// Fake gateway contributing one webview plugin and one workbench plugin.
#[derive(Clone)]
struct FakeGateway {
    data_root: PathBuf,
    connection: FakeConnection,
    enabled: bool,
}

impl FakeGateway {
    fn new(data_root: PathBuf) -> Self {
        Self {
            data_root,
            connection: FakeConnection::default(),
            enabled: true,
        }
    }

    /// The workbench asset directory the gateway serves; tests populate it before opening.
    fn workbench_assets(&self) -> PathBuf {
        self.data_root.join("assets")
    }
}

impl SurfacePluginGateway for FakeGateway {
    type Connection = FakeConnection;

    fn surface_definition(&self, plugin_id: &PluginId) -> Option<SurfaceDefinition> {
        match plugin_id.name() {
            WEBVIEW_PLUGIN => Some(SurfaceDefinition {
                plugin_id: plugin_id.clone(),
                title: "Example Hub".to_owned(),
                source: SurfaceSource::RemoteSite(RemoteSiteDefinition {
                    start_url: StartUrl::parse("https://www.example.com/skills")
                        .expect("url")
                        .as_url()
                        .clone(),
                    navigation: ora_surface::NavigationPolicy::remote_site(vec![
                        Origin::parse("https://www.example.com").expect("origin"),
                    ]),
                    download_policy: example_policy(),
                }),
            }),
            WORKBENCH_PLUGIN => Some(SurfaceDefinition {
                plugin_id: plugin_id.clone(),
                title: "Example Panel".to_owned(),
                source: SurfaceSource::Workbench(WorkbenchDefinition {
                    asset_root: self.workbench_assets(),
                    page_entry: PortableRelativePath::parse("index.html").expect("entry"),
                    declared_methods: vec![MethodName::parse("counter/get").expect("method")],
                }),
            }),
            _ => None,
        }
    }

    fn plugin_enabled(&self, _plugin_id: &PluginId) -> bool {
        self.enabled
    }

    fn data_directory(&self, _plugin_id: &PluginId) -> Result<PathBuf, GatewayFailure> {
        Ok(self.data_root.clone())
    }

    async fn ensure_running(
        &self,
        _plugin_id: &PluginId,
        _wait: Duration,
    ) -> Result<FakeConnection, GatewayFailure> {
        Ok(self.connection.clone())
    }

    async fn stop_if_idle(&self, _plugin_id: &PluginId) -> Result<(), GatewayFailure> {
        Ok(())
    }
}

/// The example policy: `/skills/` auto-imports, `/files/` prompts, everything else rejects.
fn example_policy() -> DownloadPolicy {
    DownloadPolicy {
        rules: vec![
            DownloadRule {
                page: PageMatcher {
                    origin: Origin::parse("https://www.example.com").expect("origin"),
                    path_prefix: PathPrefix::parse("/skills/").expect("prefix"),
                },
                disposition: DownloadDisposition::Auto {
                    action: DownloadAction::ImportSkill,
                },
            },
            DownloadRule {
                page: PageMatcher {
                    origin: Origin::parse("https://www.example.com").expect("origin"),
                    path_prefix: PathPrefix::parse("/files/").expect("prefix"),
                },
                disposition: DownloadDisposition::Prompt {
                    actions: vec![DownloadAction::ImportSkill, DownloadAction::SaveAs],
                },
            },
        ],
        fallback: DownloadDisposition::Reject,
    }
}

/// Serves scripted skill-import results so tests can observe the automatic action outcome.
struct FakeActionHost {
    results: Mutex<VecDeque<Result<String, String>>>,
    imported: Mutex<Vec<(PathBuf, String)>>,
}

impl FakeActionHost {
    fn new(results: Vec<Result<String, String>>) -> Arc<Self> {
        Arc::new(Self {
            results: Mutex::new(results.into()),
            imported: Mutex::new(Vec::new()),
        })
    }
}

impl DownloadActionHost for FakeActionHost {
    fn prepare_skill_import(
        &self,
        archive: &Path,
        file_name: &str,
    ) -> Result<String, ora_backend::BackendError> {
        self.imported
            .lock()
            .expect("imported")
            .push((archive.to_path_buf(), file_name.to_owned()));
        self.results
            .lock()
            .expect("results")
            .pop_front()
            .expect("scripted result")
            .map_err(|reason| {
                ora_backend::BackendError::new(
                    ora_backend::ErrorClassification::Internal,
                    ora_contracts::PublicError::InternalError(ora_contracts::EmptyErrorParams {}),
                    reason,
                )
            })
    }
}

/// Waits until `predicate` holds over the captured events or a bounded budget elapses.
fn wait_for_events(events: &Arc<Mutex<Vec<Value>>>, predicate: impl Fn(&[Value]) -> bool) {
    for _ in 0..200 {
        if predicate(&events.lock().expect("events")) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

type TestService = SurfaceService<FakeGateway, MockRuntime>;

/// Builds a mock app with a main window plus the surface service, capturing surface events.
fn harness(gateway: FakeGateway) -> (App<MockRuntime>, Arc<TestService>, Arc<Mutex<Vec<Value>>>) {
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
    let service = SurfaceService::new(app.handle().clone(), gateway);
    (app, service, events)
}

fn webview_plugin() -> PluginId {
    PluginId::new("official", WEBVIEW_PLUGIN).expect("plugin id")
}

fn workbench_plugin() -> PluginId {
    PluginId::new("official", WORKBENCH_PLUGIN).expect("plugin id")
}

/// A webview surface is a singleton: opening it twice yields one window and the same instance.
#[test]
fn opening_a_webview_singleton_twice_reuses_the_instance() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (app, service, _events) = harness(FakeGateway::new(temp.path().to_path_buf()));

    let first = service
        .open(&webview_plugin(), MountTarget::Windowed)
        .expect("first open");
    let second = service
        .open(&webview_plugin(), MountTarget::Windowed)
        .expect("second open");

    let windows = app
        .webview_windows()
        .keys()
        .filter(|label| label.starts_with(WebviewLabel::REMOTE_PREFIX))
        .count();
    assert_eq!(
        (
            first.instance,
            second.instance,
            windows,
            service.list().len()
        ),
        (SurfaceInstanceId::new(0), SurfaceInstanceId::new(0), 1, 1)
    );
}

/// A disabled plugin cannot be opened, and an unknown plugin is not found.
#[test]
fn refuses_disabled_and_unknown_plugins() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut gateway = FakeGateway::new(temp.path().to_path_buf());
    gateway.enabled = false;
    let (_app, service, _events) = harness(gateway);

    let disabled = service.open(&webview_plugin(), MountTarget::Windowed);
    let unknown = service.open(
        &PluginId::new("official", "acme.missing").expect("plugin id"),
        MountTarget::Windowed,
    );

    assert_eq!((disabled.is_err(), unknown.is_err()), (true, true));
}

/// The workbench asset protocol serves a file below the instance's asset root, marks the HTML
/// entry as a document, and refuses a path that names another instance.
#[test]
fn workbench_assets_serve_by_instance_and_refuse_other_instances() {
    let temp = tempfile::tempdir().expect("tempdir");
    let gateway = FakeGateway::new(temp.path().to_path_buf());
    std::fs::create_dir_all(gateway.workbench_assets()).expect("assets dir");
    std::fs::write(
        gateway.workbench_assets().join("index.html"),
        "<html></html>",
    )
    .expect("page");
    std::fs::write(gateway.workbench_assets().join("app.js"), "export {};").expect("script");
    let (_app, service, _events) = harness(gateway);

    let record = service
        .open(&workbench_plugin(), MountTarget::Windowed)
        .expect("open workbench");
    let registry = &service.registry;
    let instance = record.instance.value();

    let entry = resolve_asset(&registry, record.label.as_str(), &format!("/{instance}/"));
    let script = resolve_asset(
        &registry,
        record.label.as_str(),
        &format!("/{instance}/app.js"),
    );
    let other = resolve_asset(&registry, record.label.as_str(), "/999/index.html");
    let escape = resolve_asset(
        &registry,
        record.label.as_str(),
        &format!("/{instance}/../secret"),
    );

    assert_eq!(
        (
            matches!(entry, AssetOutcome::Serve { document: true, .. }),
            matches!(script, AssetOutcome::Serve { document: false, content_type, .. } if content_type.starts_with("text/javascript")),
            other,
            escape,
        ),
        (
            true,
            true,
            AssetOutcome::NotFound("path names another instance"),
            AssetOutcome::NotFound("path is not a safe relative path"),
        )
    );
}

/// The workbench bridge rejects a method outside the manifest allowlist, one the running
/// generation did not register, and forwards an allowed method inside a host envelope.
#[tokio::test]
async fn workbench_bridge_enforces_the_effective_method_set() {
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(temp.path().join("assets")).expect("assets");
    std::fs::write(
        temp.path().join("assets").join("index.html"),
        "<html></html>",
    )
    .expect("page");
    let gateway = FakeGateway::new(temp.path().to_path_buf());
    let calls = gateway.connection.calls.clone();
    let (_app, service, _events) = harness(gateway);
    let record = service
        .open(&workbench_plugin(), MountTarget::Windowed)
        .expect("open workbench");
    let label = record.label.as_str().to_owned();

    let not_allowed = service
        .workbench_invoke(&label, request("weather/get", json!({})))
        .await;
    // `counter/increment` is registered by the fake but not declared in the manifest allowlist.
    let not_declared = service
        .workbench_invoke(&label, request("counter/increment", json!({})))
        .await;
    let ok = service
        .workbench_invoke(&label, request("counter/get", json!({ "city": "SH" })))
        .await;

    let recorded = calls.lock().expect("calls").clone();
    assert_eq!(
        (not_allowed, not_declared, ok, recorded,),
        (
            Err(host_error(HostErrorCode::MethodNotAllowed)),
            Err(host_error(HostErrorCode::MethodNotAllowed)),
            Ok(json!({ "city": "SH" })),
            vec![(
                "counter/get".to_owned(),
                json!({
                    "surface": { "instance_id": record.instance.value(), "generation": 3 },
                    "input": { "city": "SH" },
                }),
            )],
        )
    );
}

/// The bridge refuses a non-workbench (remote-site) caller.
#[tokio::test]
async fn workbench_bridge_refuses_remote_site_callers() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (_app, service, _events) = harness(FakeGateway::new(temp.path().to_path_buf()));
    let record = service
        .open(&webview_plugin(), MountTarget::Windowed)
        .expect("open webview");

    let result = service
        .workbench_invoke(record.label.as_str(), request("counter/get", json!({})))
        .await;

    assert_eq!(result, Err(host_error(HostErrorCode::SurfaceUnavailable)));
}

/// The Tauri command wrapper takes the payload from the top-level `request` key, which is the
/// body shape the injected `workbench_api.js` sends; this pins the script-to-command contract.
#[test]
fn plugin_webview_invoke_command_extracts_the_request_envelope() {
    // The command under test, registered under its production name against the test service.
    #[tauri::command]
    async fn plugin_webview_invoke(
        webview: tauri::Webview<MockRuntime>,
        state: tauri::State<'_, Arc<TestService>>,
        request: WorkbenchInvokeRequest,
    ) -> Result<Value, crate::surface::workbench_bridge::BridgeError> {
        state.workbench_invoke(webview.label(), request).await
    }

    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(temp.path().join("assets")).expect("assets");
    std::fs::write(
        temp.path().join("assets").join("index.html"),
        "<html></html>",
    )
    .expect("page");
    // The mock context ships an empty ACL that denies every command; allow exactly the bridge
    // command, mirroring the production `plugin-webviews` capability.
    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    context.runtime_authority_mut().__allow_command(
        "plugin_webview_invoke".to_owned(),
        tauri::utils::acl::ExecutionContext::Local,
    );
    let app = tauri::test::mock_builder()
        .invoke_handler(tauri::generate_handler![plugin_webview_invoke])
        .build(context)
        .expect("mock app");
    let service = SurfaceService::new(
        app.handle().clone(),
        FakeGateway::new(temp.path().to_path_buf()),
    );
    app.manage(service.clone());
    let record = service
        .open(&workbench_plugin(), MountTarget::Windowed)
        .expect("open workbench");
    let webview = app
        .get_webview_window(record.label.as_str())
        .expect("workbench webview window");

    let response = tauri::test::get_ipc_response(
        &webview,
        tauri::webview::InvokeRequest {
            cmd: "plugin_webview_invoke".into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            // Tauri rewrites its local protocol to an HTTPS localhost origin on Windows and
            // Android; using the Unix spelling there makes the ACL classify this as remote.
            url: if cfg!(any(windows, target_os = "android")) {
                "https://tauri.localhost"
            } else {
                "tauri://localhost"
            }
            .parse()
            .expect("url"),
            // The exact body `workbench_api.js` sends: method and params nested under `request`.
            body: tauri::ipc::InvokeBody::Json(json!({
                "request": { "method": "counter/get", "params": { "city": "SH" } }
            })),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        },
    )
    .map(|body| body.deserialize::<Value>().expect("json result"));

    assert_eq!(
        (
            response,
            include_str!("workbench_api.js").contains("request: {"),
        ),
        (Ok(json!({ "city": "SH" })), true)
    );
}

/// A workbench instance is pinned to the process generation of its first successful call; after
/// the process restarts, the stale instance is refused and closed instead of reaching the new
/// generation with page state derived from the old one.
#[tokio::test]
async fn workbench_bridge_refuses_and_closes_instances_after_a_process_restart() {
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(temp.path().join("assets")).expect("assets");
    std::fs::write(
        temp.path().join("assets").join("index.html"),
        "<html></html>",
    )
    .expect("page");
    let gateway = FakeGateway::new(temp.path().to_path_buf());
    let generation = gateway.connection.generation.clone();
    let (_app, service, _events) = harness(gateway);
    let record = service
        .open(&workbench_plugin(), MountTarget::Windowed)
        .expect("open workbench");
    let label = record.label.as_str().to_owned();

    let first = service
        .workbench_invoke(&label, request("counter/get", json!({})))
        .await;
    // The plugin process restarts: every later connection reports a newer generation.
    generation.store(4, Ordering::SeqCst);
    let second = service
        .workbench_invoke(&label, request("counter/get", json!({})))
        .await;

    assert_eq!(
        (
            first.is_ok(),
            second,
            service.registry.record(record.instance),
        ),
        (
            true,
            Err(host_error(HostErrorCode::SurfaceUnavailable)),
            None
        )
    );
}

/// Two concurrent downloads of the same URL from one page are matched to their completions in
/// start order, so an interleaved failure settles the right download.
#[test]
fn same_url_downloads_settle_in_start_order() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (_app, service, events) = harness(FakeGateway::new(temp.path().to_path_buf()));
    let record = service
        .open(&webview_plugin(), MountTarget::Windowed)
        .expect("open webview");
    let page = Url::parse("https://www.example.com/files/list").expect("url");
    let url = Url::parse("https://cdn.example.com/pack.zip").expect("url");

    let mut first_destination = PathBuf::new();
    let mut second_destination = PathBuf::new();
    let first =
        service
            .downloads
            .requested(&record, Some(page.clone()), &url, &mut first_destination);
    let second = service
        .downloads
        .requested(&record, Some(page), &url, &mut second_destination);
    std::fs::write(&first_destination, b"one").expect("first part");
    std::fs::write(&second_destination, b"two").expect("second part");
    service.downloads.finished(&record, &url, true);
    service.downloads.finished(&record, &url, false);

    // The lifecycle `opened` event precedes the download traffic; only the latter matters here.
    wait_for_events(&events, |events| {
        events
            .iter()
            .filter(|event| event["downloadId"].is_u64())
            .count()
            >= 4
    });
    let summary: Vec<(String, u64)> = events
        .lock()
        .expect("events")
        .iter()
        .filter(|event| event["downloadId"].is_u64())
        .map(|event| {
            (
                event["type"].as_str().unwrap_or_default().to_owned(),
                event["downloadId"].as_u64().unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(
        (
            first,
            second,
            first_destination != second_destination,
            summary,
        ),
        (
            true,
            true,
            true,
            vec![
                ("downloadStarted".to_owned(), 1),
                ("downloadStarted".to_owned(), 2),
                ("downloadChoice".to_owned(), 1),
                ("downloadFailed".to_owned(), 2),
            ],
        )
    );
}

/// An automatic disposition runs its host action before reporting success; a failed action
/// reports `downloadFailed` and removes the landed file instead of claiming success.
#[test]
fn auto_downloads_run_the_host_action_before_completion() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (_app, service, events) = harness(FakeGateway::new(temp.path().to_path_buf()));
    let host = FakeActionHost::new(vec![
        Ok("session-9".to_owned()),
        Err("archive rejected".to_owned()),
    ]);
    service.install_download_action_host(host.clone());
    let record = service
        .open(&webview_plugin(), MountTarget::Windowed)
        .expect("open webview");
    let page = Url::parse("https://www.example.com/skills/42").expect("url");
    let url = Url::parse("https://cdn.example.com/skill.zip").expect("url");

    for _ in 0..2 {
        let mut destination = PathBuf::new();
        assert!(
            service
                .downloads
                .requested(&record, Some(page.clone()), &url, &mut destination)
        );
        std::fs::write(&destination, b"zip").expect("part");
        service.downloads.finished(&record, &url, true);
    }
    wait_for_events(&events, |events| {
        events
            .iter()
            .any(|event| event["type"] == "downloadCompleted")
            && events.iter().any(|event| event["type"] == "downloadFailed")
    });

    let captured = events.lock().expect("events").clone();
    let completed = captured
        .iter()
        .find(|event| event["type"] == "downloadCompleted")
        .cloned()
        .expect("completed event");
    let failed = captured
        .iter()
        .find(|event| event["type"] == "downloadFailed")
        .cloned()
        .expect("failed event");
    let imported = host.imported.lock().expect("imported").clone();
    assert_eq!(
        (
            completed["importSessionId"].clone(),
            completed["action"].clone(),
            imported.len(),
            // The successful import's artifact stays for the import session to consume.
            imported[0].0.exists(),
            failed["reason"].clone(),
            // The failed action's artifact is removed so nothing unreferenced accumulates.
            imported[1].0.exists(),
        ),
        (
            json!("session-9"),
            json!("import_skill"),
            2,
            true,
            json!("archive rejected"),
            false,
        )
    );
}

/// Builds a bridge request from a method and params.
fn request(method: &str, params: Value) -> WorkbenchInvokeRequest {
    serde_json::from_value(json!({ "method": method, "params": params })).expect("request")
}

/// Wraps a host error code in the bridge error union.
fn host_error(code: HostErrorCode) -> crate::surface::workbench_bridge::BridgeError {
    crate::surface::workbench_bridge::BridgeError::Host { code }
}
