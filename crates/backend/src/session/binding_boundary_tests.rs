//! Host-boundary evidence uses real repositories and files; no synthetic provider execution is claimed.
use super::*;
use pretty_assertions::assert_eq;
use std::time::Duration;

/// Discovery followed by actual uninstall must not erase identities already recorded in history.
#[test]
fn uninstalling_a_discovered_plugin_preserves_repeated_binding_segments() {
    run_test(async {
        let fixture = Fixture::stopped();
        let package = fixture
            ._temporary
            .path()
            .join("plugins")
            .join("installed")
            .join("official")
            .join("ora-space.fixture")
            .join("1.0.0");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(package.join("main.js"), "export {};\n").unwrap();
        std::fs::write(package.join("orax.toml"), "resolver = 1\nidentifier = \"ora-space.fixture\"\nnamespace = \"official\"\nkind = \"agent\"\nversion = \"1.0.0\"\ndescription = \"Diagnostic fixture\"\n").unwrap();
        let discovered = fixture.plugins.scan(ScanPluginsRequest {}).await.unwrap();
        assert_eq!(
            discovered
                .plugins
                .into_iter()
                .map(|plugin| plugin.id)
                .collect::<Vec<_>>(),
            vec!["official/ora-space.fixture"]
        );
        let agent = AgentRef::parse("official/ora-space.fixture").unwrap();
        fixture
            .sessions
            .repository
            .update_session_binding(
                &SessionId::new("session-1"),
                agent.clone(),
                "provider-session",
                2,
            )
            .unwrap();
        let mut initial = meta(&fixture, "provider-session");
        initial.agent_ref = agent.clone();
        let mut expected = history(
            &fixture,
            vec![
                HistoryRecord::Meta(initial),
                HistoryRecord::AgentSwitched(AgentSwitch {
                    from: agent.clone(),
                    to: AgentRef::parse("missing/other").unwrap(),
                    agent_session_id: "other-session".into(),
                }),
                HistoryRecord::AgentSwitched(AgentSwitch {
                    from: AgentRef::parse("missing/other").unwrap(),
                    to: agent,
                    agent_session_id: "provider-session".into(),
                }),
            ],
        );
        expected[2].current = true;
        let before = query(&fixture, &["session-1"]).await;
        assert_eq!(
            before,
            GetSessionBindingsResponse {
                sessions: vec![SessionBindings::Available {
                    session_id: "session-1".into(),
                    history: BindingHistoryState::Readable,
                    segments: expected
                }]
            }
        );
        let path =
            ora_history::history_path(&fixture._temporary.path().join("sessions"), "session-1")
                .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        fixture
            .plugins
            .uninstall(UninstallPluginRequest {
                plugin_id: "official/ora-space.fixture".into(),
                data_disposition: PluginDataDisposition::Retain,
                hook_execution_acknowledged: false,
            })
            .await
            .unwrap();
        assert_eq!(
            fixture
                .plugins
                .list_installed(ListInstalledPluginsRequest {})
                .unwrap()
                .plugins
                .len(),
            0
        );
        assert!(!package.exists());
        assert_eq!(query(&fixture, &["session-1"]).await, before);
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    });
}

/// Calls the public diagnostic boundary without discarding per-session failures.
async fn query(fixture: &Fixture, ids: &[&str]) -> GetSessionBindingsResponse {
    fixture
        .sessions
        .bindings(GetSessionBindingsRequest {
            session_ids: ids.iter().map(|id| (*id).into()).collect(),
        })
        .await
        .expect("binding projection")
}

/// Unrecorded intermediate bindings cannot be recovered from a later authoritative row.
#[test]
fn overwritten_unrecorded_binding_is_not_fabricated() {
    run_test(async {
        let fixture = Fixture::stopped();
        let mut expected = history(
            &fixture,
            vec![HistoryRecord::Meta(meta(&fixture, "initial-provider"))],
        );
        let path =
            ora_history::history_path(&fixture._temporary.path().join("sessions"), "session-1")
                .unwrap();
        let before = std::fs::read(&path).unwrap();
        for (timestamp, id) in [(2, "never-recorded-provider"), (3, "current-provider")] {
            fixture
                .sessions
                .repository
                .update_session_binding(
                    &SessionId::new("session-1"),
                    AgentRef::parse("ora-space.fixture").unwrap(),
                    id,
                    timestamp,
                )
                .unwrap();
        }
        expected.push(SessionBindingSegment {
            key: "live".into(),
            agent_ref: "ora-space.fixture".into(),
            agent_session_id: "current-provider".into(),
            recorded_at: None,
            current: true,
        });
        assert_eq!(
            query(&fixture, &["session-1"]).await,
            GetSessionBindingsResponse {
                sessions: vec![SessionBindings::Available {
                    session_id: "session-1".into(),
                    history: BindingHistoryState::Readable,
                    segments: expected,
                }],
            }
        );
        assert_eq!(std::fs::read(path).unwrap(), before);
    });
}

/// Even persisted malformed ids cannot redirect the diagnostic reader to a caller-selected file.
#[test]
fn unsafe_persisted_identifiers_never_become_history_paths() {
    run_test(async {
        let fixture = Fixture::stopped();
        let template = fixture
            .sessions
            .repository
            .find_session(&SessionId::new("session-1"))
            .unwrap()
            .unwrap();
        let outside = fixture._temporary.path().join("private-history.jsonl");
        let sentinel = b"private-file-must-not-be-returned-or-written\n";
        std::fs::write(&outside, sentinel).unwrap();
        let absolute = outside.to_string_lossy().into_owned();
        // Without identifier validation, sharding resolves this id outside sessions/.
        // Keep the intermediate directories for Unix path traversal, and use matching
        // metadata so a regression produces a readable escaped binding, not another IO error.
        let escaping_id = "..-x/../../outside";
        std::fs::create_dir_all(fixture._temporary.path().join("sessions")).unwrap();
        std::fs::create_dir_all(fixture._temporary.path().join("-x").join("..-x")).unwrap();
        let escaped_path = fixture._temporary.path().join("outside.jsonl");
        let mut escaped_meta = meta(&fixture, "escaped-provider");
        escaped_meta.session_id = escaping_id.into();
        let escaped_bytes = format!(
            "{}\n",
            serde_json::to_string(&HistoryLine::new(
                "2026-10-22T10:00:00+08:00",
                0,
                HistoryRecord::Meta(escaped_meta)
            ))
            .unwrap()
        );
        std::fs::write(&escaped_path, &escaped_bytes).unwrap();
        let ids = [
            escaping_id,
            "../private-history",
            "..\\private-history",
            "file:///private-history.jsonl",
            absolute.as_str(),
            "",
            "with space",
        ];
        for id in ids {
            let mut row = template.clone();
            row.id = SessionId::new(id);
            fixture.sessions.repository.create_session(row).unwrap();
        }
        let before = fixture.sessions.repository.list_sessions().unwrap();
        let mut expected: Vec<_> = ids
            .into_iter()
            .map(|id| SessionBindings::Available {
                session_id: id.into(),
                history: BindingHistoryState::Unavailable,
                segments: vec![SessionBindingSegment {
                    key: "live".into(),
                    agent_ref: "ora-space.fixture".into(),
                    agent_session_id: "provider-session".into(),
                    recorded_at: None,
                    current: true,
                }],
            })
            .collect();
        expected.sort_by(|a, b| {
            let key = |value: &SessionBindings| match value {
                SessionBindings::Available { session_id, .. } => session_id.clone(),
                _ => unreachable!(),
            };
            key(a).cmp(&key(b))
        });
        assert_eq!(
            query(&fixture, &ids).await,
            GetSessionBindingsResponse { sessions: expected }
        );
        assert_eq!(std::fs::read(outside).unwrap(), sentinel);
        assert_eq!(
            std::fs::read_to_string(escaped_path).unwrap(),
            escaped_bytes
        );
        assert_eq!(fixture.sessions.repository.list_sessions().unwrap(), before);
    });
}

/// Records both event fields and span fields so identifiers cannot hide in span metadata.
#[derive(Clone, Default)]
struct DiagnosticFields(Arc<std::sync::Mutex<String>>);

struct FieldValues(String);

impl tracing::field::Visit for FieldValues {
    /// Preserves raw strings as well as debug fields so Windows path escaping cannot hide a leak.
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.push_str(&format!("{field}={value};"));
    }
    /// Captures string and debug-rendered values using the subscriber's fallback visitor.
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.push_str(&format!("{field}={value:?};"));
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for DiagnosticFields {
    /// Observes structured events emitted by the production read stack.
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        let mut values = FieldValues(String::new());
        event.record(&mut values);
        self.0.lock().unwrap().push_str(&values.0);
    }

    /// Includes fields supplied when a span is created.
    fn on_new_span(
        &self,
        attributes: &tracing::span::Attributes<'_>,
        _: &tracing::span::Id,
        _: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut values = FieldValues(String::new());
        attributes.record(&mut values);
        self.0.lock().unwrap().push_str(&values.0);
    }

    /// Includes fields supplied later to a span rather than only its initial values.
    fn on_record(
        &self,
        _: &tracing::span::Id,
        record: &tracing::span::Record<'_>,
        _: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut values = FieldValues(String::new());
        record.record(&mut values);
        self.0.lock().unwrap().push_str(&values.0);
    }
}

/// Runs the synchronous worker body under a scoped TRACE recorder, including corrupt input paths.
#[test]
fn projection_worker_logs_neither_provider_identifiers_nor_raw_history() {
    let recorder = DiagnosticFields::default();
    ora_logging::with_recorded_trace_logging(recorder.clone(), || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let fixture = Fixture::stopped();
                history(
                    &fixture,
                    vec![
                        HistoryRecord::Meta(meta(&fixture, "historical-log-secret")),
                        HistoryRecord::Gap {
                            reason: "raw-log-secret".into(),
                        },
                    ],
                );
                let root = fixture._temporary.path().join("sessions");
                let path = ora_history::history_path(&root, "session-1").unwrap();
                recorder.0.lock().unwrap().clear();
                ora_logging::ora_info!(message = "binding-projection-log-observer-ready");
                for corrupt in [false, true] {
                    if corrupt {
                        std::fs::write(&path, b"corrupt-log-secret\n").unwrap();
                    }
                    let response = super::super::super::bindings::read(
                        &fixture.sessions.repository,
                        &root,
                        "session-1",
                    );
                    assert!(matches!(
                        response,
                        SessionBindings::Available {
                            history: BindingHistoryState::Incomplete,
                            ..
                        }
                    ));
                    fixture
                        .sessions
                        .get(GetSessionRequest {
                            session_id: "session-1".into(),
                        })
                        .unwrap();
                    fixture.sessions.list(ListSessionsRequest {}).unwrap();
                }
                let logs = recorder.0.lock().unwrap().clone();
                assert!(
                    logs.contains("binding-projection-log-observer-ready"),
                    "recorder must be live"
                );
                for secret in [
                    "provider-session",
                    "historical-log-secret",
                    "raw-log-secret",
                    "corrupt-log-secret",
                    path.to_str().unwrap(),
                ] {
                    assert!(
                        !logs.contains(secret),
                        "projection worker log exposed sensitive input"
                    );
                }
                assert!(
                    !logs.contains(&format!("{path:?}")),
                    "debug-escaped history path leaked"
                );
            });
    });
}

/// An independent writer can commit while the production consistency seam is reading history.
#[test]
fn history_read_boundary_does_not_hold_a_database_write_lock() {
    run_test(async {
        // Backend::open starts detached Effect/Git workers which may acquire unrelated write
        // transactions. This boundary test needs only the real migrated database and repository.
        let temporary = TempDir::new().unwrap();
        let database = temporary.path().join("ora.sqlite3");
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(&database),
                &default_migration_catalog().unwrap(),
            )
            .unwrap();
        let seed = rusqlite::Connection::open(&database).unwrap();
        seed.execute_batch(
            "PRAGMA foreign_keys = ON;
             INSERT INTO projects (id, name, created_at, updated_at)
             VALUES ('project-1', 'Binding boundary', 1, 1);
             INSERT INTO workspace_locations (id, location_kind, locator_json, created_at, updated_at)
             VALUES ('location-1', 'local_filesystem', '{}', 1, 1);
             INSERT INTO workspaces (id, project_id, workspace_kind, location_id, lifecycle, created_at, updated_at)
             VALUES ('workspace-1', 'project-1', 'main', 'location-1', 'active', 1, 1);",
        )
        .unwrap();
        drop(seed);
        let repository = SqliteSessionRepository::new(pool);
        repository
            .create_session(Session::new(
                SessionId::new("session-1"),
                WorkspaceId::new("workspace-1"),
                AgentRef::parse("ora-space.fixture").unwrap(),
                "provider-session",
                ora_domain::SessionStatus::Stopped,
                ora_domain::SessionMcpSelection::Automatic,
                AuditFields::new(1, 1, /*is_deleted*/ false),
            ))
            .unwrap();
        let mut reads = 0;
        let result = super::super::super::bindings::read_consistent(
            &repository,
            "session-1",
            |_| {
                reads += 1;
                let connection = rusqlite::Connection::open(&database).unwrap();
                connection.busy_timeout(Duration::ZERO).unwrap();
                connection.execute_batch("BEGIN IMMEDIATE; UPDATE sessions SET title = 'committed-during-history-read' WHERE id = 'session-1'; COMMIT;").expect("history read must not hold the database writer slot");
                (BindingHistoryState::Readable, Vec::new())
            },
        );
        assert_eq!(reads, 1);
        assert_eq!(
            result,
            SessionBindings::Available {
                session_id: "session-1".into(),
                history: BindingHistoryState::Readable,
                segments: vec![SessionBindingSegment {
                    key: "live".into(),
                    agent_ref: "ora-space.fixture".into(),
                    agent_session_id: "provider-session".into(),
                    recorded_at: None,
                    current: true,
                }],
            }
        );
        assert_eq!(
            repository
                .find_session(&SessionId::new("session-1"))
                .unwrap()
                .unwrap()
                .title
                .map(|title| title.as_str().to_owned()),
            Some("committed-during-history-read".into())
        );
    });
}

/// A real FIFO holds file IO open while the single-thread executor commits another session's rename.
#[cfg(unix)]
#[test]
fn blocked_history_io_leaves_executor_and_database_writer_available() {
    use std::io::Write;
    run_test(async {
        let fixture = Fixture::stopped();
        let mut other = fixture
            .sessions
            .repository
            .find_session(&SessionId::new("session-1"))
            .unwrap()
            .unwrap();
        other.id = SessionId::new("parallel-session");
        fixture.sessions.repository.create_session(other).unwrap();
        let path =
            ora_history::history_path(&fixture._temporary.path().join("sessions"), "session-1")
                .unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&path)
                .status()
                .expect("POSIX mkfifo fixture")
                .success()
        );
        let line = HistoryLine::new(
            "2026-10-22T10:00:00+08:00",
            0,
            HistoryRecord::Meta(meta(&fixture, "provider-session")),
        );
        let mut bytes = serde_json::to_vec(&line).unwrap();
        bytes.push(b'\n');
        let (opened, ready) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let writer = std::thread::spawn(move || {
            ora_logging::with_trace_logging(|| {
                let mut file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
                let _ = opened.send(());
                // Dropping the sender also releases this fixture on an assertion failure.
                if wait.recv().is_ok() {
                    file.write_all(&bytes).unwrap();
                }
            });
        });
        let sessions = fixture.sessions.clone();
        let pending = tokio::spawn(async move {
            sessions
                .bindings(GetSessionBindingsRequest {
                    session_ids: vec!["session-1".into()],
                })
                .await
        });
        tokio::time::timeout(Duration::from_secs(10), ready)
            .await
            .expect("query opened FIFO")
            .unwrap();
        assert!(
            !pending.is_finished(),
            "projection waits for historical bytes"
        );
        let renamed = tokio::time::timeout(
            Duration::from_secs(10),
            fixture.sessions.rename(RenameSessionRequest {
                session_id: "parallel-session".into(),
                title: "renamed-while-file-io-blocked".into(),
            }),
        )
        .await
        .expect("executor and SQLite writer remain available")
        .unwrap();
        assert_eq!(
            renamed.session.title,
            Some("renamed-while-file-io-blocked".into())
        );
        assert!(
            !pending.is_finished(),
            "writer completes before releasing historical IO"
        );
        release.send(()).unwrap();
        let result = tokio::time::timeout(Duration::from_secs(10), pending)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        writer.join().unwrap();
        assert_eq!(
            result,
            GetSessionBindingsResponse {
                sessions: vec![SessionBindings::Available {
                    session_id: "session-1".into(),
                    history: BindingHistoryState::Readable,
                    segments: vec![SessionBindingSegment {
                        key: "history:0".into(),
                        agent_ref: "ora-space.fixture".into(),
                        agent_session_id: "provider-session".into(),
                        recorded_at: Some("2026-10-22T10:00:00+08:00".into()),
                        current: true
                    }],
                }]
            }
        );
    });
}

/// Raw configuration, tool output, paths and metadata never enter the binding projection or public sessions.
#[test]
fn sensitive_history_stays_out_of_diagnostics_and_public_session_payloads() {
    run_test(async {
        let fixture = Fixture::stopped();
        let mut expected = history(
            &fixture,
            vec![HistoryRecord::Meta(meta(
                &fixture,
                "historical-provider-secret",
            ))],
        );
        let path =
            ora_history::history_path(&fixture._temporary.path().join("sessions"), "session-1")
                .unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        let tool: HistoryRecord = serde_json::from_value(serde_json::json!({
            "type": "update", "update": {
                "sessionUpdate": "tool_call", "toolCallId": "private-tool-id",
                "title": "sensitive-tool-title", "status": "completed",
                "rawInput": {"token": "private-api-token"},
                "rawOutput": {"stdout": "private-tool-output"},
                "_meta": {"mcpServers": [{"env": {"SECRET": "private-mcp-secret"}}]}
            }
        }))
        .expect("valid sensitive tool history");
        bytes.extend(
            serde_json::to_vec(&HistoryLine::new("2026-10-22T10:01:00+08:00", 1, tool)).unwrap(),
        );
        bytes.push(b'\n');
        std::fs::write(&path, &bytes).unwrap();
        let raw_history = std::str::from_utf8(&bytes).unwrap();
        for secret in [
            "private-tool-id",
            "sensitive-tool-title",
            "private-api-token",
            "private-tool-output",
            "private-mcp-secret",
            "mcpServers",
            "rawInput",
            "rawOutput",
            "cwd",
        ] {
            assert!(
                raw_history.contains(secret),
                "fixture must actually persist {secret}"
            );
        }
        let before = fixture.sessions.repository.list_sessions().unwrap();
        expected.push(SessionBindingSegment {
            key: "live".into(),
            agent_ref: "ora-space.fixture".into(),
            agent_session_id: "provider-session".into(),
            recorded_at: None,
            current: true,
        });
        let response = query(&fixture, &["session-1"]).await;
        assert_eq!(
            response,
            GetSessionBindingsResponse {
                sessions: vec![SessionBindings::Available {
                    session_id: "session-1".into(),
                    history: BindingHistoryState::Readable,
                    segments: expected,
                }]
            }
        );
        let diagnostic = serde_json::to_string(&response).unwrap();
        let public = serde_json::to_string(&(
            fixture
                .sessions
                .get(GetSessionRequest {
                    session_id: "session-1".into(),
                })
                .unwrap(),
            fixture.sessions.list(ListSessionsRequest {}).unwrap(),
        ))
        .unwrap();
        for secret in [
            "private-tool-id",
            "sensitive-tool-title",
            "private-api-token",
            "private-tool-output",
            "private-mcp-secret",
            "mcpServers",
            "rawInput",
            "rawOutput",
            "cwd",
        ] {
            assert!(!diagnostic.contains(secret), "diagnostic exposed {secret}");
            assert!(!public.contains(secret), "public session exposed {secret}");
        }
        for id in ["historical-provider-secret", "provider-session"] {
            assert!(!public.contains(id), "public session exposed provider id");
        }
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        assert_eq!(fixture.sessions.repository.list_sessions().unwrap(), before);
    });
}

/// Windows sharing denial exercises real failed file opening while other batch items remain usable.
#[cfg(windows)]
#[test]
fn denied_history_open_is_isolated_and_recovers_after_release() {
    use std::os::windows::fs::OpenOptionsExt;
    run_test(async {
        let fixture = Fixture::stopped();
        let mut expected = history(
            &fixture,
            vec![HistoryRecord::Meta(meta(&fixture, "provider-session"))],
        );
        expected[0].current = true;
        let path =
            ora_history::history_path(&fixture._temporary.path().join("sessions"), "session-1")
                .unwrap();
        let before = std::fs::read(&path).unwrap();
        let guard = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        assert_eq!(
            query(&fixture, &["session-1", "missing"]).await,
            GetSessionBindingsResponse {
                sessions: vec![
                    SessionBindings::NotFound {
                        session_id: "missing".into()
                    },
                    SessionBindings::Available {
                        session_id: "session-1".into(),
                        history: BindingHistoryState::Unavailable,
                        segments: vec![SessionBindingSegment {
                            key: "live".into(),
                            agent_ref: "ora-space.fixture".into(),
                            agent_session_id: "provider-session".into(),
                            recorded_at: None,
                            current: true
                        }],
                    }
                ],
            }
        );
        drop(guard);
        assert_eq!(
            query(&fixture, &["session-1"]).await,
            GetSessionBindingsResponse {
                sessions: vec![SessionBindings::Available {
                    session_id: "session-1".into(),
                    history: BindingHistoryState::Readable,
                    segments: expected,
                }]
            }
        );
        assert_eq!(std::fs::read(path).unwrap(), before);
    });
}
