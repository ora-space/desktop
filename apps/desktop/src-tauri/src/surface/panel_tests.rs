//! Host-level tests of the panel path: asset serving, the request bridge, and push routing,
//! driven against `tauri::test::mock_app` with the fake gateway from `tests.rs`.

use crate::surface::bridge::{BridgeError, HostErrorCode, MAX_PAYLOAD_BYTES};
use crate::surface::panel_assets::{AssetOutcome, asset_response, resolve_asset};
use crate::surface::push::PushRejection;
use crate::surface::tests::{FakeGateway, FakeProcess, PANEL_PLUGIN, PLUGIN, harness};
use ora_domain::PluginId;
use ora_plugin_lifecycle::{InboundNotification, PluginGeneration};
use ora_surface::{MountTarget, SurfaceRecord};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use tauri::Manager;

const LABEL: &str = "panel-surface:official_ora-space_hello-panel:counter:0";

/// Writes the page, a nested script, a file with a refused extension, and a decoy outside the
/// asset root that a traversal would reach.
fn write_assets(panel_root: &Path) {
    fs::create_dir_all(panel_root.join("nested")).expect("asset dirs");
    fs::write(panel_root.join("index.html"), "<html></html>").expect("index");
    fs::write(panel_root.join("nested").join("app.js"), "export {};").expect("script");
    fs::write(panel_root.join("tool.exe"), "MZ").expect("binary");
    let package_root = panel_root.parent().expect("package root");
    fs::write(package_root.join("orax.toml"), "resolver = 1").expect("decoy");
}

/// Opens the panel surface in a harness whose gateway reports a running process.
fn open_panel(
    process: FakeProcess,
) -> (
    tauri::App<tauri::test::MockRuntime>,
    std::sync::Arc<crate::surface::tests::TestService>,
    FakeGateway,
    SurfaceRecord,
) {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut gateway = FakeGateway::new(temp.path().to_path_buf());
    gateway.process = process;
    write_assets(&gateway.panel_root());
    let (app, service, _) = harness(gateway.clone());
    let record = service
        .open(
            &PluginId::new("official", PANEL_PLUGIN).expect("plugin id"),
            "counter",
            MountTarget::Windowed,
        )
        .expect("open panel");
    // The tempdir must outlive the test; leaking it keeps the fixture simple and the OS cleans up.
    std::mem::forget(temp);
    (app, service, gateway, record)
}

/// Verifies a panel opens as a windowed webview with the panel label family and custom URL.
#[test]
fn opens_panel_with_panel_label_and_asset_url() {
    let (app, _service, _gateway, record) = open_panel(FakeProcess::Stopped);

    let window = app.get_webview_window(LABEL).expect("panel window");
    assert_eq!(
        (
            record.label.as_str(),
            window.url().expect("url").to_string(),
        ),
        (
            LABEL,
            "ora-plugin://localhost/official/ora-space.hello-panel/counter/index.html".to_owned(),
        )
    );
}

/// Table of asset requests: only files below the caller's own asset root with a servable
/// extension are returned; everything else is a 404 whose reason stays in the log.
#[test]
fn asset_resolution_table() {
    let (_app, service, _gateway, _record) = open_panel(FakeProcess::Stopped);
    let registry = &service.registry;
    let serve = |content_type: &'static str, body: &str, document: bool| AssetOutcome::Serve {
        content_type,
        body: body.as_bytes().to_vec(),
        document,
    };
    let cases = [
        (
            LABEL,
            "/official/ora-space.hello-panel/counter/index.html",
            serve("text/html; charset=utf-8", "<html></html>", true),
        ),
        (
            LABEL,
            "/official/ora-space.hello-panel/counter/",
            serve("text/html; charset=utf-8", "<html></html>", true),
        ),
        (
            LABEL,
            "/official/ora-space.hello-panel/counter/nested/app.js",
            serve("text/javascript; charset=utf-8", "export {};", false),
        ),
        (
            LABEL,
            "/official/ora-space.hello-panel/counter/nested/%61pp.js",
            serve("text/javascript; charset=utf-8", "export {};", false),
        ),
        (
            LABEL,
            "/official/ora-space.hello-panel/counter/../orax.toml",
            AssetOutcome::NotFound("path is not a safe relative path"),
        ),
        (
            LABEL,
            "/official/ora-space.hello-panel/counter/%2e%2e/orax.toml",
            AssetOutcome::NotFound("path is not a safe relative path"),
        ),
        (
            LABEL,
            "/official/ora-space.hello-panel/counter/tool.exe",
            AssetOutcome::NotFound("extension is not servable"),
        ),
        (
            LABEL,
            "/official/ora-space.hello-panel/counter/nested",
            AssetOutcome::NotFound("path is not a regular file"),
        ),
        (
            LABEL,
            "/official/ora-space.hello-panel/counter/missing.css",
            AssetOutcome::NotFound("file does not resolve inside the asset root"),
        ),
        (
            LABEL,
            "/official/ora-space.skillhub/counter/index.html",
            AssetOutcome::NotFound("path names another plugin or surface"),
        ),
        (
            LABEL,
            "/official/ora-space.hello-panel/other/index.html",
            AssetOutcome::NotFound("path names another plugin or surface"),
        ),
        (
            LABEL,
            "/official/ora-space.hello-panel",
            AssetOutcome::NotFound("path lacks plugin and surface segments"),
        ),
        (
            "main",
            "/official/ora-space.hello-panel/counter/index.html",
            AssetOutcome::NotFound("label is not a live surface"),
        ),
        (
            "panel-surface:official_ora-space_hello-panel:counter:9",
            "/official/ora-space.hello-panel/counter/index.html",
            AssetOutcome::NotFound("label is not a live surface"),
        ),
    ];
    for (label, path, expected) in cases {
        assert_eq!(
            resolve_asset(registry, label, path),
            expected,
            "{label} {path}"
        );
    }
}

/// Verifies a remote-site label is refused by the asset handler even though it is live.
#[test]
fn remote_site_label_cannot_fetch_panel_assets() {
    let (_app, service, _gateway, _record) = open_panel(FakeProcess::Stopped);
    let remote = service
        .open(
            &PluginId::new("official", PLUGIN).expect("plugin id"),
            "market",
            MountTarget::Windowed,
        )
        .expect("open remote");

    assert_eq!(
        resolve_asset(
            &service.registry,
            remote.label.as_str(),
            "/official/ora-space.hello-panel/counter/index.html"
        ),
        AssetOutcome::NotFound("surface is not a panel")
    );
}

/// Verifies the HTTP projection: documents carry the CSP and are uncached, other files must be
/// revalidated on every load, refusals are bare 404s.
#[test]
fn asset_response_headers() {
    let (_app, service, _gateway, _record) = open_panel(FakeProcess::Stopped);
    let registry = &service.registry;
    let header = |response: &tauri::http::Response<Vec<u8>>, name: &str| {
        response
            .headers()
            .get(name)
            .map(|value| value.to_str().expect("ascii").to_owned())
    };

    let document = asset_response(
        registry,
        LABEL,
        "/official/ora-space.hello-panel/counter/index.html",
    );
    let script = asset_response(
        registry,
        LABEL,
        "/official/ora-space.hello-panel/counter/nested/app.js",
    );
    let refused = asset_response(
        registry,
        LABEL,
        "/official/ora-space.hello-panel/counter/tool.exe",
    );

    assert_eq!(
        (
            document.status().as_u16(),
            header(&document, "cache-control"),
            header(&document, "x-content-type-options"),
            header(&document, "content-security-policy").map(|csp| csp.contains(
                "script-src ora-plugin://localhost/official/ora-space.hello-panel/counter/;"
            )),
            script.status().as_u16(),
            header(&script, "cache-control"),
            header(&script, "content-security-policy"),
            refused.status().as_u16(),
        ),
        (
            200,
            Some("no-store".to_owned()),
            Some("nosniff".to_owned()),
            Some(true),
            200,
            Some("no-cache".to_owned()),
            None,
            404,
        )
    );
}

/// Verifies a bridge request reaches the plugin with the session params and the answer's
/// payload is returned; a remote-site label and an oversized payload are refused before that.
#[tokio::test]
async fn bridge_request_round_trip_and_refusals() {
    let (_app, service, gateway, _record) = open_panel(FakeProcess::Running);
    let remote = service
        .open(
            &PluginId::new("official", PLUGIN).expect("plugin id"),
            "market",
            MountTarget::Windowed,
        )
        .expect("open remote");

    let answer = service.request(LABEL, json!({ "type": "increment" })).await;
    let from_remote = service.request(remote.label.as_str(), json!({})).await;
    let from_unknown = service.request("main", json!({})).await;
    let oversized = service
        .request(LABEL, Value::String("x".repeat(MAX_PAYLOAD_BYTES)))
        .await;
    let calls = gateway.connection.calls.lock().expect("calls").clone();
    let request_call = calls
        .iter()
        .find(|(method, _)| method == "ora/ui/request")
        .cloned();

    assert_eq!(
        (answer, from_remote, from_unknown, oversized, request_call),
        (
            Ok(json!({ "echo": { "type": "increment" } })),
            Err(BridgeError::Host {
                code: HostErrorCode::SurfaceClosed
            }),
            Err(BridgeError::Host {
                code: HostErrorCode::SurfaceClosed
            }),
            Err(BridgeError::Host {
                code: HostErrorCode::PayloadTooLarge
            }),
            Some((
                "ora/ui/request".to_owned(),
                json!({
                    "surface_id": "counter",
                    "surface_instance_id": 0,
                    "plugin_generation": 3,
                    "payload": { "type": "increment" },
                }),
            )),
        )
    );
}

/// Builds one `ora/ui/push` notification from the panel plugin.
fn push(plugin: &str, generation: u64, params: Value) -> InboundNotification {
    InboundNotification {
        plugin_id: PluginId::new("official", plugin).expect("plugin id"),
        generation: PluginGeneration(generation),
        method: "ora/ui/push".to_owned(),
        params,
    }
}

/// Table of push deliveries: only a well-formed push from the current generation of the owning
/// plugin to a live panel instance reaches the page, and sequence numbers count per instance.
/// A push whose own `plugin_generation` disagrees with the emitting process is stale too.
#[test]
fn push_delivery_table() {
    let (_app, service, _gateway, _record) = open_panel(FakeProcess::Running);
    let remote = service
        .open(
            &PluginId::new("official", PLUGIN).expect("plugin id"),
            "market",
            MountTarget::Windowed,
        )
        .expect("open remote");
    let session =
        json!({ "surface_id": "counter", "surface_instance_id": 0, "plugin_generation": 3 });
    let with_payload = |payload: Value| {
        let mut params = session.clone();
        params["payload"] = payload;
        params
    };

    let outcomes = [
        service.deliver_push(&push(
            PANEL_PLUGIN,
            3,
            with_payload(json!({ "type": "tick" })),
        )),
        service.deliver_push(&push(PANEL_PLUGIN, 3, with_payload(json!(2)))),
        service.deliver_push(&push(PANEL_PLUGIN, 2, with_payload(json!(3)))),
        service.deliver_push(&push(PLUGIN, 3, with_payload(json!(4)))),
        service.deliver_push(&push(
            PANEL_PLUGIN,
            3,
            json!({ "surface_id": "other", "surface_instance_id": 0, "plugin_generation": 3, "payload": 5 }),
        )),
        service.deliver_push(&push(
            PANEL_PLUGIN,
            3,
            json!({ "surface_id": "counter", "surface_instance_id": 42, "plugin_generation": 3, "payload": 6 }),
        )),
        service.deliver_push(&push(
            PLUGIN,
            3,
            json!({ "surface_id": "market", "surface_instance_id": remote.instance.value(), "plugin_generation": 3, "payload": 7 }),
        )),
        service.deliver_push(&push(PANEL_PLUGIN, 3, json!({ "payload": 8 }))),
        service.deliver_push(&push(
            PANEL_PLUGIN,
            3,
            json!({ "surface_id": "counter", "surface_instance_id": 0, "payload": 10 }),
        )),
        service.deliver_push(&push(
            PANEL_PLUGIN,
            3,
            json!({ "surface_id": "counter", "surface_instance_id": 0, "plugin_generation": 2, "payload": 11 }),
        )),
        service.deliver_push(&InboundNotification {
            method: "ora/ui/other".to_owned(),
            ..push(PANEL_PLUGIN, 3, with_payload(json!(9)))
        }),
    ];

    assert_eq!(
        outcomes,
        [
            Ok(1),
            Ok(2),
            Err(PushRejection::StaleGeneration),
            Err(PushRejection::SessionMismatch),
            Err(PushRejection::SessionMismatch),
            Err(PushRejection::UnknownInstance),
            Err(PushRejection::NotPanel),
            Err(PushRejection::MalformedParams),
            Err(PushRejection::MalformedParams),
            Err(PushRejection::StaleGeneration),
            Err(PushRejection::NotPush),
        ]
    );
}
