//! Node allowlists must survive actual ACP creation, restore, rebuild, and plugin wakeups.

use super::{
    acp_calls, agent_ref, current_thread_runtime, drain_prompt, install_fake_opencode_plugin,
    install_mcp_plugin, open_ready_backend, seed_workspace,
};
use crate::setup::DesktopTestSetup;
use agent_client_protocol_schema::v1::{ContentBlock, TextContent};
use ora_backend::{Backend, BackendError};
use ora_contracts::*;
use pretty_assertions::assert_eq;
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

mod recovery;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Isolates the fixture from user rules such as global ignores and commit signing.
fn fixture_git(worktree: &Path, args: &[&str]) -> io::Result<String> {
    let config = worktree.with_file_name("gitconfig");
    let output = Command::new("git")
        .args(args)
        .env("GIT_CONFIG_GLOBAL", &config)
        .env("GIT_CONFIG_SYSTEM", &config)
        .current_dir(worktree)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    String::from_utf8(output.stdout).map_err(io::Error::other)
}

/// Gives workflow checkpoints a real checkout with both tracked edits and untracked work.
fn dirty_worktree(setup: &DesktopTestSetup) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let worktree = setup.root().join("workspace");
    let tracked = worktree.join("tracked.txt");
    fs::write(worktree.with_file_name("gitconfig"), "")?;
    fs::write(&tracked, "committed contents")?;
    fixture_git(&worktree, &["init"])?;
    fixture_git(&worktree, &["add", "tracked.txt"])?;
    fixture_git(
        &worktree,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.test",
            "commit",
            "-m",
            "initial",
        ],
    )?;
    fs::write(tracked, "unsaved tracked edit")?;
    fs::write(worktree.join("untracked.txt"), "unsaved untracked work")?;
    Ok(worktree)
}

/// Produces two independent nodes, one with a mixed allowlist and one with no MCP bindings.
fn graph() -> String {
    let agent = |id, mcps| {
        json!({"id": id, "data": {"kind": "agent", "agentConfig": {
            "executor": {"agentCli": "official/ora-space.opencode", "modelId": "anthropic/claude-sonnet-4"},
            "prompt": "hello", "interactive": true, "schemaVersion": 4, "completionSignal": "agent", "mcps": mcps
        }}})
    };
    json!({"nodes": [
        {"id": "start", "data": {"kind": "start"}},
        agent("left", json!([{"mcpId": "official/first", "enabled": true}, {"mcpId": "official/second", "enabled": false}])),
        agent("right", json!([]))
    ], "edges": [{"source": "start", "target": "left"}, {"source": "start", "target": "right"}]}).to_string()
}

/// Waits for both first turns to park, reporting node errors instead of hiding setup failures.
async fn parked_nodes(
    backend: &Backend,
    run_id: &str,
) -> Result<Vec<WorkflowNodeRun>, Box<dyn std::error::Error>> {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let detail = backend.workflow_runs().get(GetWorkflowRunRequest {
                run_id: run_id.to_string(),
            })?;
            if let Some(error) = detail.nodes.iter().find_map(|node| node.error.as_ref()) {
                return Err(std::io::Error::other(error.clone()).into());
            }
            let nodes: Vec<_> = detail
                .nodes
                .into_iter()
                .filter(|node| node.node_type == "agent")
                .collect();
            if nodes.len() == 2
                && nodes.iter().all(|node| {
                    node.status == WorkflowNodeStatus::Pending && node.session_id.is_some()
                })
            {
                return Ok(nodes);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?
}

/// Reads recorded ACP requests. A missing journal means no request has been recorded yet.
fn mcp_calls(package_root: &Path) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let journal = match fs::read_to_string(package_root.join("mcp_calls.jsonl")) {
        Ok(journal) => journal,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => return Err(error.into()),
    };
    Ok(journal
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?)
}

/// Sends a human continuation through the workflow-owned session's ordinary prompt boundary.
async fn follow_up(backend: &Backend, session_id: &str) -> TestResult {
    drain_prompt(backend.sessions().prompt(PromptSessionRequest {
        session_id: session_id.to_string(),
        model: None,
        record_prompt: None,
        prompt: vec![ContentBlock::Text(TextContent::new("continue"))],
    }))
    .await
}

/// The frozen choice, including an empty choice, must win over all installed plugins on every path.
#[test]
fn workflow_mcp_allowlists_survive_restore_rebuild_and_refresh() -> TestResult {
    ora_logging::with_trace_logging(|| {
        current_thread_runtime()?.block_on(async {
        let setup = DesktopTestSetup::new()?;
        let home = &setup.backend_paths().home_directory;
        let package_root = install_fake_opencode_plugin(home)?;
        let first = install_mcp_plugin(home, "first")?;
        install_mcp_plugin(home, "second")?;
        let backend = open_ready_backend(&setup)?;
        let workspace_id = seed_workspace(&setup, &backend)?;
        // Interactive completion snapshots require a committed checkout.
        for args in [vec!["init"], vec!["-c", "user.name=Test", "-c", "user.email=test@example.test", "commit", "--allow-empty", "-m", "initial"]] {
            let output = std::process::Command::new("git").args(args).current_dir(setup.root().join("workspace")).output()?;
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        }
        let workflow = backend.workflows().create(CreateWorkflowRequest { name: "MCP workflow".into(), graph: Some(graph()) })?;
        backend.workflows().publish(PublishWorkflowRequest { workflow_id: workflow.workflow.id.clone(), version: Some("v1".into()) })?;
        let run = backend.workflow_runs().create(CreateWorkflowRunRequest {
            workspace_id: workspace_id.clone(), workflow_id: workflow.workflow.id.clone(), locale: WorkflowRunLocale::EnUs,
            snapshot_id: None, kickoff_input: None, name: None, inject_last_failure: None,
        })?.run;
        backend.workflow_runs().start(StartWorkflowRunRequest { run_id: run.id.clone() })?;
        let nodes = parked_nodes(&backend, &run.id).await?;
        let left = nodes.iter().find(|node| node.node_id == "left").and_then(|node| node.session_id.as_ref()).ok_or("left session missing")?;
        let calls = mcp_calls(&package_root)?;
        assert_eq!(calls.len(), 2);
        let left_provider = calls.iter().find(|call| call["servers"] == json!(["official/first", "ora-workflow-completion"])).ok_or("left MCP selection missing")?["sessionId"].clone();
        let right_provider = calls.iter().find(|call| call["servers"] == json!(["ora-workflow-completion"])).ok_or("right MCP selection not empty")?["sessionId"].clone();

        let ordinary = backend.sessions().start(StartSessionRequest { workspace_id, agent_ref: agent_ref(), model: None }).await?;
        assert_eq!(mcp_calls(&package_root)?.last().ok_or("MCP journal is empty")?["servers"], json!(["official/first", "official/second"]));
        backend.workflows().update_draft(UpdateDraftRequest { workflow_id: workflow.workflow.id, graph: graph().replace("official/first", "official/second") })?;
        backend.sessions().stop(StopSessionRequest { session_id: left.clone() }).await?;
        follow_up(&backend, left).await?;
        parked_nodes(&backend, &run.id).await?;
        assert_eq!(mcp_calls(&package_root)?.last().ok_or("MCP journal is empty")?, &json!({"method": "session/load", "sessionId": left_provider, "servers": ["official/first", "ora-workflow-completion"]}));

        fs::write(package_root.join("refuse_session_load"), "refuse")?;
        backend.sessions().stop(StopSessionRequest { session_id: left.clone() }).await?;
        follow_up(&backend, left).await?;
        parked_nodes(&backend, &run.id).await?;
        let calls = mcp_calls(&package_root)?;
        assert_eq!(calls[calls.len()-2]["method"], "session/load");
        assert_eq!(calls.last().ok_or("MCP journal is empty")?["method"], "session/new");
        assert_eq!(calls.last().ok_or("MCP journal is empty")?["servers"], json!(["official/first", "ora-workflow-completion"]));
        let rebuilt_provider = calls.last().ok_or("MCP journal is empty")?["sessionId"].clone();
        fs::remove_file(package_root.join("refuse_session_load"))?;

        // An actual selected package revision wakes the live actor, still using its frozen IDs.
        let manifest = first.join("orax.toml");
        fs::write(&manifest, fs::read_to_string(&manifest)?.replace("1.0.0", "1.0.1"))?;
        let updated = first.with_file_name("1.0.1");
        fs::rename(&first, &updated)?;
        backend.plugins().scan(ScanPluginsRequest {}).await?;
        // The scan wakes the live actor into an MCP refresh that holds the session exclusively;
        // a prompt reaching it mid-refresh is answered with SessionBusy while the parked node
        // rolls back to Pending. Waiting for the refresh's own journal entry closes the wide
        // window, and a bounded retry settles the instant where the refresh response and the
        // queued prompt are ready together inside the actor's select.
        tokio::time::timeout(Duration::from_secs(15), async {
            while !mcp_calls(&package_root).is_ok_and(|calls| {
                calls.iter().any(|call| {
                    call["method"] == "session/load"
                        && call["sessionId"] == rebuilt_provider
                        && call["servers"] == json!(["official/first", "ora-workflow-completion"])
                })
            }) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
        for attempt in 0..5 {
            match follow_up(&backend, left).await {
                Ok(()) => break,
                Err(error)
                    if attempt < 4
                        && error
                            .downcast_ref::<BackendError>()
                            .is_some_and(|error| {
                                matches!(error.public_error(), PublicError::SessionBusy(_))
                            }) =>
                {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(error) => return Err(error),
            }
        }
        parked_nodes(&backend, &run.id).await?;
        let left_provider = rebuilt_provider;
        assert!(mcp_calls(&package_root)?.iter().any(|call| call["method"] == "session/load" && call["sessionId"] == left_provider && call["servers"] == json!(["official/first", "ora-workflow-completion"])));
        assert!(mcp_calls(&package_root)?.iter().filter(|call| call["sessionId"] == right_provider).all(|call| call["servers"] == json!(["ora-workflow-completion"])));
        backend.workflow_runs().cancel(CancelWorkflowRunRequest { run_id: run.id }).await?;
        backend.sessions().stop(StopSessionRequest { session_id: ordinary.session.id }).await?;
        Ok(())
    })
    })
}

/// Polls a run until it is terminal, returning the last public detail.
async fn terminal(
    backend: &Backend,
    run_id: &str,
) -> Result<GetWorkflowRunResponse, Box<dyn std::error::Error>> {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let detail = backend.workflow_runs().get(GetWorkflowRunRequest {
                run_id: run_id.to_string(),
            })?;
            if matches!(
                detail.run.status,
                WorkflowRunStatus::Succeeded
                    | WorkflowRunStatus::Failed
                    | WorkflowRunStatus::Cancelled
            ) {
                return Ok(detail);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?
}

/// One agent node that binds the MCP plugin and completes on its own.
fn single_agent_graph() -> Value {
    json!({"nodes": [
        {"id": "start", "data": {"kind": "start"}},
        {"id": "first", "data": {"kind": "agent", "title": "First step", "agentConfig": {
            "retry": {"enabled": false, "maxRetries": 0, "initialDelaySeconds": 0},
            "executor": {"agentCli": "official/ora-space.opencode", "modelId": "anthropic/claude-sonnet-4"},
            "prompt": "hello", "mcps": [{"mcpId": "official/first", "enabled": true}]
        }}}
    ], "edges": [{"source": "start", "target": "first"}]})
}

/// Puts a dispatchable agent before the MCP-dependent node so a leaked dispatch can reach ACP.
fn graph_with_unbound_entry() -> Value {
    let mut graph = single_agent_graph();
    let mut entry = graph["nodes"][1].clone();
    entry["id"] = json!("entry");
    entry["data"]["agentConfig"]["mcps"] = json!([]);
    graph["nodes"] = json!([graph["nodes"][0], entry, graph["nodes"][1]]);
    graph["edges"] = json!([
        {"source": "start", "target": "entry"},
        {"source": "entry", "target": "first"}
    ]);
    graph
}

/// The routing agent selects the same fallback whether the review is a branch or a whole loop.
fn graph_with_skipped_review() -> [Value; 2] {
    let base = single_agent_graph();
    let mut router = base["nodes"][1].clone();
    router["id"] = json!("router");
    router["data"]["agentConfig"]["prompt"] = json!("FAKE_REPLY: skip");
    router["data"]["agentConfig"]["mcps"] = json!([]);
    let mut nodes = vec![
        base["nodes"][0].clone(),
        router,
        json!({"id": "gate", "data": {"kind": "condition", "cases": [
            {"id": "review", "logic": "and", "conditions": [
                {"variableSelector": ["router", "output"], "operator": "equals", "value": "review"}
            ]}
        ]}}),
        json!({"id": "fallback", "data": {"kind": "output", "outputs": [
            {"name": "result", "variableSelector": ["router", "output"]}
        ]}}),
    ];
    let mut edges = vec![
        json!({"source": "start", "target": "router"}),
        json!({"source": "router", "target": "gate"}),
        json!({"source": "gate", "sourceHandle": "else", "target": "fallback"}),
    ];
    let mut reviewer = base["nodes"][1].clone();
    reviewer["id"] = json!("reviewer");
    let mut branch_nodes = nodes.clone();
    branch_nodes.push(reviewer.clone());
    let mut branch_edges = edges.clone();
    branch_edges.push(json!({"source": "gate", "sourceHandle": "review", "target": "reviewer"}));
    let branch = json!({"nodes": branch_nodes, "edges": branch_edges});

    reviewer["data"]["containerId"] = json!("loop");
    nodes.extend([
        json!({"id": "loop", "data": {"kind": "loop", "loopConfig": {
            "maxIterations": 1, "variables": [], "until": {"logic": "and", "conditions": []},
            "outputs": [{"name": "result", "variableSelector": ["reviewer", "output"]}]
        }}}),
        json!({"id": "entry", "data": {"kind": "start", "containerId": "loop"}}),
        reviewer,
    ]);
    edges.extend([
        json!({"source": "gate", "sourceHandle": "review", "target": "loop"}),
        json!({"source": "entry", "target": "reviewer"}),
    ]);
    [
        branch,
        json!({"schemaVersion": 2, "nodes": nodes, "edges": edges}),
    ]
}

/// A chain whose first agent binds the MCP plugin and whose second step fails on its first
/// prompt, so the run settles as `Failed` with a `Succeeded` predecessor.
fn failing_chain() -> Value {
    let agent = |id: &str, title: &str, prompt: &str, mcps: Value| {
        json!({"id": id, "data": {"kind": "agent", "title": title, "agentConfig": {
            "retry": {"enabled": false, "maxRetries": 0, "initialDelaySeconds": 0},
            "executor": {"agentCli": "official/ora-space.opencode", "modelId": "anthropic/claude-sonnet-4"},
            "prompt": prompt, "mcps": mcps
        }}})
    };
    json!({"nodes": [
        {"id": "start", "data": {"kind": "start"}},
        agent("first", "First step", "hello", json!([{"mcpId": "official/first", "enabled": true}])),
        agent("second", "Second step", "[fail-times:1] hello", json!([]))
    ], "edges": [{"source": "start", "target": "first"}, {"source": "first", "target": "second"}]})
}

/// Installs one MCP package that declares a required setting with no value: installed enough to
/// schedule, not configured enough to open a session.
fn install_unconfigured_mcp_plugin(home: &Path, name: &str) -> std::io::Result<PathBuf> {
    let root = home
        .join("plugins")
        .join("installed")
        .join("official")
        .join(name)
        .join("1.0.0");
    fs::create_dir_all(root.join("assets"))?;
    fs::write(
        root.join("orax.toml"),
        format!(
            "resolver = 1\nidentifier = \"{name}\"\nkind = \"mcp\"\nversion = \"1.0.0\"\ndescription = \"Unconfigured MCP fixture\"\n"
        ),
    )?;
    let command = root.join("assets").join("server");
    fs::write(&command, "#!/bin/sh\n")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&command, fs::Permissions::from_mode(0o755))?;
    }
    fs::write(
        root.join("assets").join("config.json"),
        json!({
            "schemaVersion": 1,
            "settings": {"apiKey": {"type": "string", "title": "API key", "description": "Required credential", "required": true}},
            "transport": {"type": "stdio", "command": "assets/server", "args": [], "env": {}}
        })
        .to_string(),
    )?;
    Ok(root)
}

/// Publishes one graph and creates a pending run the way the run screen does.
fn pending_run(
    backend: &Backend,
    workspace_id: &str,
    graph: &Value,
) -> Result<WorkflowRun, Box<dyn std::error::Error>> {
    let workflow_id = publish_graph(backend, "MCP preflight", graph)?;
    create_pending_run(backend, workspace_id, &workflow_id)
}

/// Publishes a named graph so control and rejection runs can share the same snapshot.
fn publish_graph(backend: &Backend, name: &str, graph: &Value) -> Result<String, BackendError> {
    let workflow = backend.workflows().create(CreateWorkflowRequest {
        name: name.into(),
        graph: Some(graph.to_string()),
    })?;
    backend.workflows().publish(PublishWorkflowRequest {
        workflow_id: workflow.workflow.id.clone(),
        version: Some("v1".into()),
    })?;
    Ok(workflow.workflow.id)
}

/// Freezes the latest published snapshot of the workflow in a pending run.
fn create_pending_run(
    backend: &Backend,
    workspace_id: &str,
    workflow_id: &str,
) -> Result<WorkflowRun, Box<dyn std::error::Error>> {
    Ok(backend
        .workflow_runs()
        .create(CreateWorkflowRunRequest {
            workspace_id: workspace_id.to_string(),
            workflow_id: workflow_id.to_string(),
            locale: WorkflowRunLocale::EnUs,
            snapshot_id: None,
            kickoff_input: None,
            name: None,
            inject_last_failure: None,
        })?
        .run)
}

/// Removes one installed MCP package through the plugin use case, as the plugin market does.
async fn uninstall_mcp(backend: &Backend, plugin_id: &str) -> TestResult {
    backend
        .plugins()
        .uninstall(UninstallPluginRequest {
            plugin_id: plugin_id.to_string(),
            data_disposition: PluginDataDisposition::Delete,
            hook_execution_acknowledged: false,
        })
        .await?;
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum WorktreeEntry {
    Directory,
    File(Vec<u8>),
    Symlink(PathBuf),
}

/// Captures workspace paths and bytes independently of Git's index and ignore rules.
/// Git refs are captured separately so changes to a checkpoint cannot hide inside `.git`.
fn worktree_contents(worktree: &Path) -> std::io::Result<BTreeMap<PathBuf, WorktreeEntry>> {
    let mut contents = BTreeMap::new();
    let mut directories = vec![PathBuf::new()];
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(worktree.join(&directory))? {
            let entry = entry?;
            let relative = directory.join(entry.file_name());
            if relative == Path::new(".git") {
                continue;
            }
            let file_type = entry.file_type()?;
            let content = if file_type.is_dir() {
                directories.push(relative.clone());
                WorktreeEntry::Directory
            } else if file_type.is_symlink() {
                WorktreeEntry::Symlink(fs::read_link(entry.path())?)
            } else if file_type.is_file() {
                WorktreeEntry::File(fs::read(entry.path())?)
            } else {
                return Err(std::io::Error::other(format!(
                    "unexpected worktree entry: {}",
                    relative.display()
                )));
            };
            contents.insert(relative, content);
        }
    }
    Ok(contents)
}

/// Observations at the scheduler, persistence, ACP and filesystem boundaries of a dispatch.
#[derive(Debug, PartialEq)]
struct DispatchSnapshot {
    detail: GetWorkflowRunResponse,
    sessions: Vec<Vec<SqlValue>>,
    acp_calls: Vec<String>,
    worktree: BTreeMap<PathBuf, WorktreeEntry>,
    git_refs: String,
}

impl DispatchSnapshot {
    /// Reads the Session table directly because the session-list operation hides workflow sessions.
    fn capture(
        backend: &Backend,
        database: &Connection,
        package_root: &Path,
        worktree: &Path,
        run_id: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let detail = backend.workflow_runs().get(GetWorkflowRunRequest {
            run_id: run_id.to_string(),
        })?;
        let sessions = database
            .prepare("SELECT * FROM sessions ORDER BY id")?
            .query_map([], |row| {
                (0..row.as_ref().column_count())
                    .map(|column| row.get(column))
                    .collect()
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Self {
            detail,
            sessions,
            acp_calls: acp_calls(package_root),
            worktree: worktree_contents(worktree)?,
            git_refs: fixture_git(
                worktree,
                &[
                    "for-each-ref",
                    "--format=%(refname) %(objectname) %(symref)",
                ],
            )?,
        })
    }
}

/// Opens the real database owned by this E2E fixture without creating or modifying it.
fn session_database(setup: &DesktopTestSetup) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(
        setup.backend_paths().app_data_directory.join("ora.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
}

/// Gives a spawned dispatch one second to reach Session/ACP or change the worktree.
async fn assert_dispatch_unchanged(
    backend: &Backend,
    database: &Connection,
    package_root: &Path,
    worktree: &Path,
    before: &DispatchSnapshot,
) -> TestResult {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    loop {
        assert_eq!(
            &DispatchSnapshot::capture(
                backend,
                database,
                package_root,
                worktree,
                &before.detail.run.id,
            )?,
            before
        );
        if tokio::time::Instant::now() >= deadline {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Preflight still checks the bindings on a branch and in a loop skipped at execution time.
#[test]
fn bindings_inside_skipped_branches_and_loops_block_start() -> TestResult {
    current_thread_runtime()?.block_on(async {
        let setup = DesktopTestSetup::new()?;
        let home = &setup.backend_paths().home_directory;
        install_fake_opencode_plugin(home)?;
        install_mcp_plugin(home, "first")?;
        let backend = open_ready_backend(&setup)?;
        let workspace_id = seed_workspace(&setup, &backend)?;
        let mut pending = vec![];

        for (name, graph) in ["Conditional review", "Conditional loop"]
            .into_iter()
            .zip(graph_with_skipped_review())
        {
            let workflow_id = publish_graph(&backend, name, &graph)?;
            let control = create_pending_run(&backend, &workspace_id, &workflow_id)?;
            backend.workflow_runs().start(StartWorkflowRunRequest {
                run_id: control.id.clone(),
            })?;
            let detail = terminal(&backend, &control.id).await?;
            assert_eq!(
                detail.run.status,
                WorkflowRunStatus::Succeeded,
                "{detail:?}"
            );
            assert_eq!(
                detail.condition_decisions.get("gate").map(String::as_str),
                Some("else")
            );
            assert!(
                !detail
                    .nodes
                    .iter()
                    .any(|node| node.node_id == "reviewer" || node.node_id == "loop"),
                "the conditional fallback must bypass the review and any enclosing loop"
            );
            let run = create_pending_run(&backend, &workspace_id, &workflow_id)?;
            assert_eq!(run.snapshot_id, control.snapshot_id);
            pending.push((name, run));
        }

        uninstall_mcp(&backend, "official/first").await?;
        for (name, run) in pending {
            let error = backend
                .workflow_runs()
                .start(StartWorkflowRunRequest { run_id: run.id })
                .err()
                .ok_or_else(|| format!("{name} must preflight the skipped reviewer"))?;
            assert_eq!(
                error.public_error(),
                &PublicError::WorkflowMcpNotFound(Box::new(WorkflowMcpNotFoundParams {
                    node_id: "reviewer".into(),
                    node_title: "First step".into(),
                    mcp_id: "official/first".into(),
                }))
            );
        }
        Ok(())
    })
}

/// The rejected start must not dispatch its unbound first agent.
#[test]
fn a_rejected_start_leaves_no_node_run_session_or_acp_call() -> TestResult {
    current_thread_runtime()?.block_on(async {
        let setup = DesktopTestSetup::new()?;
        let home = &setup.backend_paths().home_directory;
        let package_root = install_fake_opencode_plugin(home)?;
        install_mcp_plugin(home, "first")?;
        let backend = open_ready_backend(&setup)?;
        let database = session_database(&setup)?;
        let workspace_id = seed_workspace(&setup, &backend)?;
        let worktree = dirty_worktree(&setup)?;
        let run = pending_run(&backend, &workspace_id, &graph_with_unbound_entry())?;
        uninstall_mcp(&backend, "official/first").await?;
        let before =
            DispatchSnapshot::capture(&backend, &database, &package_root, &worktree, &run.id)?;
        assert!(before.detail.nodes.is_empty());
        assert!(before.sessions.is_empty());
        assert!(before.acp_calls.is_empty());

        let result = backend
            .workflow_runs()
            .start(StartWorkflowRunRequest { run_id: run.id });
        assert_dispatch_unchanged(&backend, &database, &package_root, &worktree, &before).await?;
        let error = result.err().ok_or("missing MCP must prevent start")?;
        assert_eq!(error.public_error().code(), "workflow_mcp_not_found");
        Ok(())
    })
}

/// A finished run whose MCP disappeared cannot restart, and the refusal dispatches nothing.
#[test]
fn a_succeeded_run_cannot_restart_after_its_mcp_plugin_is_gone() -> TestResult {
    current_thread_runtime()?.block_on(async {
        let setup = DesktopTestSetup::new()?;
        let home = &setup.backend_paths().home_directory;
        let package_root = install_fake_opencode_plugin(home)?;
        install_mcp_plugin(home, "first")?;
        let backend = open_ready_backend(&setup)?;
        let database = session_database(&setup)?;
        let workspace_id = seed_workspace(&setup, &backend)?;
        let worktree = dirty_worktree(&setup)?;
        let run = pending_run(&backend, &workspace_id, &graph_with_unbound_entry())?;
        backend.workflow_runs().start(StartWorkflowRunRequest {
            run_id: run.id.clone(),
        })?;
        let succeeded = terminal(&backend, &run.id).await?;
        assert_eq!(
            succeeded.run.status,
            WorkflowRunStatus::Succeeded,
            "run={:?} nodes={:?}",
            succeeded.run.error,
            succeeded
                .nodes
                .iter()
                .map(|node| (&node.node_id, node.status, &node.error))
                .collect::<Vec<_>>()
        );

        uninstall_mcp(&backend, "official/first").await?;
        let before =
            DispatchSnapshot::capture(&backend, &database, &package_root, &worktree, &run.id)?;
        assert!(!before.sessions.is_empty());
        assert!(
            before
                .acp_calls
                .iter()
                .any(|call| call.starts_with("session/prompt "))
        );

        let result = backend
            .workflow_runs()
            .restart(RestartWorkflowRunRequest { run_id: run.id });
        assert_dispatch_unchanged(&backend, &database, &package_root, &worktree, &before).await?;
        let error = result
            .err()
            .ok_or("a restart of a finished run must be refused")?;
        assert_eq!(error.public_error().code(), "workflow_mcp_not_found");
        Ok(())
    })
}

/// A failed run cannot resume around an MCP its already-succeeded node needs, because the whole
/// frozen graph stays the deployment unit.
#[test]
fn a_failed_run_cannot_resume_around_a_succeeded_nodes_mcp() -> TestResult {
    current_thread_runtime()?.block_on(async {
        let setup = DesktopTestSetup::new()?;
        let home = &setup.backend_paths().home_directory;
        let package_root = install_fake_opencode_plugin(home)?;
        install_mcp_plugin(home, "first")?;
        let backend = open_ready_backend(&setup)?;
        let database = session_database(&setup)?;
        let workspace_id = seed_workspace(&setup, &backend)?;
        let worktree = dirty_worktree(&setup)?;
        let run = pending_run(&backend, &workspace_id, &failing_chain())?;
        backend.workflow_runs().start(StartWorkflowRunRequest {
            run_id: run.id.clone(),
        })?;
        let failed = terminal(&backend, &run.id).await?;
        assert_eq!(failed.run.status, WorkflowRunStatus::Failed);
        assert_eq!(
            failed
                .nodes
                .iter()
                .find(|node| node.node_id == "first")
                .map(|node| node.status),
            Some(WorkflowNodeStatus::Succeeded)
        );

        uninstall_mcp(&backend, "official/first").await?;
        let before =
            DispatchSnapshot::capture(&backend, &database, &package_root, &worktree, &run.id)?;
        assert!(!before.sessions.is_empty());
        assert!(
            before
                .acp_calls
                .iter()
                .any(|call| call.starts_with("session/prompt "))
        );

        let result = backend
            .workflow_runs()
            .resume_from_failure(ResumeWorkflowRunRequest {
                run_id: run.id,
                rollback: None,
                snapshot_id: None,
            });
        assert_dispatch_unchanged(&backend, &database, &package_root, &worktree, &before).await?;
        let error = result
            .err()
            .ok_or("a resume must not route around a removed MCP")?;
        assert_eq!(error.public_error().code(), "workflow_mcp_not_found");
        Ok(())
    })
}

/// An installed but unconfigured MCP passes the scheduling precheck and then fails the node's
/// session setup, so one run shows both sides of the boundary.
#[test]
fn an_incomplete_mcp_passes_precheck_and_fails_the_node_session() -> TestResult {
    current_thread_runtime()?.block_on(async {
        let setup = DesktopTestSetup::new()?;
        let home = &setup.backend_paths().home_directory;
        let package_root = install_fake_opencode_plugin(home)?;
        install_unconfigured_mcp_plugin(home, "first")?;
        let backend = open_ready_backend(&setup)?;
        let workspace_id = seed_workspace(&setup, &backend)?;
        let run = pending_run(&backend, &workspace_id, &single_agent_graph())?;

        let started = backend.workflow_runs().start(StartWorkflowRunRequest {
            run_id: run.id.clone(),
        })?;
        assert_eq!(started.run.status, WorkflowRunStatus::Running);

        let failed = terminal(&backend, &run.id).await?;
        assert_eq!(failed.run.status, WorkflowRunStatus::Failed);
        let node = failed
            .nodes
            .iter()
            .find(|node| node.node_id == "first")
            .ok_or("the node row is missing")?;
        let error = node
            .error
            .as_deref()
            .ok_or("the node did not report an error")?;
        assert!(
            error.contains("configuration is incomplete"),
            "the node must fail the session setup for its incomplete configuration: {error}"
        );
        assert!(node.session_id.is_none(), "no Session may be delivered");
        assert_eq!(
            mcp_calls(&package_root)?.len(),
            0,
            "an incomplete selection must never reach ACP"
        );
        Ok(())
    })
}

/// The package can disappear between the precheck and the first dispatch: the node session setup
/// then refuses it, so nothing runs with a partial selection.
#[test]
fn uninstalling_between_precheck_and_dispatch_fails_the_node_session() -> TestResult {
    current_thread_runtime()?.block_on(async {
        let setup = DesktopTestSetup::new()?;
        let home = &setup.backend_paths().home_directory;
        let package_root = install_fake_opencode_plugin(home)?;
        install_mcp_plugin(home, "first")?;
        let backend = open_ready_backend(&setup)?;
        let workspace_id = seed_workspace(&setup, &backend)?;
        let run = pending_run(&backend, &workspace_id, &single_agent_graph())?;

        let started = backend.workflow_runs().start(StartWorkflowRunRequest {
            run_id: run.id.clone(),
        })?;
        assert_eq!(started.run.status, WorkflowRunStatus::Running);
        // On this current-thread runtime, uninstall removes the package before its first yield
        // lets the dispatch spawned by start resolve the MCP selection.
        uninstall_mcp(&backend, "official/first").await?;

        let failed = terminal(&backend, &run.id).await?;
        assert_eq!(failed.run.status, WorkflowRunStatus::Failed);
        let node = failed
            .nodes
            .iter()
            .find(|node| node.node_id == "first")
            .ok_or("the node row is missing")?;
        let error = node
            .error
            .as_deref()
            .ok_or("the node did not report an error")?;
        assert!(
            error.contains("selected MCP plugin `official/first` is not installed"),
            "the node must fail closed as its selected plugin disappeared: {error}"
        );
        assert!(node.session_id.is_none(), "no Session may be delivered");
        assert_eq!(
            mcp_calls(&package_root)?.len(),
            0,
            "a missing selection must never reach ACP"
        );
        Ok(())
    })
}
