use super::*;
use crate::support::until;
use pretty_assertions::assert_eq;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Entry {
    Startup,
    SameVersion,
    Disconnect,
}

/// Pin the first initialize so the update lands before any provider session exists.
#[test]
fn first_session_rotates_changed_packages_before_session_new() {
    run(Entry::Startup);
}

/// With identical packages the same protocol interleaving keeps the original attempt.
#[test]
fn first_session_keeps_unchanged_packages() {
    run(Entry::SameVersion);
}

/// A new turn explicitly recovers the conversation after its actual plugin dies.
#[test]
fn disconnected_session_requires_stop_proof_before_replacement() {
    run(Entry::Disconnect);
}

/// Observe Node's durable attempt and the original process independently of ACP success.
fn run(entry: Entry) {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server = GitServerFixture::new(fixture.path(), fixture.path().join("main/.git"));
        let config = configuration(&fixture, &server);
        let root = prepare(&fixture, &config, &server);
        fs::write(
            root.join("mcp-capabilities.json"),
            r#"{"load":true,"http":true}"#,
        )
        .unwrap();
        install_mcp(&fixture, "1.0.0");
        install_mcp(&fixture, "2.0.0");
        let listener = host_listener(&root);
        let gate = root.join("hold-initialize");
        fs::write(&gate, []).unwrap();
        let mut child = launch(&fixture, &config);
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            let (mut stream, _) = connect(&fixture).await;
            let input = lifecycle_start(listener.local_addr().unwrap().port(), SECRET);
            send(&mut stream, ControllerToNodeMessage::StartAgentSession(input)).await;
            let methods = || fs::read_to_string(root.join("echo-acp-methods")).unwrap_or_default();
            until(|| methods().lines().any(|m| m == "initialize"));
            let original = plugin(&root);
            let home = fixture.config().home_directory.clone();
            let db = rusqlite::Connection::open(home.join("ora-node.sqlite3")).unwrap();
            let count = || db.query_row("SELECT count(*) FROM session_stop_attempts WHERE execution=?1",[EXECUTION],|r| r.get::<_,usize>(0)).unwrap();
            let attempt: String = db.query_row("SELECT attempt FROM session_stop_attempts WHERE execution=?1",[EXECUTION],|r| r.get(0)).unwrap();
            let lock = home.join("session-attempts").join(format!("{attempt}.lock"));
            let marker = fs::read(&lock).unwrap();
            if entry == Entry::Disconnect {
                fs::remove_file(&gate).unwrap();
                loop {
                    if let NodeToControllerMessage::ThreadEvent(event) = receive(&mut stream).await
                        && event.payload.record["type"] == "turnEnded" { break; }
                }
                fs::write(&lock, b"await original proof").unwrap();
                original.signal(ora_utils::process::ProcessSignal::Kill).unwrap();
                until(|| original.has_exited().unwrap());
                let mut next = turn();
                if let ControllerToNodeMessage::SubmitUserTurn(command) = &mut next {
                    command.payload.mcp_generation = Some(0);
                }
                send(&mut stream,next).await;
            } else {
                let mut update = version_update();
                if entry == Entry::SameVersion
                    && let ControllerToNodeMessage::UpdateSessionMcp(message) = &mut update
                    && let SessionMcpUpdateMember::Configured { plugin_version, .. } = &mut message.payload.members[0]
                {
                    *plugin_version = PluginVersion::new("1.0.0");
                }
                send(&mut stream,update).await;
                assert_eq!(reply(&mut stream).await.result,SessionMcpResult::Accepted);
                assert!(!methods().contains("session/new"));
                if entry == Entry::Startup { fs::write(&lock,b"await original proof").unwrap(); }
                fs::remove_file(&gate).unwrap();
            }
            if entry != Entry::SameVersion {
                until(|| db.query_row("SELECT first_stop_at_ms IS NOT NULL FROM session_stop_attempts WHERE attempt=?1",[&attempt],|r| r.get::<_,bool>(0)).unwrap());
                until(|| original.has_exited().unwrap());
                assert_eq!(count(),1);
                if entry == Entry::Startup { assert!(!methods().contains("session/new")); }
                fs::write(&lock,marker).unwrap();
                until(|| count() == 2);
                let confirmed: bool = db.query_row("SELECT state='confirmed' AND evidence='lock_and_processes' FROM session_stop_attempts WHERE attempt=?1",[&attempt],|r| r.get(0)).unwrap();
                assert!(confirmed);
            }
            let expected = if entry == Entry::Disconnect { "session/load" } else { "session/new" };
            until(|| methods().lines().any(|method| method == expected));
            if entry == Entry::SameVersion {
                assert_eq!(count(),1);
                assert!(!original.has_exited().unwrap());
            }
            send(&mut stream,end()).await;
            ended(&mut stream).await;
        });
        child.terminate();
    });
}
