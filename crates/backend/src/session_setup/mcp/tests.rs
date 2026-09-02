use super::{
    AgentSessionMcpCapabilities, InstalledMcpCandidate, LiveMcpEvent, LiveMcpPromptAdmission,
    LiveMcpState, McpConfigurationEligibility, SessionMcpCatalog, SessionMcpConfigurationSource,
    SessionMcpError, SessionMcpMemberRevision, SessionMcpRevision, SessionMcpTransportKind,
    resolve_session_mcp, resolve_session_mcp_revision,
};
use agent_client_protocol_schema::v1::McpServer;
use ora_domain::PluginId;
use ora_plugin_config::{
    CompiledMcpConfiguration, McpArgument, McpHttpTransport, McpStdioTransport, McpTransport,
    McpValueExpression, SettingValue,
};
use ora_utils::path::PortableRelativePath;
use pretty_assertions::assert_eq;
use semver::Version;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use url::Url;

struct Fixture {
    _dir: TempDir,
    package_root: PathBuf,
    command: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = TempDir::new().expect("tempdir");
        let package_root = dir.path().join("pkg");
        let command = package_root.join("assets").join("server");
        std::fs::create_dir_all(command.parent().expect("parent")).expect("assets");
        std::fs::write(&command, b"#!/bin/sh\n").expect("command");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o755))
                .expect("executable");
        }
        Self {
            _dir: dir,
            package_root,
            command,
        }
    }
}

#[derive(Clone)]
struct FakeCatalog {
    inner: Arc<Mutex<Vec<Vec<InstalledMcpCandidate>>>>,
}

impl FakeCatalog {
    fn new(candidates: Vec<InstalledMcpCandidate>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(vec![candidates])),
        }
    }

    fn then(&self, candidates: Vec<InstalledMcpCandidate>) {
        self.inner.lock().expect("catalog").push(candidates);
    }
}

impl SessionMcpCatalog for FakeCatalog {
    fn installed_mcps(&self) -> Result<Vec<InstalledMcpCandidate>, SessionMcpError> {
        let mut pages = self.inner.lock().expect("catalog");
        if pages.len() > 1 {
            Ok(pages.remove(0))
        } else {
            Ok(pages.first().cloned().unwrap_or_default())
        }
    }
}

struct FakeConfigurations {
    by_id: BTreeMap<String, McpConfigurationEligibility>,
}

impl SessionMcpConfigurationSource for FakeConfigurations {
    fn eligibility(
        &self,
        candidate: &InstalledMcpCandidate,
    ) -> Result<McpConfigurationEligibility, SessionMcpError> {
        Ok(self
            .by_id
            .get(&candidate.plugin_id.canonical())
            .cloned()
            .unwrap_or(McpConfigurationEligibility::Unavailable))
    }
}

fn plugin(name: &str) -> PluginId {
    PluginId::new("ora-space", name).expect("plugin id")
}

fn stdio_config() -> CompiledMcpConfiguration {
    CompiledMcpConfiguration {
        schema_version: 1,
        settings: None,
        transport: McpTransport::Stdio(McpStdioTransport {
            command: PortableRelativePath::parse("assets/server").expect("command"),
            args: vec![
                McpArgument::Value(McpValueExpression::Literal(".".to_string())),
                McpArgument::WorkspaceContext,
            ],
            env: BTreeMap::new(),
        }),
    }
}

fn http_config() -> CompiledMcpConfiguration {
    CompiledMcpConfiguration {
        schema_version: 1,
        settings: None,
        transport: McpTransport::Http(McpHttpTransport {
            url: Url::parse("https://mcp.example.test/mcp").expect("url"),
            headers: BTreeMap::from([(
                "Authorization".to_string(),
                McpValueExpression::Setting {
                    id: "apiKey".to_string(),
                    prefix: "Bearer ".to_string(),
                    suffix: String::new(),
                },
            )]),
        }),
    }
}

fn candidate(
    name: &str,
    version: Version,
    package_root: &Path,
    configuration: CompiledMcpConfiguration,
) -> InstalledMcpCandidate {
    InstalledMcpCandidate {
        plugin_id: plugin(name),
        version,
        package_root: package_root.to_path_buf(),
        configuration,
    }
}

fn capabilities(load_session: bool, http: bool) -> AgentSessionMcpCapabilities {
    AgentSessionMcpCapabilities::new(load_session, http)
}

fn revision(
    name: &str,
    version: Version,
    configuration_revision: u64,
    transport: SessionMcpTransportKind,
) -> SessionMcpMemberRevision {
    SessionMcpMemberRevision {
        plugin_id: plugin(name),
        package_version: version,
        configuration_revision,
        transport,
    }
}

#[test]
fn maps_stdio_and_http_in_canonical_plugin_id_order_with_exact_revisions() {
    let fixture = Fixture::new();
    let cwd = fixture.package_root.clone();
    let catalog = FakeCatalog::new(vec![
        candidate(
            "zeta-search",
            Version::new(1, 2, 0),
            &fixture.package_root,
            stdio_config(),
        ),
        candidate(
            "alpha-search",
            Version::new(2, 0, 1),
            &fixture.package_root,
            http_config(),
        ),
    ]);
    let configurations = FakeConfigurations {
        by_id: BTreeMap::from([
            (
                plugin("alpha-search").canonical(),
                McpConfigurationEligibility::Complete {
                    revision: 7,
                    values: BTreeMap::from([(
                        "apiKey".to_string(),
                        SettingValue::String("super-secret".into()),
                    )]),
                },
            ),
            (
                plugin("zeta-search").canonical(),
                McpConfigurationEligibility::NoSettings,
            ),
        ]),
    };

    let snapshot = resolve_session_mcp(
        &catalog,
        &configurations,
        &cwd,
        capabilities(/*load_session*/ true, /*http*/ true),
    )
    .expect("snapshot");

    assert_eq!(
        snapshot.revision(),
        &SessionMcpRevision::new(vec![
            revision(
                "alpha-search",
                Version::new(2, 0, 1),
                7,
                SessionMcpTransportKind::Http
            ),
            revision(
                "zeta-search",
                Version::new(1, 2, 0),
                0,
                SessionMcpTransportKind::Stdio
            ),
        ])
    );
    let debug = format!("{:?}", snapshot);
    assert!(!debug.contains("super-secret"));
    match snapshot.servers() {
        [McpServer::Http(http), McpServer::Stdio(stdio)] => {
            assert_eq!(http.name, "ora-space/alpha-search");
            assert_eq!(http.headers[0].value, "Bearer super-secret");
            assert_eq!(stdio.name, "ora-space/zeta-search");
            assert_eq!(
                std::fs::canonicalize(&stdio.command).expect("mapped command"),
                std::fs::canonicalize(&fixture.command).expect("package command")
            );
            assert_eq!(
                stdio.args,
                vec![".".to_string(), cwd.to_string_lossy().into_owned()]
            );
        }
        other => panic!("unexpected servers: {other:?}"),
    }
}

#[test]
fn omits_incomplete_mcp_without_failing_complete_peers() {
    let fixture = Fixture::new();
    let catalog = FakeCatalog::new(vec![
        candidate(
            "incomplete",
            Version::new(1, 0, 0),
            &fixture.package_root,
            http_config(),
        ),
        candidate(
            "ready",
            Version::new(1, 0, 0),
            &fixture.package_root,
            stdio_config(),
        ),
    ]);
    let configurations = FakeConfigurations {
        by_id: BTreeMap::from([
            (
                plugin("incomplete").canonical(),
                McpConfigurationEligibility::Incomplete,
            ),
            (
                plugin("ready").canonical(),
                McpConfigurationEligibility::NoSettings,
            ),
        ]),
    };

    let snapshot = resolve_session_mcp(
        &catalog,
        &configurations,
        &fixture.package_root,
        capabilities(/*load_session*/ true, /*http*/ false),
    )
    .expect("snapshot");
    assert_eq!(snapshot.servers().len(), 1);
    assert_eq!(snapshot.revision().members()[0].plugin_id, plugin("ready"));
}

#[test]
fn fails_closed_when_http_capability_is_missing() {
    let fixture = Fixture::new();
    let catalog = FakeCatalog::new(vec![candidate(
        "alpha-search",
        Version::new(1, 0, 0),
        &fixture.package_root,
        http_config(),
    )]);
    let configurations = FakeConfigurations {
        by_id: BTreeMap::from([(
            plugin("alpha-search").canonical(),
            McpConfigurationEligibility::Complete {
                revision: 1,
                values: BTreeMap::from([(
                    "apiKey".to_string(),
                    SettingValue::String("secret".into()),
                )]),
            },
        )]),
    };

    let error = resolve_session_mcp(
        &catalog,
        &configurations,
        &fixture.package_root,
        capabilities(/*load_session*/ true, /*http*/ false),
    )
    .expect_err("http capability");
    assert_eq!(error.code().as_str(), "mcp_http_capability_missing");
    assert!(!error.to_string().contains("secret"));
    let public = error.public_error();
    let encoded = serde_json::to_string(&public).expect("json");
    assert!(!encoded.contains("secret"));
}

#[test]
fn fails_before_send_when_load_capability_is_missing_for_a_non_empty_set() {
    let fixture = Fixture::new();
    let catalog = FakeCatalog::new(vec![candidate(
        "ready",
        Version::new(1, 0, 0),
        &fixture.package_root,
        stdio_config(),
    )]);
    let configurations = FakeConfigurations {
        by_id: BTreeMap::from([(
            plugin("ready").canonical(),
            McpConfigurationEligibility::NoSettings,
        )]),
    };

    let error = resolve_session_mcp(
        &catalog,
        &configurations,
        &fixture.package_root,
        capabilities(/*load_session*/ false, /*http*/ false),
    )
    .expect_err("load capability");
    assert!(matches!(error, SessionMcpError::LoadCapabilityMissing));
}

#[test]
fn empty_set_does_not_require_load_capability() {
    let catalog = FakeCatalog::new(vec![]);
    let configurations = FakeConfigurations {
        by_id: BTreeMap::new(),
    };
    let snapshot = resolve_session_mcp(
        &catalog,
        &configurations,
        Path::new("/tmp"),
        capabilities(/*load_session*/ false, /*http*/ false),
    )
    .expect("empty snapshot");
    assert!(snapshot.servers().is_empty());
    assert!(snapshot.revision().is_empty());
}

#[test]
fn regenerates_when_package_version_changes_during_resolve() {
    let fixture = Fixture::new();
    let first = candidate(
        "ready",
        Version::new(1, 0, 0),
        &fixture.package_root,
        stdio_config(),
    );
    let second = candidate(
        "ready",
        Version::new(1, 1, 0),
        &fixture.package_root,
        stdio_config(),
    );
    let catalog = FakeCatalog::new(vec![first]);
    let configurations = FakeConfigurations {
        by_id: BTreeMap::from([(
            plugin("ready").canonical(),
            McpConfigurationEligibility::NoSettings,
        )]),
    };
    catalog.then(vec![second.clone()]);

    let snapshot = resolve_session_mcp(
        &catalog,
        &configurations,
        &fixture.package_root,
        capabilities(/*load_session*/ true, /*http*/ false),
    )
    .expect("retry snapshot");
    assert_eq!(
        snapshot.revision().members()[0].package_version,
        Version::new(1, 1, 0)
    );
}

#[test]
fn desired_revision_excludes_setting_values() {
    let fixture = Fixture::new();
    let catalog = FakeCatalog::new(vec![candidate(
        "alpha-search",
        Version::new(1, 0, 0),
        &fixture.package_root,
        http_config(),
    )]);
    let configurations = FakeConfigurations {
        by_id: BTreeMap::from([(
            plugin("alpha-search").canonical(),
            McpConfigurationEligibility::Complete {
                revision: 3,
                values: BTreeMap::from([(
                    "apiKey".to_string(),
                    SettingValue::String("never-in-revision".into()),
                )]),
            },
        )]),
    };

    let revision = resolve_session_mcp_revision(&catalog, &configurations).expect("revision");
    let debug = format!("{revision:?}");
    assert!(!debug.contains("never-in-revision"));
    assert_eq!(revision.members()[0].configuration_revision, 3);
}

#[test]
fn live_idle_session_owes_refresh_when_desired_changes() {
    let previous = SessionMcpRevision::new(vec![revision(
        "ready",
        Version::new(1, 0, 0),
        1,
        SessionMcpTransportKind::Stdio,
    )]);
    let next = SessionMcpRevision::new(vec![revision(
        "ready",
        Version::new(1, 0, 0),
        2,
        SessionMcpTransportKind::Stdio,
    )]);
    let (state, refresh) = LiveMcpState::Active(previous.clone())
        .on_event(LiveMcpEvent::DesiredObserved(next.clone()));
    assert_eq!(
        state,
        LiveMcpState::RefreshPending {
            active: previous,
            desired: next.clone(),
        }
    );
    assert!(refresh);
    assert_eq!(
        state.prompt_admission(&next),
        LiveMcpPromptAdmission::RefreshFirst { desired: next }
    );
}

#[test]
fn live_busy_success_cannot_clear_a_newer_pending_revision() {
    let first = SessionMcpRevision::new(vec![revision(
        "ready",
        Version::new(1, 0, 0),
        1,
        SessionMcpTransportKind::Stdio,
    )]);
    let second = SessionMcpRevision::new(vec![revision(
        "ready",
        Version::new(1, 0, 0),
        2,
        SessionMcpTransportKind::Stdio,
    )]);
    let third = SessionMcpRevision::new(vec![revision(
        "ready",
        Version::new(1, 0, 0),
        3,
        SessionMcpTransportKind::Stdio,
    )]);
    let refreshing = LiveMcpState::Refreshing {
        in_flight: first.clone(),
        newer: None,
    };
    let (refreshing, _) = refreshing.on_event(LiveMcpEvent::DesiredObserved(second.clone()));
    let (refreshing, _) = refreshing.on_event(LiveMcpEvent::DesiredObserved(third.clone()));
    let (state, refresh) = refreshing.on_event(LiveMcpEvent::RefreshSucceeded(first.clone()));
    assert_eq!(
        state,
        LiveMcpState::RefreshPending {
            active: first,
            desired: third.clone(),
        }
    );
    assert!(refresh);
    assert!(!state.is_current(&second));
}

#[test]
fn live_refresh_failure_blocks_prompts_until_retry() {
    let desired = SessionMcpRevision::new(vec![revision(
        "ready",
        Version::new(1, 0, 0),
        4,
        SessionMcpTransportKind::Stdio,
    )]);
    let (state, _) = LiveMcpState::Refreshing {
        in_flight: desired.clone(),
        newer: None,
    }
    .on_event(LiveMcpEvent::RefreshFailed {
        requested: desired.clone(),
    });
    assert_eq!(
        state,
        LiveMcpState::Blocked {
            desired: desired.clone()
        }
    );
    assert_eq!(
        state.prompt_admission(&desired),
        LiveMcpPromptAdmission::RefreshFirst { desired }
    );
}

#[test]
fn stopped_sessions_ignore_desired_changes() {
    let desired = SessionMcpRevision::new(vec![revision(
        "ready",
        Version::new(1, 0, 0),
        1,
        SessionMcpTransportKind::Stdio,
    )]);
    let (state, refresh) = LiveMcpState::Inactive.on_event(LiveMcpEvent::DesiredObserved(desired));
    assert_eq!(state, LiveMcpState::Inactive);
    assert!(!refresh);
}

#[test]
fn resolver_does_not_create_workspace_files() {
    let fixture = Fixture::new();
    let workspace = TempDir::new().expect("workspace");
    let before = collect_paths(workspace.path());
    let snapshot = resolve_session_mcp(
        &FakeCatalog::new(vec![candidate(
            "ready",
            Version::new(1, 0, 0),
            &fixture.package_root,
            stdio_config(),
        )]),
        &FakeConfigurations {
            by_id: BTreeMap::from([(
                plugin("ready").canonical(),
                McpConfigurationEligibility::NoSettings,
            )]),
        },
        workspace.path(),
        capabilities(/*load_session*/ true, /*http*/ false),
    )
    .expect("snapshot");
    assert_eq!(snapshot.servers().len(), 1);
    assert_eq!(collect_paths(workspace.path()), before);
}

fn collect_paths(root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    fn walk(dir: &Path, paths: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read") {
            let entry = entry.expect("entry");
            let path = entry.path();
            if path.is_dir() {
                walk(&path, paths);
            }
            paths.push(path);
        }
    }
    walk(root, &mut paths);
    paths.sort();
    paths
}

#[tokio::test]
async fn mcp_and_effect_share_one_agent_session_barrier() {
    use crate::session_setup::{AgentSessionBarriers, BarrierReason};
    let barriers = AgentSessionBarriers::new();
    let plugin = plugin("opencode");
    let first = barriers
        .for_plugin(&plugin)
        .try_acquire(BarrierReason::McpRefresh)
        .expect("first hold");
    assert!(barriers.for_plugin(&plugin).is_held());
    assert!(
        barriers
            .for_plugin(&plugin)
            .try_acquire(BarrierReason::EffectMutation)
            .is_none()
    );
    drop(first);
    assert!(
        barriers
            .for_plugin(&plugin)
            .try_acquire(BarrierReason::AgentReplacement)
            .is_some()
    );
}

#[test]
fn unrelated_agents_do_not_share_a_barrier() {
    use crate::session_setup::{AgentSessionBarriers, BarrierReason};
    let barriers = AgentSessionBarriers::new();
    let _hold = barriers
        .for_plugin(&plugin("opencode"))
        .try_acquire(BarrierReason::McpRefresh)
        .expect("opencode hold");
    assert!(
        barriers
            .for_plugin(&plugin("claude"))
            .try_acquire(BarrierReason::EffectMutation)
            .is_some()
    );
}
