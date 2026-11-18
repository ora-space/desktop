//! Host MCP URLs and headers never reach SQLite; the durable input keeps only server names.
use super::*;
use pretty_assertions::assert_eq;

const MARKER: &str = "m1264-host-value";

/// One host server whose URL and header both carry `marker`.
fn server(name: &str, marker: &str) -> HostMcpServer {
    HostMcpServer {
        name: name.into(),
        url: format!("http://127.0.0.1:41234/mcp/{marker}"),
        headers: [("authorization".to_owned(), format!("Bearer {marker}"))]
            .into_iter()
            .collect(),
    }
}

/// The base command with two host servers.
fn with_host_servers(marker: &str) -> StartAgentSessionMessage {
    let mut start = command();
    start.payload.spec.host_mcp_servers = vec![server("ora-a", marker), server("ora-b", marker)];
    start
}

/// Reads every file the database owns, including the WAL and shared-memory files.
fn stored_bytes(dir: &std::path::Path) -> Vec<u8> {
    std::fs::read_dir(dir)
        .unwrap()
        .flat_map(|entry| std::fs::read(entry.unwrap().path()).unwrap())
        .collect()
}

/// The raw stored input of the base execution.
fn stored_input(db: &NodeDatabase) -> serde_json::Value {
    let input: String = db
        .connection
        .query_row(
            "SELECT input FROM node_executions WHERE execution=?1",
            [command().execution_id.as_str()],
            |row| row.get(/*idx*/ 0),
        )
        .unwrap();
    serde_json::from_str(&input).unwrap()
}

/// Accepting, starting and reopening keep names only, in a shape distinct from the wire field.
#[test]
fn host_mcp_values_never_persist_and_names_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let start = with_host_servers(MARKER);
    let mut db = NodeDatabase::open(&path, NodeIdentity::Require(NodeId::new("node"))).unwrap();
    let accepted = db.accept_session(&start).unwrap();
    assert_eq!(
        accepted,
        SessionExecution {
            command: command(),
            host_mcp_server_names: vec!["ora-a".into(), "ora-b".into()],
            state: ExecutionState::Accepted,
            last_sequence: 0,
        }
    );
    assert!(
        db.start_session(&start, &NodeIncarnationId::new("first"))
            .unwrap()
    );
    assert_eq!(
        stored_input(&db)["payload"]["spec"]
            .as_object()
            .unwrap()
            .iter()
            .filter(|(key, _)| key.starts_with("host_mcp"))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<Vec<_>>(),
        vec![(
            "host_mcp_server_names".to_owned(),
            serde_json::json!(["ora-a", "ora-b"])
        )]
    );
    let bytes = stored_bytes(dir.path());
    assert!(
        !bytes
            .windows(MARKER.len())
            .any(|window| window == MARKER.as_bytes())
    );
    drop(db);
    let db = NodeDatabase::open(&path, NodeIdentity::Require(NodeId::new("node"))).unwrap();
    assert_eq!(
        db.recoverable_sessions().unwrap(),
        vec![SessionExecution {
            command: command(),
            host_mcp_server_names: vec!["ora-a".into(), "ora-b".into()],
            state: ExecutionState::Running,
            last_sequence: 0,
        }]
    );
}

/// A repeated start compares names only: other values are ignored, other names conflict.
#[test]
fn host_mcp_duplicate_start_compares_names_only() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = NodeDatabase::open(
        &dir.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
    )
    .unwrap();
    let first = db.accept_session(&with_host_servers("first")).unwrap();
    assert_eq!(
        db.accept_session(&with_host_servers("second")).unwrap(),
        first
    );
    assert!(
        db.start_session(&with_host_servers("third"), &NodeIncarnationId::new("i"))
            .unwrap()
    );
    let mut renamed = with_host_servers("first");
    renamed.payload.spec.host_mcp_servers[1].name = "ora-c".into();
    assert!(matches!(
        db.accept_session(&renamed),
        Err(Error::IdentityConflict)
    ));
    let mut fewer = with_host_servers("first");
    fewer.payload.spec.host_mcp_servers.pop();
    assert!(matches!(
        db.accept_session(&fewer),
        Err(Error::IdentityConflict)
    ));
}

/// Sessions without host servers write no names, so their rows equal those of older Nodes.
#[test]
fn host_mcp_absent_names_are_not_written() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = NodeDatabase::open(
        &dir.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
    )
    .unwrap();
    db.accept_session(&command()).unwrap();
    assert_eq!(stored_input(&db), serde_json::to_value(command()).unwrap());
    assert_eq!(
        db.find_session(&command().operation_id, &command().execution_id)
            .unwrap()
            .map(|record| record.host_mcp_server_names),
        Some(Vec::new())
    );
}
