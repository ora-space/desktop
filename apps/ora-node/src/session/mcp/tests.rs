//! Exact package resolution and gating through the production SessionSetup interface.
use super::*;
use agent_client_protocol_schema::v1::{
    EnvVariable, HttpHeader, McpServer, McpServerHttp, McpServerStdio,
};
use ora_node_protocol::{PluginId as WireId, PluginVersion};
use pretty_assertions::assert_eq;
use serde_json::json;
use std::path::PathBuf;

/// Contains no discovery fallback; tests choose the exact paths visible to the session.
struct Catalog(BTreeMap<String, PathBuf>);
impl PluginCatalog for Catalog {
    type Lease = ();
    /// Tests retain immutable directories for the whole call.
    fn lease(&self, _plugin_id: &WireId) {}
    /// Models exact-version lookup rather than latest-version discovery.
    fn installed(&self, id: &WireId, version: &PluginVersion) -> Option<PathBuf> {
        (version.as_str() == "1.0.0")
            .then(|| self.0.get(id.as_str()).cloned())
            .flatten()
    }
    /// MCP tests never apply a model adapter.
    fn check_configuration(
        &self,
        _configuration: &ora_node_protocol::FrozenAgentConfiguration,
    ) -> Result<(), super::super::super::InstalledConfigurationError> {
        Ok(())
    }
}

/// Writes a valid installed MCP package, including a command whose existence is rechecked.
fn package(root: &Path, name: &str, transport: serde_json::Value) -> PathBuf {
    let path = root.join(name);
    std::fs::create_dir_all(path.join("assets")).unwrap();
    std::fs::write(path.join("orax.toml"), format!("resolver = 1\nidentifier = \"{name}\"\nkind = \"mcp\"\nversion = \"1.0.0\"\ndescription = \"test\"\n")).unwrap();
    std::fs::write(path.join("assets").join("server"), "#!/bin/sh\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        path.join("assets").join("server"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    std::fs::write(path.join("assets").join("config.json"), serde_json::to_vec(&json!({
        "schemaVersion":1, "settings":{
            "token":{"type":"string", "title":"Token", "description":"test", "required":true},
            "argument":{"type":"string", "title":"Argument", "description":"test", "default":"default-arg"}
        }, "transport": transport
    })).unwrap()).unwrap();
    ora_plugin_manager::inspect_planned_package(
        &path,
        &PluginId::parse(&format!("official/{name}")).unwrap(),
        &"1.0.0".parse().unwrap(),
    )
    .unwrap();
    path
}

/// Freezes both the environment and argument input without any host configuration files.
fn binding(name: &str) -> SessionMcpBinding {
    SessionMcpBinding {
        plugin_id: WireId::new(format!("official/{name}")),
        plugin_version: PluginVersion::new("1.0.0"),
        settings: BTreeMap::from([
            (
                "token".into(),
                SettingValue::String("m938-test-secret".into()),
            ),
            (
                "argument".into(),
                SettingValue::String("argument-secret".into()),
            ),
        ]),
    }
}

/// Builds both transport declarations using the actual plugin-config schema.
fn transports() -> [serde_json::Value; 2] {
    [
        json!({"type":"stdio", "command":"assets/server", "args":[{"context":"workspace"}, {"setting":"argument"}], "env":{"TOKEN":{"setting":"token"}}}),
        json!({"type":"http", "url":"https://mcp.example.test/mcp", "headers":{"Authorization":{"setting":"token", "prefix":"Bearer "}}}),
    ]
}

/// Sorting, substitutions and revision identity are shared with Desktop rather than reimplemented.
#[test]
fn resolves_exact_packages_and_secret_free_revision() {
    ora_logging::with_trace_logging(|| {
        let root = tempfile::tempdir().unwrap();
        let [stdio, http] = transports();
        let stdio = package(root.path(), "acme.stdio", stdio);
        let http = package(root.path(), "acme.web", http);
        let catalog = Catalog(BTreeMap::from([
            ("official/acme.stdio".into(), stdio.clone()),
            ("official/acme.web".into(), http),
        ]));
        let bindings = vec![binding("acme.stdio"), binding("acme.web")];
        let mut defaulted = bindings.clone();
        defaulted[0].settings.remove("argument");
        let (setup, _leases) = NodeSessionMcp::prepare(&catalog, defaulted, root.path()).unwrap();
        let capabilities =
            AgentSessionMcpCapabilities::new(/*load_session*/ true, /*http*/ true);
        let snapshot = setup.resolve_mcp(root.path(), capabilities).unwrap();
        assert_eq!(
            snapshot.servers(),
            &[
                McpServer::Stdio(
                    McpServerStdio::new(
                        "official/acme.stdio",
                        stdio.join("assets").join("server").canonicalize().unwrap()
                    )
                    .args(vec![
                        root.path().to_string_lossy().into_owned(),
                        "default-arg".into()
                    ])
                    .env(vec![EnvVariable::new("TOKEN", "m938-test-secret")])
                ),
                McpServer::Http(
                    McpServerHttp::new("official/acme.web", "https://mcp.example.test/mcp")
                        .headers(vec![HttpHeader::new(
                            "Authorization",
                            "Bearer m938-test-secret"
                        )])
                )
            ]
        );
        let mut changed = bindings;
        changed[0].settings.insert(
            "token".into(),
            SettingValue::String("changed-secret".into()),
        );
        let (changed, _) = NodeSessionMcp::prepare(&catalog, changed, root.path()).unwrap();
        assert_eq!(
            snapshot.revision(),
            changed
                .resolve_mcp(root.path(), capabilities)
                .unwrap()
                .revision()
        );
        assert!(!format!("{snapshot:?}").contains("m938-test-secret"));
        let (empty, _) =
            NodeSessionMcp::prepare(&Catalog(BTreeMap::new()), Vec::new(), root.path()).unwrap();
        assert_eq!(
            empty
                .resolve_mcp(
                    root.path(),
                    AgentSessionMcpCapabilities::new(
                        /*load_session*/ false, /*http*/ false
                    )
                )
                .unwrap()
                .servers(),
            &[]
        );
        // A runtime view cannot widen the spec to Automatic discovery.
        assert_eq!(
            setup
                .with_selection(SessionMcpSelection::Automatic)
                .selection(),
            setup.selection()
        );
    });
}

/// Capability rejection is all-or-nothing even when an earlier stdio member could resolve.
#[test]
fn rejects_missing_capabilities_and_missing_or_invalid_packages_and_settings() {
    ora_logging::with_trace_logging(|| {
        let root = tempfile::tempdir().unwrap();
        let [stdio, http] = transports();
        let stdio = package(root.path(), "acme.stdio", stdio);
        let http = package(root.path(), "acme.web", http);
        let catalog = Catalog(BTreeMap::from([
            ("official/acme.stdio".into(), stdio.clone()),
            ("official/acme.web".into(), http),
        ]));
        let bindings = vec![binding("acme.stdio"), binding("acme.web")];
        let (setup, _) = NodeSessionMcp::prepare(&catalog, bindings.clone(), root.path()).unwrap();
        for (load, http, code, public_error) in [
            (
                false,
                true,
                "mcp_load_session_missing",
                PublicError::SessionMcpSetupFailed(Box::new(SessionMcpSetupFailedParams {
                    error_code: "mcp_load_session_missing".into(),
                    plugin_id: None,
                    setting_id: None,
                    transport: None,
                })),
            ),
            (
                true,
                false,
                "mcp_http_capability_missing",
                PublicError::SessionMcpSetupFailed(Box::new(SessionMcpSetupFailedParams {
                    error_code: "mcp_http_capability_missing".into(),
                    plugin_id: Some("official/acme.web".into()),
                    setting_id: None,
                    transport: Some("http".into()),
                })),
            ),
        ] {
            let error = setup
                .resolve_mcp(root.path(), AgentSessionMcpCapabilities::new(load, http))
                .unwrap_err();
            assert_eq!(
                (
                    start_failure(&error, &ExecutionId::new("test")).detail,
                    error.public_error()
                ),
                (Some(code), &public_error)
            );
            assert!(!format!("{error:?}").contains("m938-test-secret"));
        }
        let mut missing = binding("acme.stdio");
        missing.settings.remove("token");
        let failure = NodeSessionMcp::prepare(&catalog, vec![missing], root.path())
            .err()
            .unwrap();
        assert_eq!(
            failure,
            NodeMcpFailure {
                code: NodeMcpError::Setting,
                plugin_id: Some("official/acme.stdio".into()),
                setting_id: Some("token".into()),
                transport: Some(SessionMcpTransportKind::Stdio),
            }
        );
        assert_eq!(
            failure.public_error(),
            PublicError::SessionMcpSetupFailed(Box::new(SessionMcpSetupFailedParams {
                error_code: "mcp_setting_missing".into(),
                plugin_id: Some("official/acme.stdio".into()),
                setting_id: Some("token".into()),
                transport: Some("stdio".into()),
            }))
        );
        let mut invalid = binding("acme.stdio");
        invalid
            .settings
            .insert("token".into(), SettingValue::String("secret\nvalue".into()));
        assert_eq!(
            NodeSessionMcp::prepare(&catalog, vec![invalid], root.path())
                .err()
                .map(|failure| failure.code),
            Some(NodeMcpError::Setting)
        );
        let mut wrong = binding("acme.stdio");
        wrong.plugin_version = PluginVersion::new("2.0.0");
        assert_eq!(
            NodeSessionMcp::prepare(&catalog, vec![wrong], root.path())
                .err()
                .map(|failure| failure.code),
            Some(NodeMcpError::Plugin)
        );
        assert_eq!(
            NodeSessionMcp::prepare(&catalog, vec![binding("acme.missing")], root.path())
                .err()
                .map(|failure| failure.code),
            Some(NodeMcpError::Plugin)
        );
        std::fs::remove_file(stdio.join("assets").join("server")).unwrap();
        assert_eq!(
            NodeSessionMcp::prepare(&catalog, bindings, root.path())
                .err()
                .map(|failure| failure.code),
            Some(NodeMcpError::Plugin)
        );
    });
}

/// An optional declaration still fails when its transport reference has no effective value.
#[test]
fn maps_optional_referenced_setting_missing_to_full_failure() {
    ora_logging::with_trace_logging(|| {
        let root = tempfile::tempdir().unwrap();
        let path = package(
            root.path(),
            "acme.stdio",
            json!({"type":"stdio", "command":"assets/server", "args":[{"setting":"argument"}]}),
        );
        let config_path = path.join("assets").join("config.json");
        let mut config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&config_path).unwrap()).unwrap();
        config["settings"] = json!({
            "region": {"type":"string", "title":"Region", "description":"test"}
        });
        config["transport"]["args"] = json!([{"setting":"region"}]);
        std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
        let catalog = Catalog(BTreeMap::from([("official/acme.stdio".into(), path)]));
        let mut selected = binding("acme.stdio");
        selected.settings.clear();

        let failure = NodeSessionMcp::prepare(&catalog, vec![selected], root.path())
            .err()
            .unwrap();

        assert_eq!(
            failure,
            NodeMcpFailure {
                code: NodeMcpError::Setting,
                plugin_id: Some("official/acme.stdio".into()),
                setting_id: Some("region".into()),
                transport: Some(SessionMcpTransportKind::Stdio),
            }
        );
    });
}

/// A settings-less declaration cannot silently discard an explicit override.
#[test]
fn rejects_an_override_for_a_settings_less_package() {
    ora_logging::with_trace_logging(|| {
        let root = tempfile::tempdir().unwrap();
        let path = package(
            root.path(),
            "acme.stdio",
            json!({"type":"stdio", "command":"assets/server", "args":[]}),
        );
        let config_path = path.join("assets").join("config.json");
        let mut config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&config_path).unwrap()).unwrap();
        config.as_object_mut().unwrap().remove("settings");
        std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
        let catalog = Catalog(BTreeMap::from([("official/acme.stdio".into(), path)]));
        let mut selected = binding("acme.stdio");
        selected
            .settings
            .retain(|setting_id, _| setting_id == "token");

        let failure = NodeSessionMcp::prepare(&catalog, vec![selected], root.path())
            .err()
            .unwrap();

        assert_eq!(
            failure,
            NodeMcpFailure {
                code: NodeMcpError::Setting,
                plugin_id: Some("official/acme.stdio".into()),
                setting_id: Some("token".into()),
                transport: Some(SessionMcpTransportKind::Stdio),
            }
        );
        assert!(!format!("{failure:?}").contains("m938-test-secret"));
    });
}
