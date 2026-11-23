//! A Node killed around the turn process fence: the restarted Node ends the session as
//! `interrupted`, settles any pending cancel first, and stops the whole attempt, whatever the
//! fence had frozen or not yet killed. Nothing about a turn survives the restart.
//!
//! Evidence for specs/test-cases/node/protocol/turn-process-fence.md case 7.
//!
//! The production Node keeps sessions only for the versions on its compiled-in turn-boundary
//! list, so the echo fixture is installed under the first identity that list verifies on Linux.
use super::turn_cancel::{cancel, greet};
use super::*;
use crate::support::{ChildGuard, until};
use pretty_assertions::assert_eq;
use std::time::{Duration, Instant};
use tokio::net::UnixStream;

/// The live processes whose environment lists `attempt` under the attempt mark, with their
/// one-letter states.
fn marked(attempt: &str) -> Vec<(u32, char)> {
    let mut found = Vec::new();
    for entry in fs::read_dir("/proc").unwrap().flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let (Ok(environ), Ok(stat)) = (
            fs::read(entry.path().join("environ")),
            fs::read_to_string(entry.path().join("stat")),
        ) else {
            continue;
        };
        let carries = environ.split(|byte| *byte == 0).any(|variable| {
            String::from_utf8_lossy(variable)
                .strip_prefix("ORA_SESSION_ATTEMPT=")
                .is_some_and(|value| value.split(':').any(|token| token == attempt))
        });
        let state = stat
            .rsplit_once(") ")
            .and_then(|(_, rest)| rest.chars().next());
        if let (true, Some(state)) = (carries, state)
            && state != 'Z'
        {
            found.push((pid, state));
        }
    }
    found
}

/// The one-letter state of a process, `None` once it is gone.
fn state(pid: u32) -> Option<char> {
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()?
        .rsplit_once(") ")?
        .1
        .chars()
        .next()
}

/// A Node serving a session of the echo fixture installed under the first identity the
/// production list verifies on Linux, so a stopped turn can keep that session.
struct Listed {
    fixture: Fixture,
    config: CloneConfig,
    listed: ora_node::TurnBoundary,
    checkout: PathBuf,
}

impl Listed {
    fn new() -> Self {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let echo = prepare(&fixture, &config, &server);
        let checkout = PathBuf::from(checkout(&fixture).path.as_str());
        let listed = ora_node::TurnCancelConfig::default()
            .turn_boundary
            .into_iter()
            .find(|boundary| boundary.platform == ora_node::TurnBoundaryPlatform::Linux)
            .expect("the production list verifies a version on Linux");
        let (namespace, identifier) = listed.plugin.as_str().split_once('/').unwrap();
        let installed = echo
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(namespace)
            .join(identifier)
            .join(listed.version.as_str());
        fs::create_dir_all(&installed).unwrap();
        fs::write(installed.join("main.js"), "export {};\n").unwrap();
        fs::write(
            installed.join("orax.toml"),
            format!(
                "resolver = 1\nidentifier = \"{identifier}\"\nkind = \"agent\"\nversion = \"{}\"\ndescription = \"test\"\n",
                listed.version.as_str()
            ),
        )
        .unwrap();
        Self {
            fixture,
            config,
            listed,
            checkout,
        }
    }

    /// Starts a session that chose to be kept after a cancel, with `prompt` as its first turn,
    /// and returns once its first Thread event arrived.
    async fn start(&self, prompt: &str) -> UnixStream {
        let (mut stream, _) = greet(&self.fixture, /*turn_cancel*/ true).await;
        let mut input = start(prompt);
        input.payload.spec.agent_plugin_id = self.listed.plugin.clone();
        input.payload.spec.agent_plugin_version = self.listed.version.clone();
        input.payload.spec.keep_session_on_cancel = true;
        send(
            &mut stream,
            ControllerToNodeMessage::StartAgentSession(input),
        )
        .await;
        while !matches!(
            receive(&mut stream).await,
            NodeToControllerMessage::ThreadEvent(_)
        ) {}
        stream
    }

    /// The session's one attempt.
    fn attempt(&self) -> String {
        rusqlite::Connection::open(self.fixture.config().home_directory.join("ora-node.sqlite3"))
            .unwrap()
            .query_row(
                "SELECT attempt FROM session_stop_attempts WHERE execution=?1 AND json_extract(record,'$.kind')='agent'",
                [EXECUTION],
                |row| row.get(0),
            )
            .unwrap()
    }

    /// Waits until the fixture recorded a PID in `name` and returns it.
    fn recorded(&self, name: &str) -> u32 {
        let record = self.checkout.join(name);
        let mut pid = None;
        until(|| {
            pid = fs::read_to_string(&record)
                .ok()
                .and_then(|pid| pid.trim().parse().ok());
            pid.is_some()
        });
        pid.unwrap()
    }

    /// Restarts the Node after `node` was killed and returns, once the session ended, every
    /// cancel settlement the new Node delivered and the end reason, then stops that Node and
    /// checks that nothing of `attempt` was left at the end.
    fn restart(
        &self,
        runtime: &tokio::runtime::Runtime,
        attempt: &str,
    ) -> (
        Vec<TurnCancelOutcome>,
        AgentSessionEndReason,
        Vec<(u32, char)>,
    ) {
        let mut replacement = launch(&self.fixture, &self.config);
        let (settled, ended) = runtime.block_on(async {
            let (mut stream, _) = greet(&self.fixture, /*turn_cancel*/ true).await;
            let mut settled = Vec::new();
            loop {
                match receive(&mut stream).await {
                    NodeToControllerMessage::TurnCancelSettled(event) => {
                        settled.push(event.payload.outcome);
                    }
                    NodeToControllerMessage::AgentSessionEnded(event) => {
                        let AgentSessionResult::AgentSessionEnded(ended) = event.payload;
                        return (settled, ended.reason);
                    }
                    _ => {}
                }
            }
        });
        let remaining = marked(attempt);
        replacement.terminate();
        until(|| marked(attempt).is_empty());
        (settled, ended, remaining)
    }
}

/// A current-thread runtime for the Controller side.
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// Kills the Node the way a crash does, while the Controller connection is closed.
fn crash(node: &mut ChildGuard, stream: UnixStream) {
    drop(stream);
    node.kill();
}

/// The Node is killed while the fence has a job frozen: the job stays stopped, since nothing
/// continues it, and the restarted Node kills it with the rest of the attempt.
#[test]
fn a_node_killed_during_the_fence_ends_the_session_and_stops_the_attempt() {
    ora_logging::with_trace_logging(|| {
        let listed = Listed::new();
        let mut node = launch(&listed.fixture, &listed.config);
        let runtime = runtime();
        let mut stream = runtime.block_on(listed.start("[respawn] [hold]"));
        let attempt = listed.attempt();
        listed.recorded("respawn.pid");
        runtime.block_on(async {
            tokio::time::sleep(Duration::from_millis(500)).await;
            send(&mut stream, cancel("cancel-fence", "first")).await;
        });
        // The supervisor restarts its job each time the fence kills it, and F is 5 s: the fence
        // freezes job after job. The Node dies the moment one of them is seen stopped, which the
        // fence keeps it for a whole scan of procfs.
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut frozen = None;
        while frozen.is_none() && Instant::now() < deadline {
            let current = fs::read_to_string(listed.checkout.join("respawn.pid"))
                .ok()
                .and_then(|pid| pid.trim().parse::<u32>().ok());
            if let Some(pid) = current
                && state(pid) == Some('T')
            {
                frozen = Some(pid);
            }
        }
        crash(&mut node, stream);
        let frozen = frozen.expect("the fence froze a job while the Node ran");
        let frozen_at_kill = state(frozen);
        let (settled, ended, remaining) = listed.restart(&runtime, &attempt);
        assert_eq!(
            (frozen_at_kill, state(frozen), settled, ended, remaining),
            (
                Some('T'),
                None,
                vec![TurnCancelOutcome::SessionEnding],
                AgentSessionEndReason::Interrupted,
                Vec::new()
            )
        );
    });
}

/// The Node is killed once the turn started, before any cancel: nothing about the turn
/// survives, the restarted Node ends the session and kills the turn's job.
#[test]
fn a_node_killed_after_the_turn_started_ends_the_session_and_stops_the_attempt() {
    ora_logging::with_trace_logging(|| {
        let listed = Listed::new();
        let mut node = launch(&listed.fixture, &listed.config);
        let runtime = runtime();
        let stream = runtime.block_on(listed.start("[background 30000] [hold]"));
        let attempt = listed.attempt();
        let job = listed.recorded("background.pid");
        crash(&mut node, stream);
        let alive_after_crash = state(job).is_some();
        let (settled, ended, remaining) = listed.restart(&runtime, &attempt);
        assert_eq!(
            (alive_after_crash, settled, ended, remaining),
            (
                true,
                Vec::new(),
                AgentSessionEndReason::Interrupted,
                Vec::new()
            )
        );
        assert!(!listed.checkout.join("background-leak").exists());
    });
}

/// The Node is killed after a cancel kept the session and the next turn started a job: the
/// earlier `stopped` stays the cancel's only settlement, and the restarted Node ends the session
/// and kills the whole attempt.
#[test]
fn a_node_killed_after_a_kept_cancel_ends_the_session_and_stops_the_attempt() {
    ora_logging::with_trace_logging(|| {
        let listed = Listed::new();
        let mut node = launch(&listed.fixture, &listed.config);
        let runtime = runtime();
        let mut stream = runtime.block_on(listed.start("[hold]"));
        let attempt = listed.attempt();
        let first = runtime.block_on(async {
            tokio::time::sleep(Duration::from_millis(500)).await;
            send(&mut stream, cancel("cancel-kept", "first")).await;
            loop {
                if let NodeToControllerMessage::TurnCancelSettled(event) =
                    receive(&mut stream).await
                {
                    break event.payload.outcome;
                }
            }
        });
        runtime.block_on(send(
            &mut stream,
            ControllerToNodeMessage::SubmitUserTurn(SubmitUserTurnMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: start("").operation_id,
                execution_id: ExecutionId::new(EXECUTION),
                payload: SubmitUserTurn {
                    mcp_generation: None,
                    node_id: NodeId::new("test-node"),
                    command_id: CommandId::new("next-command"),
                    turn: UserTurn {
                        turn_id: TurnId::new("next"),
                        content: vec![ContentBlock::Text {
                            text: "[background 30000] [hold]".into(),
                        }],
                    },
                },
            }),
        ));
        let job = listed.recorded("background.pid");
        crash(&mut node, stream);
        let (settled, ended, remaining) = listed.restart(&runtime, &attempt);
        // The restarted Node may deliver the stored settlement again; it is the same one.
        let replayed_only_stopped = settled
            .iter()
            .all(|outcome| *outcome == TurnCancelOutcome::Stopped);
        assert_eq!(
            (first, replayed_only_stopped, ended, state(job), remaining),
            (
                TurnCancelOutcome::Stopped,
                true,
                AgentSessionEndReason::Interrupted,
                None,
                Vec::new()
            )
        );
        assert!(!listed.checkout.join("background-leak").exists());
    });
}
