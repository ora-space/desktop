//! Turn boundary conformance of one installed release Agent package through the real Node.
//!
//! A verified turn boundary is the Agent's answer together with the Node's turn process fence, so
//! conformance runs through both: the production `ora-node`, the real Deno and the package's own
//! CLI, driven by the loopback model of `tests/turn_boundary/serve.py`. A plain turn comes first,
//! so the Agent's own processes exist before the target turn; the target turn then runs one
//! probe scenario and is cancelled while it runs (`error` fails on its own instead), and a plain
//! turn follows. The run passes only when the stopped turn kept the session, nothing the turn
//! started outside the sessions of the Agent's earlier processes was alive when the stop settled,
//! no marker was written afterwards, and the next turn completed. A failed turn settles no
//! cancel, so for `error` the processes are sampled at the next turn's first event, which the
//! Node writes only after it judged the failed turn and ran the fence.
//!
//! The test finds the turn's processes the way the Node does, by the attempt mark, so it shares
//! the mark's blind spot: an orphan that cleared or rewrote its environment is invisible to both.
//! The probe's markers are what catch such a job, when it writes one within the observation.
//!
//! The Node keeps a session only for a version on its turn-boundary list, so measuring a version
//! the list does not name needs it added locally first. Inputs, all required:
//! `ORA_PROBE_PACKAGE` (an extracted, unmodified `.orax`), `ORA_PROBE_AGENT` (`opencode` or
//! `claude`), `ORA_PROBE_SCENARIO`, `ORA_PROBE_DENO` (absolute path) and `ORA_PROBE_OUTPUT` (a
//! new directory for `summary.json` and the Node's log). Python 3.10+ must be on `PATH`.
use super::turn_cancel::cancel;
use super::*;
use crate::support::{ChildGuard, until};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};
use tokio::net::UnixStream;

/// How long after the stop settles a marker written by the turn still counts as a violation; the
/// probe's jobs write three seconds after they start.
const OBSERVE: Duration = Duration::from_secs(6);

/// Reads one required input.
fn input(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is required"))
}

/// The loopback model, serving until its stdin closes.
struct ModelServer {
    child: Child,
    stdin: Option<ChildStdin>,
    /// The Agent environment `probe.py` uses, pointing the Agent at this model.
    environment: Vec<(String, String)>,
}

impl ModelServer {
    fn start(agent: &str, scenario: &str, home: &Path, output: &Path) -> Self {
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("turn_boundary")
            .join("serve.py");
        let mut child = Command::new("python3")
            .arg(script)
            .args(["--agent", agent, "--scenario", scenario, "--home"])
            .arg(home)
            .arg("--output")
            .arg(output)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(fs::File::create(output.join("model.stderr")).unwrap())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let started: serde_json::Value = serde_json::from_str(&line).unwrap();
        let environment = started["environment"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(name, value)| (name.clone(), value.as_str().unwrap().to_owned()))
            .collect();
        Self {
            stdin: child.stdin.take(),
            child,
            environment,
        }
    }
}

impl Drop for ModelServer {
    fn drop(&mut self) {
        drop(self.stdin.take());
        let _ = self.child.wait();
    }
}

/// The package's identifier and version from its manifest.
fn identity(package: &Path) -> (String, String) {
    let manifest = fs::read_to_string(package.join("orax.toml")).unwrap();
    let field = |key: &str| {
        manifest
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once('=')?;
                (name.trim() == key).then(|| value.trim().trim_matches('"').to_owned())
            })
            .unwrap()
    };
    (field("identifier"), field("version"))
}

/// Launches the production Node with the real Deno and the Agent's environment.
fn launch(
    fixture: &Fixture,
    clone: &CloneConfig,
    environment: &[(String, String)],
    log: &Path,
) -> ChildGuard {
    let path = ipc::write_config(fixture, clone, /*frame_timeout_ms*/ 120_000);
    let mut config: ora_node::ServiceConfig =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    config.agent = Some(ora_node::AgentConfig {
        model_proxy: None,
        deno_path: PathBuf::from(input("ORA_PROBE_DENO")),
        ready_timeout_ms: 60_000,
        workload_directory: None,
        ora_git_directory: Some(fixture.path().join("ora-git")),
    });
    config.control.as_mut().unwrap().heartbeat_ms = 1000;
    config.node.session_stop.confirmation_timeout_ms = 30_000;
    fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
    let child = ChildGuard(
        Command::new(env!("CARGO_BIN_EXE_ora-node"))
            .arg(path)
            .env_clear()
            .envs(environment.iter().map(|(name, value)| (name, value)))
            .stdin(Stdio::null())
            .stdout(fs::File::create(log).unwrap())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    until(|| {
        fs::read_to_string(log)
            .unwrap_or_default()
            .contains("Node IPC listening")
    });
    child
}

/// One live process that carries the session attempt's mark.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
struct Marked {
    pid: u32,
    session: u32,
    start_ticks: u64,
    command: String,
}

/// Every live process whose environment lists `attempt` under the attempt mark.
fn marked(attempt: &str) -> Vec<Marked> {
    let entry = "ORA_SESSION_ATTEMPT=";
    let mut found = Vec::new();
    for directory in fs::read_dir("/proc").unwrap().flatten() {
        let Ok(pid) = directory.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let (Ok(environ), Ok(stat), Ok(command)) = (
            fs::read(directory.path().join("environ")),
            fs::read_to_string(directory.path().join("stat")),
            fs::read(directory.path().join("cmdline")),
        ) else {
            continue;
        };
        let carries = environ.split(|byte| *byte == 0).any(|variable| {
            String::from_utf8_lossy(variable)
                .strip_prefix(entry)
                .is_some_and(|value| value.split(':').any(|token| token == attempt))
        });
        let fields: Vec<&str> = stat
            .rsplit_once(')')
            .map_or("", |(_, rest)| rest)
            .split_whitespace()
            .collect();
        if carries && fields.first() != Some(&"Z") {
            found.push(Marked {
                pid,
                session: fields[3].parse().unwrap(),
                start_ticks: fields[19].parse().unwrap(),
                command: String::from_utf8_lossy(&command).replace('\0', " "),
            });
        }
    }
    found
}

/// The Agent session's one attempt, once the session registered it.
fn attempt(fixture: &Fixture) -> Option<String> {
    rusqlite::Connection::open(fixture.config().home_directory.join("ora-node.sqlite3"))
        .ok()?
        .query_row(
            "SELECT attempt FROM session_stop_attempts WHERE execution=?1 AND json_extract(record,'$.kind')='agent'",
            [EXECUTION],
            |row| row.get(0),
        )
        .ok()
}

/// What the Node reported for one turn.
#[derive(Debug, Default, serde::Serialize)]
struct Turn {
    ended: bool,
    settled: Option<TurnCancelOutcome>,
    session_end: Option<(AgentSessionEndReason, Option<String>)>,
    /// Marked processes alive when the stop settled, or when the turn ended without a cancel.
    alive_at_end: Vec<Marked>,
    /// Marked processes alive when the turn's first Thread event arrived.
    alive_at_first_event: Option<Vec<Marked>>,
}

/// Receives until the turn has ended and, with `command` given, its cancel settled, or until
/// the session ended. Events are acknowledged as they arrive.
async fn follow(stream: &mut UnixStream, command: Option<&str>, attempt: &str) -> Turn {
    let mut turn = Turn::default();
    loop {
        match receive(stream).await {
            NodeToControllerMessage::ThreadEvent(event) => {
                if turn.alive_at_first_event.is_none() {
                    turn.alive_at_first_event = Some(marked(attempt));
                }
                send(stream, ack(event.sequence.value())).await;
                if event
                    .payload
                    .record
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    == Some("turnEnded")
                {
                    turn.ended = true;
                }
            }
            NodeToControllerMessage::TurnCancelSettled(event)
                if Some(event.payload.command_id.as_str()) == command =>
            {
                turn.settled = Some(event.payload.outcome);
                turn.alive_at_end = marked(attempt);
            }
            NodeToControllerMessage::AgentSessionEnded(event) => {
                send(stream, ack(event.sequence.value())).await;
                let AgentSessionResult::AgentSessionEnded(ended) = event.payload;
                turn.session_end = Some((ended.reason, ended.detail));
                return turn;
            }
            _ => {}
        }
        // A stop that settled `session_ending` is followed by the session's end.
        if turn.ended
            && (command.is_none()
                || turn
                    .settled
                    .is_some_and(|outcome| outcome != TurnCancelOutcome::SessionEnding))
        {
            if command.is_none() {
                turn.alive_at_end = marked(attempt);
            }
            return turn;
        }
    }
}

/// Submits one user turn.
fn submit(turn: &str, text: &str) -> ControllerToNodeMessage {
    ControllerToNodeMessage::SubmitUserTurn(SubmitUserTurnMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: start("").operation_id,
        execution_id: ExecutionId::new(EXECUTION),
        payload: SubmitUserTurn {
            mcp_generation: None,
            node_id: NodeId::new("test-node"),
            command_id: CommandId::new(format!("{turn}-command")),
            turn: UserTurn {
                turn_id: TurnId::new(turn),
                content: vec![ContentBlock::Text { text: text.into() }],
            },
        },
    })
}

#[test]
#[ignore = "runs an installed release Agent package; see the module documentation"]
fn turn_boundary_conformance() {
    ora_logging::with_trace_logging(|| {
        let package = PathBuf::from(input("ORA_PROBE_PACKAGE"));
        let (agent, scenario) = (input("ORA_PROBE_AGENT"), input("ORA_PROBE_SCENARIO"));
        let output = PathBuf::from(input("ORA_PROBE_OUTPUT"));
        fs::create_dir(&output).unwrap();
        let (identifier, version) = identity(&package);
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        prepare(&fixture, &config, &server);
        let installed = fixture
            .config()
            .home_directory
            .join("plugins")
            .join("installed")
            .join("official")
            .join(&identifier)
            .join(&version);
        fs::create_dir_all(installed.parent().unwrap()).unwrap();
        assert!(
            Command::new("cp")
                .arg("-a")
                .arg(&package)
                .arg(&installed)
                .status()
                .unwrap()
                .success()
        );
        let checkout = PathBuf::from(checkout(&fixture).path.as_str());
        let home = fixture.path().join("agent-home");
        fs::create_dir(&home).unwrap();
        let model = ModelServer::start(&agent, &scenario, &home, &output);
        let plain = output.join("plain");
        fs::write(&plain, "").unwrap();
        let log = output.join("node.log");
        let mut node = launch(&fixture, &config, &model.environment, &log);
        let failing = scenario == "error";
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let summary = runtime.block_on(async {
            let (mut stream, _) = super::turn_cancel::greet(&fixture, /*turn_cancel*/ true).await;
            let mut input = start("Answer briefly.");
            input.payload.spec.agent_plugin_id = PluginId::new(format!("official/{identifier}"));
            input.payload.spec.agent_plugin_version = PluginVersion::new(version.clone());
            input.payload.spec.keep_session_on_cancel = !failing;
            input.payload.spec.keep_session_on_failure = failing;
            send(
                &mut stream,
                ControllerToNodeMessage::StartAgentSession(input),
            )
            .await;
            let attempt = until_attempt(&fixture).await;
            let warm = follow(&mut stream, None, &attempt).await;
            let earlier = marked(&attempt);
            fs::remove_file(&plain).unwrap();
            // The model follows the flag within ten milliseconds.
            tokio::time::sleep(Duration::from_millis(200)).await;
            send(
                &mut stream,
                submit(
                    "target",
                    "Run the cancellation conformance task using the available tools.",
                ),
            )
            .await;
            let target = if failing {
                follow(&mut stream, None, &attempt).await
            } else {
                let deadline = Instant::now() + Duration::from_secs(60);
                while !checkout.join("started").exists() {
                    assert!(
                        Instant::now() < deadline,
                        "the Agent never ran the target tool"
                    );
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
                send(&mut stream, cancel("target-cancel", "target")).await;
                follow(&mut stream, Some("target-cancel"), &attempt).await
            };
            let settled_at = Instant::now();
            let markers_at_end = ["leaked", "woke"].map(|name| checkout.join(name).exists());
            fs::write(&plain, "").unwrap();
            tokio::time::sleep(Duration::from_millis(200)).await;
            let next = if target.session_end.is_none() {
                send(&mut stream, submit("next", "Continue with a short answer.")).await;
                Some(follow(&mut stream, None, &attempt).await)
            } else {
                None
            };
            tokio::time::sleep((settled_at + OBSERVE).saturating_duration_since(Instant::now()))
                .await;
            let markers_after = ["leaked", "woke"].map(|name| checkout.join(name).exists());
            let final_end = if next.as_ref().is_some_and(|next| next.session_end.is_none()) {
                send(&mut stream, end()).await;
                follow(&mut stream, None, &attempt).await.session_end
            } else {
                None
            };
            let earlier_sessions: std::collections::BTreeSet<u32> =
                earlier.iter().map(|process| process.session).collect();
            let sampled = if failing {
                next.as_ref()
                    .and_then(|next| next.alive_at_first_event.clone())
                    .unwrap_or_default()
            } else {
                target.alive_at_end.clone()
            };
            let escaped: Vec<&Marked> = sampled
                .iter()
                .filter(|process| !earlier.contains(process))
                .filter(|process| !earlier_sessions.contains(&process.session))
                .collect();
            let conformant = warm.session_end.is_none()
                && target.ended
                && target.session_end.is_none()
                && (failing || target.settled == Some(TurnCancelOutcome::Stopped))
                && escaped.is_empty()
                && markers_after == [false, false]
                && next
                    .as_ref()
                    .is_some_and(|next| next.ended && next.session_end.is_none())
                && final_end
                    .as_ref()
                    .is_some_and(|(reason, _)| *reason == AgentSessionEndReason::UserEnded);
            serde_json::json!({
                "package": {"identifier": identifier, "version": version},
                "agent": agent,
                "scenario": scenario,
                "earlier_processes": earlier,
                "target": target,
                "escaped_at_end": escaped,
                "markers_at_end": markers_at_end,
                "markers_after_observation": markers_after,
                "next": next,
                "final_end": final_end,
                "conformant": conformant,
            })
        });
        node.terminate();
        drop(model);
        fs::write(
            output.join("summary.json"),
            serde_json::to_vec_pretty(&summary).unwrap(),
        )
        .unwrap();
        assert!(summary["conformant"].as_bool().unwrap(), "{summary:#}");
    });
}

/// Waits until the session registered its attempt.
async fn until_attempt(fixture: &Fixture) -> String {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(attempt) = attempt(fixture) {
            return attempt;
        }
        assert!(
            Instant::now() < deadline,
            "the session registered no attempt"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
