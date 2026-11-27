//! Covers the Desktop translations the shared runtime depends on.
//!
//! The runtime is tested against in-memory hosts, so what only Desktop does — reading installed
//! packages through the plugin API and publishing lifecycle notifications as application events —
//! is pinned here, where a wrong translation would otherwise pass every runtime test.

use crate::app_event::AppEventHub;
use crate::clock::SystemClock;
use crate::plugin::PluginApi;
use crate::settings::Settings;
use ora_agent_runtime::{AgentAttach, RuntimeEvents};
use ora_contracts::{AppEvent, ScanPluginsRequest};
use ora_db::{DatabaseBootstrapper, DatabaseLocation, default_migration_catalog};
use ora_domain::{AgentRef, PluginId, SessionId};
use pretty_assertions::assert_eq;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;

/// Writes one minimal agent package into the plugin root a lifecycle discovers.
fn write_plugin_package(data_directory: &Path, package_name: &str) {
    let package_root = data_directory
        .join("plugins")
        .join("installed")
        .join("official")
        .join(package_name)
        .join("1.0.0");
    fs::create_dir_all(&package_root).expect("create plugin package");
    fs::write(package_root.join("main.js"), "export {};\n").expect("write plugin entrypoint");
    fs::write(
        package_root.join("orax.toml"),
        format!(
            "resolver = 1\nidentifier = {package_name:?}\nnamespace = \"official\"\nkind = \"agent\"\nversion = \"1.0.0\"\ndescription = \"Example\"\n"
        ),
    )
    .expect("write plugin manifest");
}

/// Opens the plugin API over `root` with its own migrated database.
fn open_plugin_api(root: &Path) -> PluginApi {
    let pool = DatabaseBootstrapper::new(crate::test_clock::TestClock)
        .bootstrap_repository_pool(
            &DatabaseLocation::path(root.join("ora.sqlite3")),
            &default_migration_catalog().expect("build migration catalog"),
        )
        .expect("create repository pool");
    PluginApi::open(
        pool.clone(),
        root.to_path_buf(),
        PathBuf::from("deno"),
        SystemClock,
        AppEventHub::new().publisher(),
        Arc::new(Settings::new(pool)),
        Default::default(),
    )
    .expect("open plugin host")
}

/// Verifies a package scanned after the host opened becomes an agent the runtime can supervise.
///
/// The supervised set is decided by which packages are installed, so a package that appears while
/// Ora runs must be reported without reopening the host.
#[tokio::test]
async fn reports_scanned_agent_packages_as_installed_agents() {
    let temporary = TempDir::new().expect("create plugin test directory");
    let plugin_api = open_plugin_api(temporary.path());
    assert_eq!(plugin_api.installed_agent_plugins(), Vec::new());

    write_plugin_package(temporary.path(), "example");
    plugin_api
        .scan(ScanPluginsRequest {})
        .await
        .expect("scan plugins");

    let agent = PluginId::new("official", "example").expect("plugin id");
    let absent = PluginId::new("official", "absent").expect("plugin id");
    assert_eq!(plugin_api.installed_agent_plugins(), vec![agent.clone()]);
    assert_eq!(
        [agent, absent].map(|plugin_id| plugin_api.is_installed(&plugin_id)),
        [true, false],
    );
}

/// Verifies runtime notifications reach clients as the application events they re-query on.
///
/// An agent is identified by its whole canonical plugin id, so the invalidation carries that
/// string: it is what a client keys its model discovery query by.
#[tokio::test]
async fn publishes_runtime_notifications_as_application_events() {
    let hub = AppEventHub::new();
    let mut events = hub.subscribe();
    let publisher = hub.publisher();

    publisher.session_title_updated(&SessionId::new("session-1"));
    publisher.session_options_changed(&SessionId::new("session-1"));
    publisher.agent_models_invalidated(
        &AgentRef::parse("official/ora-space.opencode").expect("agent identity"),
    );

    let mut received = Vec::new();
    for _ in 0..4 {
        received.push(events.recv().await.expect("stream is open").expect("event"));
    }
    assert_eq!(
        received,
        vec![
            AppEvent::Ready,
            AppEvent::SessionTitleUpdated {
                session_id: "session-1".to_string(),
            },
            AppEvent::SessionOptionsChanged {
                session_id: "session-1".to_string(),
            },
            AppEvent::AgentModelsInvalidated {
                agent_ref: "official/ora-space.opencode".to_string(),
            },
        ],
    );
}
