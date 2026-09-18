//! Behavior tests for in-memory Host MCP health.

use super::super::tests::{
    FakeCatalog, FakeConfigurations, Fixture, candidate, plugin, stdio_config,
};
use super::super::{SessionMcpSelection, resolve_session_mcp};
use super::{McpHealthIdentity, McpHealthStore, map_probe_error};
use crate::app_event::AppEventHub;
use ora_contracts::{
    AppEvent, ListMcpHealthRequest, McpHealthErrorCode, McpHealthStatus, McpHealthUnknownReason,
    ProbeMcpHealthRequest,
};
use ora_domain::SessionId;
use ora_plugin_config::{
    CompiledMcpConfiguration, McpHttpTransport, McpStdioTransport, McpTransport,
    McpValueExpression, SettingValue,
};
use ora_utils::mcp::ProbeError;
use ora_utils::path::PortableRelativePath;
use pretty_assertions::assert_eq;
use semver::Version;
use std::collections::{BTreeMap, BTreeSet};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing_subscriber::Registry;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;
use url::Url;

/// Test probe timeout: long enough to prove sharing, short enough to keep the suite quick.
const TEST_PROBE_TIMEOUT: Duration = Duration::from_millis(300);

fn store() -> (McpHealthStore, AppEventHub) {
    let hub = AppEventHub::new();
    (
        McpHealthStore::new(hub.publisher(), TEST_PROBE_TIMEOUT),
        hub,
    )
}

/// Builds one configuration-complete stdio member whose command cannot be started.
///
/// An empty regular file inside the package is a portable "file exists but is not a runnable
/// program": Windows rejects it as a bad executable and Unix rejects the missing execute bit, so
/// both platforms reach `mcp_spawn_failed` without a platform-specific fixture.
fn unrunnable_stdio_member(fixture: &Fixture, name: &str) -> super::EligibleMcpMember {
    let command = fixture.package_root.join("assets").join("not-runnable");
    std::fs::write(&command, b"").expect("write unrunnable command");
    let configuration = CompiledMcpConfiguration {
        schema_version: 1,
        settings: None,
        transport: McpTransport::Stdio(McpStdioTransport {
            command: PortableRelativePath::parse("assets/not-runnable").expect("command"),
            // Workspace context makes the member's identity cwd-bound.
            args: vec![ora_plugin_config::McpArgument::WorkspaceContext],
            env: BTreeMap::new(),
        }),
    };
    super::EligibleMcpMember {
        candidate: candidate(
            name,
            Version::new(1, 0, 0),
            &fixture.package_root,
            configuration,
        ),
        configuration_revision: 0,
        values: BTreeMap::new(),
    }
}

/// Builds one configuration-complete HTTP member carrying a Setting-backed secret header.
fn http_member_with_secret(fixture: &Fixture, name: &str, url: Url) -> super::EligibleMcpMember {
    let configuration = CompiledMcpConfiguration {
        schema_version: 1,
        settings: None,
        transport: McpTransport::Http(McpHttpTransport {
            url,
            headers: BTreeMap::from([(
                "Authorization".to_string(),
                McpValueExpression::Setting {
                    id: "apiKey".to_string(),
                    prefix: "Bearer ".to_string(),
                    suffix: String::new(),
                },
            )]),
        }),
    };
    super::EligibleMcpMember {
        candidate: candidate(
            name,
            Version::new(1, 0, 0),
            &fixture.package_root,
            configuration,
        ),
        configuration_revision: 1,
        values: BTreeMap::from([(
            "apiKey".to_string(),
            SettingValue::String("super-secret-key".into()),
        )]),
    }
}

/// Configuration source marking exactly the named plugins complete.
fn complete_configurations(members: &[super::EligibleMcpMember]) -> FakeConfigurations {
    FakeConfigurations {
        by_id: members
            .iter()
            .map(|member| {
                (
                    member.candidate.plugin_id.canonical(),
                    super::super::McpConfigurationEligibility::Complete {
                        revision: member.configuration_revision,
                        values: member.values.clone(),
                    },
                )
            })
            .collect(),
    }
}

/// Loopback server that accepts connections, never answers, and counts every accept.
///
/// Holding the streams open keeps each client request pending until its own timeout, which makes
/// the probe's hard timeout and single-flight sharing observable without external network access.
struct SilentHttpServer {
    url: Url,
    accepts: Arc<AtomicUsize>,
    stop: mpsc::Sender<()>,
    handle: Option<JoinHandle<()>>,
}

impl SilentHttpServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind silent server");
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let addr = listener.local_addr().expect("silent server address");
        let accepts = Arc::new(AtomicUsize::new(0));
        let counter = accepts.clone();
        let (stop, stop_rx) = mpsc::channel::<()>();
        let handle = thread::spawn(move || {
            let mut held: Vec<TcpStream> = Vec::new();
            loop {
                if stop_rx.try_recv().is_ok() {
                    return;
                }
                match listener.accept() {
                    Ok((stream, _)) => {
                        counter.fetch_add(1, Ordering::SeqCst);
                        held.push(stream);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => return,
                }
            }
        });
        Self {
            url: Url::parse(&format!("http://{addr}/mcp")).expect("silent server url"),
            accepts,
            stop,
            handle: Some(handle),
        }
    }

    fn accepts(&self) -> usize {
        self.accepts.load(Ordering::SeqCst)
    }
}

impl Drop for SilentHttpServer {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Runs one async body under a TRACE subscriber scoped to this thread so structured health events
/// are observable even though probes settle on spawned tasks.
fn with_scoped_trace<T>(action: impl std::future::Future<Output = T>) -> T {
    let subscriber = Registry::default().with(LevelFilter::TRACE);
    let guard = tracing::subscriber::set_default(subscriber);
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime")
        .block_on(action);
    drop(guard);
    result
}

/// Waits until an identity has a completed result, letting spawned probe tasks run.
async fn wait_ready(store: &McpHealthStore, identity: &McpHealthIdentity) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if store.ready_entry(identity).is_some() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("health result did not settle before the deadline");
}

#[test]
fn configuration_incomplete_member_is_not_listed_or_probed() {
    with_scoped_trace(async {
        let fixture = Fixture::new();
        let (store, _) = store();
        let catalog = FakeCatalog::new(vec![candidate(
            "search",
            Version::new(1, 0, 0),
            &fixture.package_root,
            stdio_config(),
        )]);
        let configurations = FakeConfigurations {
            by_id: BTreeMap::from([(
                plugin("search").canonical(),
                super::super::McpConfigurationEligibility::Incomplete,
            )]),
        };

        // Invariant 11: an unconfigured plugin is never probed and has no health row.
        let listed = store
            .list(
                &catalog,
                &configurations,
                ListMcpHealthRequest { cwd: None },
            )
            .expect("list");
        assert!(listed.entries.is_empty());
        let error = store
            .probe(
                &catalog,
                &configurations,
                ProbeMcpHealthRequest {
                    plugin_id: plugin("search").canonical(),
                    cwd: None,
                },
            )
            .await
            .expect_err("ineligible probe must be refused");
        assert_eq!(
            error.classification(),
            crate::ErrorClassification::InvalidRequest
        );
    });
}

/// Workspace-context members bind the Session cwd, stay `context_missing` on the card, and never
/// write a Session result back into the card identity.
#[test]
fn workspace_context_member_binds_session_cwd_and_keeps_card_separate() {
    with_scoped_trace(async {
        let fixture = Fixture::new();
        let (store, _) = store();
        let member = unrunnable_stdio_member(&fixture, "workspace-tool");
        let catalog = FakeCatalog::new(vec![member.candidate.clone()]);
        let configurations = complete_configurations(std::slice::from_ref(&member));
        let plugin_id = member.candidate.plugin_id.canonical();
        let workspace = fixture.package_root.clone();
        let card_identity = McpHealthIdentity::for_member(&member, /*cwd*/ None);
        let session_identity = McpHealthIdentity::for_member(&member, Some(workspace.as_path()));
        assert_ne!(card_identity, session_identity);

        // Card view: no real cwd, so the member reports context_missing rather than a fake result.
        let card_view = store
            .list(
                &catalog,
                &configurations,
                ListMcpHealthRequest { cwd: None },
            )
            .expect("card list");
        assert_eq!(
            card_view.entries[0].status,
            McpHealthStatus::Unknown {
                reason: McpHealthUnknownReason::ContextMissing
            }
        );
        let refused = store
            .probe(
                &catalog,
                &configurations,
                ProbeMcpHealthRequest {
                    plugin_id: plugin_id.clone(),
                    cwd: None,
                },
            )
            .await
            .expect("card re-detect is allowed to report context_missing");
        assert_eq!(
            refused.entry.status,
            McpHealthStatus::Unknown {
                reason: McpHealthUnknownReason::ContextMissing
            }
        );
        // Nothing was probed, so no card identity result exists to present.
        assert_eq!(
            store.status_for(&card_identity),
            McpHealthStatus::Unknown {
                reason: McpHealthUnknownReason::NotProbed
            }
        );

        // Session view: the real absolute cwd is substituted and the member is probed for real.
        let session_view = store
            .list(
                &catalog,
                &configurations,
                ListMcpHealthRequest {
                    cwd: Some(workspace.to_string_lossy().into_owned()),
                },
            )
            .expect("session list");
        assert_eq!(
            session_view.entries[0].identity.cwd,
            Some(workspace.to_string_lossy().into_owned())
        );
        assert_eq!(
            session_view.entries[0].status,
            McpHealthStatus::Unknown {
                reason: McpHealthUnknownReason::NotProbed
            }
        );
        let probed = store
            .probe(
                &catalog,
                &configurations,
                ProbeMcpHealthRequest {
                    plugin_id,
                    cwd: Some(workspace.to_string_lossy().into_owned()),
                },
            )
            .await
            .expect("session probe");
        assert_eq!(
            probed.entry.status,
            McpHealthStatus::Unhealthy {
                error_code: McpHealthErrorCode::McpSpawnFailed
            }
        );
        assert_eq!(
            store.status_for(&session_identity),
            McpHealthStatus::Unhealthy {
                error_code: McpHealthErrorCode::McpSpawnFailed
            }
        );
        // The Session probe must not become the card's answer.
        assert_eq!(
            store.status_for(&card_identity),
            McpHealthStatus::Unknown {
                reason: McpHealthUnknownReason::NotProbed
            }
        );
    });
}

/// Concurrent triggers for one identity share a single probe.
#[test]
fn concurrent_triggers_share_one_probe() {
    with_scoped_trace(async {
        let fixture = Fixture::new();
        let server = SilentHttpServer::start();
        let (store, _) = store();
        let member = http_member_with_secret(&fixture, "http-tool", server.url.clone());
        let catalog = FakeCatalog::new(vec![member.candidate.clone()]);
        let configurations = complete_configurations(std::slice::from_ref(&member));
        let request = ProbeMcpHealthRequest {
            plugin_id: member.candidate.plugin_id.canonical(),
            cwd: None,
        };

        let started = Instant::now();
        let (first, second) = tokio::join!(
            store.probe(&catalog, &configurations, request.clone()),
            store.probe(&catalog, &configurations, request),
        );
        let first = first.expect("first probe");
        let second = second.expect("second probe");
        assert_eq!(first, second);
        assert_eq!(
            first.entry.status,
            McpHealthStatus::Unhealthy {
                error_code: McpHealthErrorCode::McpProbeTimeout
            }
        );
        // One connection means one probe; two triggers that did not share would open two.
        assert_eq!(server.accepts(), 1);
        // Sharing also means the two triggers overlap instead of running back to back.
        assert!(
            started.elapsed() < TEST_PROBE_TIMEOUT * 2,
            "probes did not overlap: {:?}",
            started.elapsed()
        );
    });
}

/// A Session backfill reuses a result whose identity already matches, instead of probing again.
#[test]
fn session_observation_reuses_a_matching_identity() {
    with_scoped_trace(async {
        let fixture = Fixture::new();
        let server = SilentHttpServer::start();
        let (store, _) = store();
        let member = http_member_with_secret(&fixture, "http-tool", server.url.clone());
        let catalog = FakeCatalog::new(vec![member.candidate.clone()]);
        let configurations = complete_configurations(std::slice::from_ref(&member));
        let identity = McpHealthIdentity::for_member(&member, /*cwd*/ None);

        store
            .probe(
                &catalog,
                &configurations,
                ProbeMcpHealthRequest {
                    plugin_id: member.candidate.plugin_id.canonical(),
                    cwd: None,
                },
            )
            .await
            .expect("seed probe");
        assert_eq!(server.accepts(), 1);

        // The Session set contains the same eligible member, whose identity already matched, so the
        // backfill presents the existing result and opens no new connection.
        store.clone().spawn_session_observation(
            catalog.clone(),
            configurations.clone(),
            SessionMcpSelection::Automatic,
            SessionId::new("session-reuse"),
            fixture.package_root.clone(),
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(server.accepts(), 1);
        assert_eq!(
            store.status_for(&identity),
            McpHealthStatus::Unhealthy {
                error_code: McpHealthErrorCode::McpProbeTimeout
            }
        );
    });
}

/// Explicit empty selection resolves no members, so no backfill work exists for it.
#[test]
fn explicit_empty_selection_has_no_members_or_backfill() {
    with_scoped_trace(async {
        let fixture = Fixture::new();
        let member = unrunnable_stdio_member(&fixture, "workspace-tool");
        let catalog = FakeCatalog::new(vec![member.candidate.clone()]);
        let configurations = complete_configurations(std::slice::from_ref(&member));
        let selection = SessionMcpSelection::Explicit(BTreeSet::new());

        let snapshot = resolve_session_mcp(
            &catalog,
            &configurations,
            &fixture.package_root,
            super::super::AgentSessionMcpCapabilities::new(
                /*load_session*/ true, /*http*/ true,
            ),
            &selection,
        )
        .expect("empty snapshot");
        assert!(snapshot.servers().is_empty());
        assert!(snapshot.revision().is_empty());

        // The same empty selection is the Session observation's input, and it must schedule no
        // probe at all. Neither identity may have a stored result: a probe here would have failed
        // visibly, because this member's command cannot start.
        let (store, _) = store();
        store.clone().spawn_session_observation(
            catalog.clone(),
            configurations.clone(),
            selection,
            SessionId::new("session-empty"),
            fixture.package_root.clone(),
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
        let card_identity = McpHealthIdentity::for_member(&member, /*cwd*/ None);
        let session_identity =
            McpHealthIdentity::for_member(&member, Some(fixture.package_root.as_path()));
        for identity in [card_identity, session_identity] {
            assert_eq!(
                store.status_for(&identity),
                McpHealthStatus::Unknown {
                    reason: McpHealthUnknownReason::NotProbed
                }
            );
        }
    });
}

/// A non-empty Explicit whitelist backfills only its own members, binding the real Session cwd.
#[test]
fn explicit_selection_backfills_only_its_whitelist() {
    with_scoped_trace(async {
        let fixture = Fixture::new();
        let selected = unrunnable_stdio_member(&fixture, "selected-tool");
        let unselected = unrunnable_stdio_member(&fixture, "unselected-tool");
        let catalog = FakeCatalog::new(vec![
            selected.candidate.clone(),
            unselected.candidate.clone(),
        ]);
        let configurations = complete_configurations(&[selected.clone(), unselected.clone()]);
        let (store, _) = store();
        let selection =
            SessionMcpSelection::Explicit(BTreeSet::from([selected.candidate.plugin_id.clone()]));
        store.clone().spawn_session_observation(
            catalog.clone(),
            configurations.clone(),
            selection,
            SessionId::new("session-explicit"),
            fixture.package_root.clone(),
        );
        tokio::time::sleep(Duration::from_millis(200)).await;

        // The whitelisted member is probed with the Session's real cwd and fails to spawn.
        assert_eq!(
            store.status_for(&McpHealthIdentity::for_member(
                &selected,
                Some(fixture.package_root.as_path())
            )),
            McpHealthStatus::Unhealthy {
                error_code: McpHealthErrorCode::McpSpawnFailed
            }
        );
        // A member the Session never selected is never probed, even though it is eligible.
        assert_eq!(
            store.status_for(&McpHealthIdentity::for_member(
                &unselected,
                Some(fixture.package_root.as_path())
            )),
            McpHealthStatus::Unknown {
                reason: McpHealthUnknownReason::NotProbed
            }
        );
    });
}

/// Invalidating one plugin drops every identity of it and tells clients to re-query.
#[test]
fn invalidation_clears_plugin_identities_and_publishes() {
    with_scoped_trace(async {
        let fixture = Fixture::new();
        let server = SilentHttpServer::start();
        let (store, hub) = store();
        let mut events = hub.subscribe();
        assert_eq!(
            events.recv().await.expect("ready").expect("event"),
            AppEvent::Ready
        );
        let member = http_member_with_secret(&fixture, "http-tool", server.url.clone());
        let catalog = FakeCatalog::new(vec![member.candidate.clone()]);
        let configurations = complete_configurations(std::slice::from_ref(&member));
        let plugin_id = member.candidate.plugin_id.clone();
        let identity = McpHealthIdentity::for_member(&member, /*cwd*/ None);

        store
            .probe(
                &catalog,
                &configurations,
                ProbeMcpHealthRequest {
                    plugin_id: plugin_id.canonical(),
                    cwd: None,
                },
            )
            .await
            .expect("seed probe");
        assert!(store.ready_entry(&identity).is_some());
        // The first published change belongs to the completed probe.
        assert_eq!(
            events.recv().await.expect("probe event").expect("event"),
            AppEvent::McpHealthChanged {
                plugin_id: plugin_id.canonical()
            }
        );

        store.invalidate_plugin(&plugin_id);
        assert_eq!(
            store.status_for(&identity),
            McpHealthStatus::Unknown {
                reason: McpHealthUnknownReason::NotProbed
            }
        );
        assert!(store.ready_entry(&identity).is_none());
        assert_eq!(
            events
                .recv()
                .await
                .expect("invalidate event")
                .expect("event"),
            AppEvent::McpHealthChanged {
                plugin_id: plugin_id.canonical()
            }
        );
    });
}

/// A fresh store has no history: nothing about health survives a process restart.
#[test]
fn restart_starts_from_not_probed() {
    with_scoped_trace(async {
        let fixture = Fixture::new();
        let server = SilentHttpServer::start();
        let member = http_member_with_secret(&fixture, "http-tool", server.url.clone());
        let catalog = FakeCatalog::new(vec![member.candidate.clone()]);
        let configurations = complete_configurations(std::slice::from_ref(&member));
        let identity = McpHealthIdentity::for_member(&member, /*cwd*/ None);

        let (first, _) = store();
        first
            .probe(
                &catalog,
                &configurations,
                ProbeMcpHealthRequest {
                    plugin_id: member.candidate.plugin_id.canonical(),
                    cwd: None,
                },
            )
            .await
            .expect("seed probe");
        assert!(first.ready_entry(&identity).is_some());

        let (restarted, _) = store();
        assert_eq!(
            restarted.status_for(&identity),
            McpHealthStatus::Unknown {
                reason: McpHealthUnknownReason::NotProbed
            }
        );
    });
}

/// Probe failures never change what Session setup delivers.
#[test]
fn probe_failure_does_not_change_the_delivered_snapshot() {
    with_scoped_trace(async {
        let fixture = Fixture::new();
        let (store, _) = store();
        let member = unrunnable_stdio_member(&fixture, "workspace-tool");
        let catalog = FakeCatalog::new(vec![member.candidate.clone()]);
        let configurations = complete_configurations(std::slice::from_ref(&member));
        let capabilities = super::super::AgentSessionMcpCapabilities::new(
            /*load_session*/ true, /*http*/ true,
        );
        let selection = SessionMcpSelection::Automatic;

        let before = resolve_session_mcp(
            &catalog,
            &configurations,
            &fixture.package_root,
            capabilities,
            &selection,
        )
        .expect("snapshot before probing");

        let probed = store
            .probe(
                &catalog,
                &configurations,
                ProbeMcpHealthRequest {
                    plugin_id: member.candidate.plugin_id.canonical(),
                    cwd: Some(fixture.package_root.to_string_lossy().into_owned()),
                },
            )
            .await
            .expect("probe");
        assert_eq!(
            probed.entry.status,
            McpHealthStatus::Unhealthy {
                error_code: McpHealthErrorCode::McpSpawnFailed
            }
        );

        let after = resolve_session_mcp(
            &catalog,
            &configurations,
            &fixture.package_root,
            capabilities,
            &selection,
        )
        .expect("snapshot after probing");
        // Delivery is unchanged: the complete list and its revision are identical.
        assert_eq!(after.revision(), before.revision());
        assert_eq!(after.servers(), before.servers());
    });
}

/// A result bound to an older identity is never presented as the current one.
#[test]
fn stale_identity_is_not_presented_as_current() {
    with_scoped_trace(async {
        let fixture = Fixture::new();
        let server = SilentHttpServer::start();
        let (store, _) = store();
        let member = http_member_with_secret(&fixture, "http-tool", server.url.clone());
        let catalog = FakeCatalog::new(vec![member.candidate.clone()]);
        let configurations = complete_configurations(std::slice::from_ref(&member));
        store
            .probe(
                &catalog,
                &configurations,
                ProbeMcpHealthRequest {
                    plugin_id: member.candidate.plugin_id.canonical(),
                    cwd: None,
                },
            )
            .await
            .expect("seed probe at revision one");

        // The configuration advances to a new revision: the previous identity's result is stale,
        // and a mismatch reads as `Unknown(not_probed)` rather than as the old answer.
        let mut advanced = member.clone();
        advanced.configuration_revision = 2;
        let advanced_configurations = complete_configurations(std::slice::from_ref(&advanced));
        let listed = store
            .list(
                &catalog,
                &advanced_configurations,
                ListMcpHealthRequest { cwd: None },
            )
            .expect("list at the new revision");
        assert_eq!(listed.entries[0].identity.configuration_revision, 2);
        assert_eq!(
            listed.entries[0].status,
            McpHealthStatus::Unknown {
                reason: McpHealthUnknownReason::NotProbed
            }
        );
    });
}

/// Transport failures map onto the closed product code family one to one.
#[test]
fn maps_every_probe_error_to_a_closed_product_code() {
    let cases = [
        (ProbeError::SpawnFailed, McpHealthErrorCode::McpSpawnFailed),
        (
            ProbeError::ExitedPrematurely,
            McpHealthErrorCode::McpExitedPrematurely,
        ),
        (
            ProbeError::HandshakeFailed,
            McpHealthErrorCode::McpHandshakeFailed,
        ),
        (ProbeError::Timeout, McpHealthErrorCode::McpProbeTimeout),
        (
            ProbeError::ToolsUnavailable,
            McpHealthErrorCode::McpToolsUnavailable,
        ),
        (
            ProbeError::HttpUnreachable,
            McpHealthErrorCode::McpHttpUnreachable,
        ),
        (
            ProbeError::HttpUnauthorized,
            McpHealthErrorCode::McpHttpUnauthorized,
        ),
        (
            ProbeError::HttpServerError,
            McpHealthErrorCode::McpHttpServerError,
        ),
    ];
    for (error, code) in cases {
        assert_eq!(map_probe_error(error), code);
    }
}

/// Every surface stays secret-free: the pairing log carries identity, code, and duration only.
#[test]
fn session_observation_pairs_secret_free_results() {
    let fixture = Fixture::new();
    let server = SilentHttpServer::start();
    let recorder = EventTextRecorder::default();
    ora_logging::with_recorded_trace_logging(recorder.layer(), || {
        let (store, _) = store();
        let member = http_member_with_secret(&fixture, "http-tool", server.url.clone());
        let catalog = FakeCatalog::new(vec![member.candidate.clone()]);
        let configurations = complete_configurations(std::slice::from_ref(&member));
        let identity = McpHealthIdentity::for_member(&member, /*cwd*/ None);

        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(async {
                store.clone().spawn_session_observation(
                    catalog,
                    configurations,
                    SessionMcpSelection::Automatic,
                    SessionId::new("session-pairing"),
                    fixture.package_root.clone(),
                );
                wait_ready(&store, &identity).await;
            });
        assert_eq!(
            store.status_for(&identity),
            McpHealthStatus::Unhealthy {
                error_code: McpHealthErrorCode::McpProbeTimeout
            }
        );
    });

    let recorded = recorder.text();
    assert!(
        recorded.contains("host MCP health probe result"),
        "{recorded}"
    );
    assert!(recorded.contains("session-pairing"), "{recorded}");
    assert!(
        recorded.contains(&format!("plugin_id={}", plugin("http-tool").canonical())),
        "{recorded}"
    );
    assert!(
        recorded.contains("mcp_health_code=mcp_probe_timeout"),
        "{recorded}"
    );
    assert!(recorded.contains("duration_ms="), "{recorded}");
    // No Setting value, credential, header, or endpoint may reach a structured log.
    assert!(!recorded.contains("super-secret-key"), "{recorded}");
    assert!(!recorded.contains("Authorization"), "{recorded}");
    assert!(!recorded.contains("Bearer"), "{recorded}");
    assert!(!recorded.contains(server.url.as_str()), "{recorded}");
}

/// Validates that each health test above only ever sees a cwd it actually supplied.
#[test]
fn rejects_relative_cwd_without_inventing_one() {
    with_scoped_trace(async {
        let fixture = Fixture::new();
        let (store, _) = store();
        let member = unrunnable_stdio_member(&fixture, "workspace-tool");
        let catalog = FakeCatalog::new(vec![member.candidate.clone()]);
        let configurations = complete_configurations(std::slice::from_ref(&member));

        let error = store
            .list(
                &catalog,
                &configurations,
                ListMcpHealthRequest {
                    cwd: Some("relative/workspace".to_string()),
                },
            )
            .expect_err("relative cwd must be refused");
        assert_eq!(
            error.classification(),
            crate::ErrorClassification::InvalidRequest
        );
    });
}

/// Captures the rendered fields of every event emitted into the scoped TRACE subscriber.
#[derive(Clone, Debug, Default)]
struct EventTextRecorder {
    text: Arc<Mutex<String>>,
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
    text: Arc<Mutex<String>>,
}

impl<S> Layer<S> for EventTextLayer
where
    S: tracing::Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, S>) {
        event.record(&mut EventTextVisitor {
            text: self.text.clone(),
        });
    }
}

/// Renders each field as `name=value` so the test can assert the whole leak boundary.
struct EventTextVisitor {
    text: Arc<Mutex<String>>,
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
