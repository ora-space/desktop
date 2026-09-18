//! Behavior tests for Host MCP health at the plugin-operation surface.
//!
//! These drive the real `Plugins` composition — install, save, uninstall, and the health queries —
//! so the evidence covers the paths a user actually takes, not the store in isolation.

use super::Plugins;
use crate::agent_runtime::{AgentRuntimeManager, AgentRuntimeSetup};
use crate::app_event::AppEventHub;
use crate::clock::SystemClock;
use crate::plugin::PluginApi;
use crate::settings::Settings;
use ora_contracts::{
    AppEvent, GetPluginConfigurationRequest, ImportPluginRequest, ListMcpHealthRequest,
    ListMcpHealthResponse, McpHealthErrorCode, McpHealthStatus, McpHealthUnknownReason,
    PluginConfigurationCompleteness, PluginSettingValue, ProbeMcpHealthRequest,
    ResetPluginConfigurationMode, ResetPluginConfigurationRequest, SavePluginConfigurationRequest,
    UninstallPluginRequest,
};
use ora_db::{DatabaseBootstrapper, DatabaseLocation, RepositoryPool, default_migration_catalog};
use ora_scheduler::Scheduler;
use pretty_assertions::assert_eq;
use std::fs::File;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tempfile::TempDir;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

/// Opens a throwaway SQLite pool under `root`.
fn test_pool(root: &Path) -> RepositoryPool {
    ora_logging::initialize_test_clock();
    DatabaseBootstrapper::new(crate::test_clock::TestClock)
        .bootstrap_repository_pool(
            &DatabaseLocation::path(root.join("test.sqlite")),
            &default_migration_catalog().expect("build migration catalog"),
        )
        .expect("create repository pool")
}

/// Exercises the public plugin interface together with the event hub it publishes into.
fn test_plugins(root: &Path, pool: &RepositoryPool) -> (Plugins, AppEventHub) {
    let (plugins, events, _host) = test_plugins_with_host(root, pool);
    (plugins, events)
}

/// Same composition as [`test_plugins`], also exposing the plugin API for delivery assertions.
fn test_plugins_with_host(
    root: &Path,
    pool: &RepositoryPool,
) -> (Plugins, AppEventHub, Arc<PluginApi>) {
    let events = AppEventHub::new();
    let host = Arc::new(
        PluginApi::open(
            pool.clone(),
            root.to_path_buf(),
            std::path::PathBuf::from("deno"),
            SystemClock,
            events.publisher(),
            Arc::new(Settings::new(pool.clone())),
        )
        .expect("open plugin host"),
    );
    let runtime = Arc::new(
        AgentRuntimeManager::new(AgentRuntimeSetup {
            plugin_host: host.clone(),
            pool: pool.clone(),
            home_directory: root.to_path_buf(),
            relative_path_base: root.to_path_buf(),
            sessions_root: root.join("sessions"),
            clock: SystemClock,
            scheduler: Scheduler::new(chrono_tz::Asia::Shanghai),
            app_events: events.publisher(),
        })
        .expect("agent runtime"),
    );
    (Plugins::new(host.clone(), runtime), events, host)
}

/// How a loopback peer treats the connections it accepts.
enum PeerBehavior {
    /// Accept and hold every connection without answering: an exchange can only end by timeout.
    Hold,
    /// Accept and close immediately: any exchange fails at connect or handshake.
    Close,
}

/// Loopback TCP peer used to drive one HTTP probe outcome deterministically.
struct RawTcpPeer {
    port: u16,
    stop: mpsc::Sender<()>,
    handle: Option<JoinHandle<()>>,
}

impl RawTcpPeer {
    fn start(behavior: PeerBehavior) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind peer");
        listener.set_nonblocking(true).expect("nonblocking peer");
        let port = listener.local_addr().expect("peer address").port();
        let (stop, stop_rx) = mpsc::channel::<()>();
        let handle = thread::spawn(move || {
            let mut held: Vec<TcpStream> = Vec::new();
            loop {
                if stop_rx.try_recv().is_ok() {
                    return;
                }
                match listener.accept() {
                    Ok((stream, _)) => match behavior {
                        PeerBehavior::Hold => held.push(stream),
                        PeerBehavior::Close => drop(stream),
                    },
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => return,
                }
            }
        });
        Self {
            port,
            stop,
            handle: Some(handle),
        }
    }

    /// The MCP HTTP endpoint pointing at this peer.
    fn url(&self) -> String {
        format!("https://127.0.0.1:{}/mcp", self.port)
    }
}

impl Drop for RawTcpPeer {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Writes an MCP `.orax` archive with the given `assets/config.json` body and command file.
///
/// The stdio command starts as a regular file inside the package, so discovery accepts it; the
/// caller chooses whether it is runnable.
fn write_mcp_orax_with_command(
    path: &Path,
    identifier: &str,
    config: &str,
    command_name: &str,
    command_bytes: &[u8],
) {
    let manifest = format!(
        "resolver = 1\nidentifier = \"{identifier}\"\nnamespace = \"official\"\nkind = \"mcp\"\nversion = \"0.1.0\"\ndescription = \"MCP probe fixture\"\n"
    );
    let mut writer = ZipWriter::new(File::create(path).expect("create archive"));
    // Store entries verbatim: the runnable command may be tens of megabytes, and deflating it
    // would dominate the test for no benefit.
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    writer.start_file("orax.toml", options).expect("manifest");
    writer
        .write_all(manifest.as_bytes())
        .expect("write manifest");
    writer
        .start_file("assets/config.json", options)
        .expect("config");
    writer.write_all(config.as_bytes()).expect("write config");
    writer
        .start_file(command_name, options.unix_permissions(0o755))
        .expect("command file");
    writer.write_all(command_bytes).expect("write command");
    writer.finish().expect("finish archive");
}

/// Writes an MCP `.orax` archive whose stdio command is a regular file no platform can execute — a
/// missing interpreter for Unix and an unusable image for Windows — so both reach
/// `mcp_spawn_failed` without depending on the host's installed programs.
fn write_mcp_orax(path: &Path, identifier: &str, config: &str) {
    write_mcp_orax_with_command(
        path,
        identifier,
        config,
        "assets/server",
        b"#!/nonexistent-ora-probe-interpreter\n",
    );
}

/// Imports one MCP archive and returns its canonical plugin id.
async fn import_mcp(plugins: &Plugins, root: &Path, identifier: &str, config: &str) -> String {
    let archive = root.join(format!("{identifier}.orax"));
    write_mcp_orax(&archive, identifier, config);
    let response = plugins
        .import(ImportPluginRequest {
            path: archive.to_string_lossy().into_owned(),
        })
        .await
        .expect("import MCP package");
    response.plugin_id
}

/// Polls the card health view until the named plugin leaves `Unknown(not_probed)`.
async fn wait_for_card_health(plugins: &Plugins, plugin_id: &str) -> ListMcpHealthResponse {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let listed = plugins
            .list_mcp_health(ListMcpHealthRequest { cwd: None })
            .expect("list mcp health");
        let settled = listed.entries.iter().find(|entry| {
            entry.identity.plugin_id == plugin_id
                && !matches!(
                    entry.status,
                    McpHealthStatus::Unknown {
                        reason: McpHealthUnknownReason::NotProbed
                    }
                )
        });
        if settled.is_some() {
            return listed;
        }
        if Instant::now() >= deadline {
            panic!("health for {plugin_id} did not settle: {listed:?}");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// The default fixture: a stdio MCP with no Settings and an unrunnable command.
const UNRUNNABLE_STDIO_CONFIG: &str = r#"{
    "schemaVersion": 1,
    "transport": {
        "type": "stdio",
        "command": "assets/server"
    }
}"#;

/// A stdio MCP whose argument substitutes the Session workspace directory.
const WORKSPACE_CONTEXT_STDIO_CONFIG: &str = r#"{
    "schemaVersion": 1,
    "transport": {
        "type": "stdio",
        "command": "assets/server",
        "args": [{ "context": "workspace" }]
    }
}"#;

/// A stdio MCP that binds a required Setting into the process environment.
const SECRET_ENV_STDIO_CONFIG: &str = r#"{
    "schemaVersion": 1,
    "settings": {
        "apiKey": {
            "type": "string",
            "title": "API key",
            "description": "Credential passed to the server process",
            "required": true
        }
    },
    "transport": {
        "type": "stdio",
        "command": "assets/server",
        "env": {
            "ORA_FIXTURE_TOKEN": { "setting": "apiKey", "prefix": "Bearer " }
        }
    }
}"#;

/// Installing an unrunnable MCP publishes exactly one closed code on the card.
#[test]
fn installing_an_unrunnable_mcp_publishes_spawn_failed() {
    ora_logging::with_trace_logging(|| {
        let temporary = TempDir::new().expect("temp directory");
        let pool = test_pool(temporary.path());
        let (plugins, hub) = test_plugins(temporary.path(), &pool);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(async {
                // Subscribing needs the runtime: it starts the forwarding task.
                let mut events = hub.subscribe();
                assert_eq!(
                    events.recv().await.expect("ready").expect("event"),
                    AppEvent::Ready
                );
                let plugin_id = import_mcp(
                    &plugins,
                    temporary.path(),
                    "unrunnable-mcp",
                    UNRUNNABLE_STDIO_CONFIG,
                )
                .await;
                let listed = wait_for_card_health(&plugins, &plugin_id).await;
                assert_eq!(
                    listed.entries,
                    vec![ora_contracts::McpHealthEntry {
                        identity: ora_contracts::McpHealthIdentity {
                            plugin_id: plugin_id.clone(),
                            package_version: "0.1.0".to_string(),
                            configuration_revision: 0,
                            transport: ora_contracts::McpHealthTransport::Stdio,
                            cwd: None,
                        },
                        status: McpHealthStatus::Unhealthy {
                            error_code: McpHealthErrorCode::McpSpawnFailed
                        },
                    }]
                );
                // The card is told to re-query; the event carries identity only.
                let mut saw_changed = false;
                for _ in 0..64 {
                    match tokio::time::timeout(Duration::from_millis(200), events.recv()).await {
                        Ok(Some(Ok(event))) => {
                            if event
                                == (AppEvent::McpHealthChanged {
                                    plugin_id: plugin_id.clone(),
                                })
                            {
                                saw_changed = true;
                            }
                        }
                        Ok(Some(Err(_))) | Ok(None) | Err(_) => break,
                    }
                }
                assert!(saw_changed, "health change must reach the card");
            });
    });
}

/// A workspace-context MCP has no card identity and must never be probed against a fake cwd.
#[test]
fn workspace_context_mcp_stays_context_missing_on_the_card() {
    ora_logging::with_trace_logging(|| {
        let temporary = TempDir::new().expect("temp directory");
        let pool = test_pool(temporary.path());
        let (plugins, _hub) = test_plugins(temporary.path(), &pool);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(async {
                let plugin_id = import_mcp(
                    &plugins,
                    temporary.path(),
                    "workspace-mcp",
                    WORKSPACE_CONTEXT_STDIO_CONFIG,
                )
                .await;
                // Give any erroneous probe a chance to run before asserting it never did.
                tokio::time::sleep(Duration::from_millis(100)).await;
                let listed = plugins
                    .list_mcp_health(ListMcpHealthRequest { cwd: None })
                    .expect("list mcp health");
                assert_eq!(
                    listed.entries[0].status,
                    McpHealthStatus::Unknown {
                        reason: McpHealthUnknownReason::ContextMissing
                    }
                );
                // Re-detect from the card must not invent a cwd either.
                let probed = plugins
                    .probe_mcp_health(ProbeMcpHealthRequest {
                        plugin_id: plugin_id.clone(),
                        cwd: None,
                    })
                    .await
                    .expect("card re-detect");
                assert_eq!(
                    probed.entry.status,
                    McpHealthStatus::Unknown {
                        reason: McpHealthUnknownReason::ContextMissing
                    }
                );
            });
    });
}

/// Importing a workspace-context MCP announces its identity so the card re-queries and renders the
/// `Unknown(context_missing)` row even though no probe result is ever stored for it.
#[test]
fn importing_a_workspace_context_mcp_announces_a_card_health_change() {
    ora_logging::with_trace_logging(|| {
        let temporary = TempDir::new().expect("temp directory");
        let pool = test_pool(temporary.path());
        let (plugins, hub) = test_plugins(temporary.path(), &pool);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(async {
                let mut events = hub.subscribe();
                assert_eq!(
                    events.recv().await.expect("ready").expect("event"),
                    AppEvent::Ready
                );
                let plugin_id = import_mcp(
                    &plugins,
                    temporary.path(),
                    "workspace-mcp-event",
                    WORKSPACE_CONTEXT_STDIO_CONFIG,
                )
                .await;
                // The card view is a separate query: without this event its cached list would stay
                // stale and the context-missing row would never appear.
                let mut saw_changed = false;
                for _ in 0..64 {
                    match tokio::time::timeout(Duration::from_millis(200), events.recv()).await {
                        Ok(Some(Ok(event))) => {
                            if event
                                == (AppEvent::McpHealthChanged {
                                    plugin_id: plugin_id.clone(),
                                })
                            {
                                saw_changed = true;
                            }
                        }
                        Ok(Some(Err(_))) | Ok(None) | Err(_) => break,
                    }
                }
                assert!(
                    saw_changed,
                    "becoming eligible must announce a card health change"
                );
            });
    });
}

/// Required Settings keep a member out of probing until a complete save qualifies it.
#[test]
fn incomplete_settings_are_not_probed_until_a_complete_save() {
    ora_logging::with_trace_logging(|| {
        let temporary = TempDir::new().expect("temp directory");
        let pool = test_pool(temporary.path());
        let (plugins, _hub) = test_plugins(temporary.path(), &pool);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(async {
                let plugin_id = import_mcp(
                    &plugins,
                    temporary.path(),
                    "configured-mcp",
                    SECRET_ENV_STDIO_CONFIG,
                )
                .await;

                // Incomplete: no probe is started and the card shows no health row at all.
                tokio::time::sleep(Duration::from_millis(100)).await;
                let before = plugins
                    .list_mcp_health(ListMcpHealthRequest { cwd: None })
                    .expect("list before saving");
                assert!(before.entries.is_empty(), "{before:?}");

                let details = plugins
                    .get_configuration(GetPluginConfigurationRequest {
                        plugin_id: plugin_id.clone(),
                    })
                    .expect("configuration")
                    .configuration;
                let saved = plugins
                    .save_configuration(SavePluginConfigurationRequest {
                        plugin_id: plugin_id.clone(),
                        expected_revision: details.revision,
                        declaration_fingerprint: details.declaration_fingerprint.clone(),
                        values: std::collections::BTreeMap::from([(
                            "apiKey".to_string(),
                            PluginSettingValue::String("super-secret-key".into()),
                        )]),
                        preserve_setting_ids: Vec::new(),
                    })
                    .expect("save configuration")
                    .configuration;
                assert_eq!(
                    saved.summary,
                    ora_contracts::PluginConfigurationSummary::Available {
                        completeness: PluginConfigurationCompleteness::Complete
                    }
                );
                assert_ne!(saved.revision, details.revision);

                // The save itself probed once and only once, on the new revision.
                let listed = wait_for_card_health(&plugins, &plugin_id).await;
                assert_eq!(listed.entries.len(), 1);
                assert_eq!(
                    listed.entries[0].identity.configuration_revision,
                    saved.revision
                );
                assert_eq!(
                    listed.entries[0].status,
                    McpHealthStatus::Unhealthy {
                        error_code: McpHealthErrorCode::McpSpawnFailed
                    }
                );
            });
    });
}

/// Uninstalling a plugin removes its health rows everywhere.
#[test]
fn uninstall_removes_the_health_row() {
    ora_logging::with_trace_logging(|| {
        let temporary = TempDir::new().expect("temp directory");
        let pool = test_pool(temporary.path());
        let (plugins, _hub) = test_plugins(temporary.path(), &pool);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(async {
                let plugin_id = import_mcp(
                    &plugins,
                    temporary.path(),
                    "unrunnable-mcp",
                    UNRUNNABLE_STDIO_CONFIG,
                )
                .await;
                wait_for_card_health(&plugins, &plugin_id).await;

                plugins
                    .uninstall(UninstallPluginRequest {
                        plugin_id: plugin_id.clone(),
                        data_disposition: ora_contracts::PluginDataDisposition::Delete,
                    })
                    .await
                    .expect("uninstall");

                let listed = plugins
                    .list_mcp_health(ListMcpHealthRequest { cwd: None })
                    .expect("list after uninstall");
                assert!(listed.entries.is_empty(), "{listed:?}");
            });
    });
}

/// No health surface may carry a Setting value, credential, or process environment.
#[test]
fn health_surfaces_never_expose_setting_values() {
    let temporary = TempDir::new().expect("temp directory");
    let pool = test_pool(temporary.path());
    let (plugins, _hub) = test_plugins(temporary.path(), &pool);
    let recorder = EventTextRecorder::default();
    ora_logging::with_recorded_trace_logging(recorder.layer(), || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(async {
                let plugin_id = import_mcp(
                    &plugins,
                    temporary.path(),
                    "secret-mcp",
                    SECRET_ENV_STDIO_CONFIG,
                )
                .await;
                let details = plugins
                    .get_configuration(GetPluginConfigurationRequest {
                        plugin_id: plugin_id.clone(),
                    })
                    .expect("configuration")
                    .configuration;
                plugins
                    .save_configuration(SavePluginConfigurationRequest {
                        plugin_id: plugin_id.clone(),
                        expected_revision: details.revision,
                        declaration_fingerprint: details.declaration_fingerprint,
                        values: std::collections::BTreeMap::from([(
                            "apiKey".to_string(),
                            PluginSettingValue::String("super-secret-key".into()),
                        )]),
                        preserve_setting_ids: Vec::new(),
                    })
                    .expect("save configuration");
                let listed = wait_for_card_health(&plugins, &plugin_id).await;
                let probed = plugins
                    .probe_mcp_health(ProbeMcpHealthRequest {
                        plugin_id: plugin_id.clone(),
                        cwd: None,
                    })
                    .await
                    .expect("re-detect");

                // The installed-plugin listing, the query, and the probe response are all
                // serialized exactly as the frontend would receive them.
                let installed = serde_json::to_string(
                    &plugins
                        .list_installed(ora_contracts::ListInstalledPluginsRequest {})
                        .expect("list installed"),
                )
                .expect("serialize installed");
                let listed_json = serde_json::to_string(&listed).expect("serialize list");
                let probed_json = serde_json::to_string(&probed).expect("serialize probe");
                for surface in [&installed, &listed_json, &probed_json] {
                    assert!(!surface.contains("super-secret-key"), "{surface}");
                    assert!(!surface.contains("ORA_FIXTURE_TOKEN"), "{surface}");
                    assert!(!surface.contains("Bearer"), "{surface}");
                }
                assert_eq!(listed.entries.len(), 1);
                assert_eq!(
                    listed.entries[0].status,
                    McpHealthStatus::Unhealthy {
                        error_code: McpHealthErrorCode::McpSpawnFailed
                    }
                );
            });
    });

    let recorded = recorder.text();
    assert!(!recorded.contains("super-secret-key"), "{recorded}");
    assert!(!recorded.contains("ORA_FIXTURE_TOKEN"), "{recorded}");
    assert!(!recorded.contains("Bearer"), "{recorded}");
}

/// A failed Host probe never changes what Session setup resolves and delivers.
#[test]
fn probe_failure_does_not_change_session_delivery() {
    ora_logging::with_trace_logging(|| {
        let temporary = TempDir::new().expect("temp directory");
        let pool = test_pool(temporary.path());
        let (plugins, _hub, plugin_host) = test_plugins_with_host(temporary.path(), &pool);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(async {
                let plugin_id = import_mcp(
                    &plugins,
                    temporary.path(),
                    "unrunnable-mcp",
                    UNRUNNABLE_STDIO_CONFIG,
                )
                .await;
                let listed = wait_for_card_health(&plugins, &plugin_id).await;
                assert_eq!(
                    listed.entries[0].status,
                    McpHealthStatus::Unhealthy {
                        error_code: McpHealthErrorCode::McpSpawnFailed
                    }
                );

                // Session setup still resolves the complete set: the probe result never gates
                // delivery and never produces a partial list.
                let session_host =
                    crate::session_setup::SessionMcpHost::from_plugin_api(plugin_host);
                let setup = crate::session_setup::SessionSetup::resolve(
                    &session_host,
                    temporary.path(),
                    crate::session_setup::AgentSessionMcpCapabilities::new(
                        /*load_session*/ true, /*http*/ true,
                    ),
                )
                .expect("session setup resolves despite the failed probe");
                assert_eq!(setup.mcp.servers().len(), 1);
                assert_eq!(setup.mcp.revision().members().len(), 1);
                assert_eq!(
                    setup.mcp.revision().members()[0].plugin_id.canonical(),
                    plugin_id
                );
            });
    });
}

/// Installing a member whose probe is still running answers immediately.
#[test]
fn importing_a_slow_http_member_does_not_wait_for_the_probe() {
    ora_logging::with_trace_logging(|| {
        let temporary = TempDir::new().expect("temp directory");
        let pool = test_pool(temporary.path());
        let (plugins, _hub) = test_plugins(temporary.path(), &pool);
        // The peer accepts and never answers, so the triggered probe can only end in a timeout.
        let peer = RawTcpPeer::start(PeerBehavior::Hold);
        let config = serde_json::json!({
            "schemaVersion": 1,
            "transport": { "type": "http", "url": peer.url() }
        })
        .to_string();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(async {
                let started = Instant::now();
                let plugin_id = import_mcp(&plugins, temporary.path(), "slow-mcp", &config).await;
                assert!(
                    started.elapsed() < Duration::from_secs(2),
                    "the import waited for the probe: {:?}",
                    started.elapsed()
                );
                // The install response arrived while the probe was still in flight, so the card
                // reads a pending identity rather than a completed result.
                let listed = plugins
                    .list_mcp_health(ListMcpHealthRequest { cwd: None })
                    .expect("list right after installing");
                assert_eq!(listed.entries[0].identity.plugin_id, plugin_id);
                assert_eq!(
                    listed.entries[0].status,
                    McpHealthStatus::Unknown {
                        reason: McpHealthUnknownReason::NotProbed
                    }
                );
            });
    });
}

/// An HTTP credential failure reaches the card as one closed code and leaks nothing.
#[test]
fn http_credential_failure_reaches_the_card_without_leaking_the_key() {
    let temporary = TempDir::new().expect("temp directory");
    let pool = test_pool(temporary.path());
    let (plugins, _hub) = test_plugins(temporary.path(), &pool);
    // A closed loopback port makes the TLS handshake fail immediately, so the probe ends in the
    // HTTP-unreachable classification without waiting for a timeout.
    let peer = RawTcpPeer::start(PeerBehavior::Close);
    let config = serde_json::json!({
        "schemaVersion": 1,
        "settings": {
            "apiKey": {
                "type": "string",
                "title": "API key",
                "description": "Credential sent to the remote server",
                "required": true
            }
        },
        "transport": {
            "type": "http",
            "url": peer.url(),
            "headers": { "Authorization": { "setting": "apiKey", "prefix": "Bearer " } }
        }
    })
    .to_string();
    let recorder = EventTextRecorder::default();
    ora_logging::with_recorded_trace_logging(recorder.layer(), || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(async {
                let plugin_id =
                    import_mcp(&plugins, temporary.path(), "credential-mcp", &config).await;
                // Incomplete configuration is never probed and shows no health row.
                assert!(
                    plugins
                        .list_mcp_health(ListMcpHealthRequest { cwd: None })
                        .expect("list before saving")
                        .entries
                        .is_empty()
                );
                let details = plugins
                    .get_configuration(GetPluginConfigurationRequest {
                        plugin_id: plugin_id.clone(),
                    })
                    .expect("configuration")
                    .configuration;
                plugins
                    .save_configuration(SavePluginConfigurationRequest {
                        plugin_id: plugin_id.clone(),
                        expected_revision: details.revision,
                        declaration_fingerprint: details.declaration_fingerprint,
                        values: std::collections::BTreeMap::from([(
                            "apiKey".to_string(),
                            PluginSettingValue::String("super-secret-key".into()),
                        )]),
                        preserve_setting_ids: Vec::new(),
                    })
                    .expect("save configuration");

                let listed = wait_for_card_health(&plugins, &plugin_id).await;
                assert_eq!(
                    listed.entries[0].status,
                    McpHealthStatus::Unhealthy {
                        error_code: McpHealthErrorCode::McpHttpUnreachable
                    }
                );
                let probed = plugins
                    .probe_mcp_health(ProbeMcpHealthRequest {
                        plugin_id: plugin_id.clone(),
                        cwd: None,
                    })
                    .await
                    .expect("re-detect");

                let installed = serde_json::to_string(
                    &plugins
                        .list_installed(ora_contracts::ListInstalledPluginsRequest {})
                        .expect("list installed"),
                )
                .expect("serialize installed");
                let listed_json = serde_json::to_string(&listed).expect("serialize list");
                let probed_json = serde_json::to_string(&probed).expect("serialize probe");
                for surface in [&installed, &listed_json, &probed_json] {
                    assert!(!surface.contains("super-secret-key"), "{surface}");
                    assert!(!surface.contains("Bearer"), "{surface}");
                    assert!(!surface.contains("Authorization"), "{surface}");
                }
            });
    });

    let recorded = recorder.text();
    assert!(!recorded.contains("super-secret-key"), "{recorded}");
    assert!(!recorded.contains("Bearer"), "{recorded}");
}

/// Captures the rendered fields of every event emitted into the scoped TRACE subscriber.
#[derive(Clone, Debug, Default)]
struct EventTextRecorder {
    text: Arc<std::sync::Mutex<String>>,
}

impl EventTextRecorder {
    fn layer(&self) -> EventTextLayer {
        EventTextLayer {
            text: self.text.clone(),
        }
    }

    fn text(&self) -> String {
        self.text.lock().expect("recorded event lock").clone()
    }
}

#[derive(Clone, Debug)]
struct EventTextLayer {
    text: Arc<std::sync::Mutex<String>>,
}

impl<S> tracing_subscriber::layer::Layer<S> for EventTextLayer
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _context: tracing_subscriber::layer::Context<'_, S>,
    ) {
        event.record(&mut EventTextVisitor {
            text: self.text.clone(),
        });
    }
}

/// Renders each field as `name=value` so the test can assert the whole leak boundary.
struct EventTextVisitor {
    text: Arc<std::sync::Mutex<String>>,
}

impl EventTextVisitor {
    fn record(&mut self, field: &tracing::field::Field, value: impl std::fmt::Display) {
        let mut text = self.text.lock().expect("recorded event lock");
        text.push_str(field.name());
        text.push('=');
        text.push_str(&value.to_string());
        text.push('\n');
    }
}

impl tracing::field::Visit for EventTextVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.record(field, value);
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.record(field, format!("{value:?}"));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.record(field, value);
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.record(field, value);
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.record(field, value);
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.record(field, value);
    }
}

/// A stdio MCP that respawns the test binary as a well-behaved MCP server (`ok` mode).
const HEALTHY_STDIO_CONFIG: &str = r#"{
    "schemaVersion": 1,
    "transport": {
        "type": "stdio",
        "command": "assets/fake-server-ok.exe",
        "args": ["--exact", "plugin::operations::health_tests::fake_mcp_stdio_server", "--nocapture"]
    }
}"#;

/// A runnable MCP server that records every start, gated by a required Setting.
///
/// The required Setting keeps the member ineligible until a complete save, so a test can count how
/// many times that save actually started the process.
const COUNTING_STDIO_CONFIG: &str = r#"{
    "schemaVersion": 1,
    "settings": {
        "apiKey": {
            "type": "string",
            "title": "API key",
            "description": "Credential gating the member",
            "required": true
        }
    },
    "transport": {
        "type": "stdio",
        "command": "assets/fake-server-count.exe",
        "args": ["--exact", "plugin::operations::health_tests::fake_mcp_stdio_server", "--nocapture"]
    }
}"#;

/// Respawned as a child under the `fake-server-<mode>.exe` name, acts as a well-behaved MCP server.
///
/// The mode comes from this process's own file name so a plugin package needs no environment
/// plumbing; under the test binary's normal name the function is a no-op pass. The `count` mode
/// additionally appends one line to `starts.log` beside the executable, which lets a test prove how
/// many times a trigger actually started the server.
#[test]
fn fake_mcp_stdio_server() {
    use std::io::{BufRead, Write};

    let exe = std::env::current_exe().expect("current exe");
    let stem = exe
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    let Some(mode) = stem.strip_prefix("fake-server-") else {
        return;
    };
    if mode == "count" {
        if let Some(directory) = exe.parent() {
            if let Ok(mut log) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(directory.join("starts.log"))
            {
                let _ = writeln!(log, "start");
            }
        }
    }
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let id = match line.find("\"id\":") {
            Some(index) => {
                let rest = line[index + 5..].trim_start();
                let end = rest
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(rest.len());
                if end == 0 {
                    "1".to_string()
                } else {
                    rest[..end].to_string()
                }
            }
            None => "1".to_string(),
        };
        if line.contains("\"method\":\"initialize\"") {
            let _ = writeln!(
                out,
                "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"result\":{{\"protocolVersion\":\"2025-03-26\",\"capabilities\":{{}}}}}}"
            );
        } else if line.contains("\"method\":\"tools/list\"") {
            let _ = writeln!(
                out,
                "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"result\":{{\"tools\":[]}}}}"
            );
        }
        let _ = out.flush();
    }
}

/// A runnable, well-behaved MCP imported through the product path reports Host `Healthy`.
#[test]
fn healthy_stdio_mcp_imports_as_healthy() {
    ora_logging::with_trace_logging(|| {
        let temporary = TempDir::new().expect("temp directory");
        let pool = test_pool(temporary.path());
        let (plugins, _hub) = test_plugins(temporary.path(), &pool);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(async {
                let archive = temporary.path().join("healthy-mcp.orax");
                let server = std::fs::read(std::env::current_exe().expect("test binary"))
                    .expect("read test binary");
                write_mcp_orax_with_command(
                    &archive,
                    "healthy-mcp",
                    HEALTHY_STDIO_CONFIG,
                    "assets/fake-server-ok.exe",
                    &server,
                );
                let plugin_id = plugins
                    .import(ImportPluginRequest {
                        path: archive.to_string_lossy().into_owned(),
                    })
                    .await
                    .expect("import healthy MCP")
                    .plugin_id;
                let listed = wait_for_card_health(&plugins, &plugin_id).await;
                assert_eq!(
                    listed.entries,
                    vec![ora_contracts::McpHealthEntry {
                        identity: ora_contracts::McpHealthIdentity {
                            plugin_id: plugin_id.clone(),
                            package_version: "0.1.0".to_string(),
                            configuration_revision: 0,
                            transport: ora_contracts::McpHealthTransport::Stdio,
                            cwd: None,
                        },
                        status: McpHealthStatus::Healthy,
                    }]
                );
            });
    });
}

/// Saving a complete configuration starts the server exactly once and never retries on its own.
#[test]
fn save_probes_exactly_once_without_automatic_retry() {
    ora_logging::with_trace_logging(|| {
        let temporary = TempDir::new().expect("temp directory");
        let pool = test_pool(temporary.path());
        let (plugins, _hub) = test_plugins(temporary.path(), &pool);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(async {
                let archive = temporary.path().join("counting-mcp.orax");
                let server = std::fs::read(std::env::current_exe().expect("test binary"))
                    .expect("read test binary");
                write_mcp_orax_with_command(
                    &archive,
                    "counting-mcp",
                    COUNTING_STDIO_CONFIG,
                    "assets/fake-server-count.exe",
                    &server,
                );
                let plugin_id = plugins
                    .import(ImportPluginRequest {
                        path: archive.to_string_lossy().into_owned(),
                    })
                    .await
                    .expect("import counting MCP")
                    .plugin_id;
                let starts_log = temporary
                    .path()
                    .join("plugins/installed/local/counting-mcp/0.1.0/assets/starts.log");

                // Incomplete required Settings never probe, so the server has not started yet.
                tokio::time::sleep(Duration::from_millis(150)).await;
                assert!(
                    !starts_log.exists(),
                    "an ineligible member must not be probed"
                );

                let details = plugins
                    .get_configuration(GetPluginConfigurationRequest {
                        plugin_id: plugin_id.clone(),
                    })
                    .expect("configuration")
                    .configuration;
                plugins
                    .save_configuration(SavePluginConfigurationRequest {
                        plugin_id: plugin_id.clone(),
                        expected_revision: details.revision,
                        declaration_fingerprint: details.declaration_fingerprint,
                        values: std::collections::BTreeMap::from([(
                            "apiKey".to_string(),
                            PluginSettingValue::String("super-secret-key".into()),
                        )]),
                        preserve_setting_ids: Vec::new(),
                    })
                    .expect("save configuration");
                let listed = wait_for_card_health(&plugins, &plugin_id).await;
                assert_eq!(
                    listed.entries[0].status,
                    McpHealthStatus::Healthy,
                    "the counting server is well behaved"
                );

                // Give an automatic retry time to appear before asserting that there is none.
                tokio::time::sleep(Duration::from_millis(500)).await;
                let starts = std::fs::read_to_string(&starts_log).unwrap_or_default();
                assert_eq!(
                    starts.lines().count(),
                    1,
                    "save must probe once and never retry: {starts:?}"
                );
            });
    });
}

/// Reset All makes the member ineligible again, so its Host health row disappears everywhere.
#[test]
fn reset_configuration_drops_the_health_row() {
    ora_logging::with_trace_logging(|| {
        let temporary = TempDir::new().expect("temp directory");
        let pool = test_pool(temporary.path());
        let (plugins, _hub) = test_plugins(temporary.path(), &pool);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(async {
                let plugin_id = import_mcp(
                    &plugins,
                    temporary.path(),
                    "resettable-mcp",
                    SECRET_ENV_STDIO_CONFIG,
                )
                .await;
                let details = plugins
                    .get_configuration(GetPluginConfigurationRequest {
                        plugin_id: plugin_id.clone(),
                    })
                    .expect("configuration")
                    .configuration;
                let saved = plugins
                    .save_configuration(SavePluginConfigurationRequest {
                        plugin_id: plugin_id.clone(),
                        expected_revision: details.revision,
                        declaration_fingerprint: details.declaration_fingerprint.clone(),
                        values: std::collections::BTreeMap::from([(
                            "apiKey".to_string(),
                            PluginSettingValue::String("super-secret-key".into()),
                        )]),
                        preserve_setting_ids: Vec::new(),
                    })
                    .expect("save configuration")
                    .configuration;
                wait_for_card_health(&plugins, &plugin_id).await;

                plugins
                    .reset_configuration(ResetPluginConfigurationRequest {
                        plugin_id: plugin_id.clone(),
                        declaration_fingerprint: details.declaration_fingerprint,
                        reset: ResetPluginConfigurationMode::ResetAll {
                            expected_revision: saved.revision,
                        },
                    })
                    .expect("reset configuration");

                // Clearing the Settings drops the member back to ineligible, so the old identity
                // must not keep presenting a health row.
                let listed = plugins
                    .list_mcp_health(ListMcpHealthRequest { cwd: None })
                    .expect("list after reset");
                assert!(
                    listed
                        .entries
                        .iter()
                        .all(|entry| entry.identity.plugin_id != plugin_id),
                    "reset must remove the health row: {listed:?}"
                );
            });
    });
}

/// A delivery boundary logs the ACP send first, then the paired Host health observation.
#[test]
fn session_health_observation_is_logged_after_the_acp_send_boundary() {
    let temporary = TempDir::new().expect("temp directory");
    let pool = test_pool(temporary.path());
    let (plugins, _hub, plugin_host) = test_plugins_with_host(temporary.path(), &pool);
    let recorder = EventTextRecorder::default();
    ora_logging::with_recorded_trace_logging(recorder.layer(), || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(async {
                import_mcp(
                    &plugins,
                    temporary.path(),
                    "boundary-mcp",
                    UNRUNNABLE_STDIO_CONFIG,
                )
                .await;
                let host =
                    crate::session_setup::SessionMcpHost::from_plugin_api(plugin_host.clone());
                let setup = crate::session_setup::SessionSetup::resolve(
                    &host,
                    temporary.path(),
                    crate::session_setup::AgentSessionMcpCapabilities::new(
                        /*load_session*/ true, /*http*/ true,
                    ),
                )
                .expect("resolve session mcp");
                crate::agent_runtime::record_session_mcp_boundary(
                    &host,
                    &ora_domain::SessionId::new("boundary-session"),
                    &ora_domain::AgentRef::parse("official/ora-space.opencode").expect("agent ref"),
                    Some("provider-session"),
                    "session/new",
                    &setup.mcp,
                    temporary.path(),
                );
                // The observation runs on its own task; let it report before the runtime drops.
                let deadline = Instant::now() + Duration::from_secs(10);
                while Instant::now() < deadline
                    && !recorder.text().contains("host MCP health probe result")
                {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            });
    });

    let recorded = recorder.text();
    let send = recorded
        .find("sending ACP session configuration")
        .unwrap_or_else(|| panic!("missing send log: {recorded}"));
    let probe = recorded
        .find("host MCP health probe result")
        .unwrap_or_else(|| panic!("missing probe result log: {recorded}"));
    assert!(
        send < probe,
        "the Host health observation must follow the ACP send boundary: {recorded}"
    );
    // Both events carry the same session so an operator can pair them.
    assert!(recorded.contains("boundary-session"), "{recorded}");
    // The pairing log stays secret-free: no command path, env, or credential.
    assert!(
        !recorded.contains("#!/nonexistent-ora-probe-interpreter"),
        "{recorded}"
    );
}
