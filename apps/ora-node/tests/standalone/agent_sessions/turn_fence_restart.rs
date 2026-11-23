//! A Node killed while the turn process fence runs: the restarted Node ends the session as
//! `interrupted`, settles the pending cancel first, and stops the whole attempt, whatever the
//! fence had frozen or not yet killed.
//!
//! Evidence for specs/test-cases/node/protocol/turn-process-fence.md case 7.
//!
//! The production Node keeps sessions only for the versions on its compiled-in turn-boundary
//! list, so the echo fixture is installed under the first identity that list verifies on Linux.
use super::turn_cancel::{cancel, greet};
use super::*;
use crate::support::until;
use pretty_assertions::assert_eq;
use std::time::{Duration, Instant};

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

#[test]
fn a_node_killed_during_the_fence_ends_the_session_and_stops_the_attempt() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let echo = prepare(&fixture, &config, &server);
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
        let mut node = launch(&fixture, &config);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let stream = runtime.block_on(async {
            let (mut stream, _) = greet(&fixture, /*turn_cancel*/ true).await;
            let mut input = start("[respawn] [hold]");
            input.payload.spec.agent_plugin_id = listed.plugin.clone();
            input.payload.spec.agent_plugin_version = listed.version.clone();
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
            tokio::time::sleep(Duration::from_millis(500)).await;
            send(&mut stream, cancel("cancel-fence", "first")).await;
            stream
        });
        let attempt: String = rusqlite::Connection::open(
            fixture.config().home_directory.join("ora-node.sqlite3"),
        )
        .unwrap()
        .query_row(
            "SELECT attempt FROM session_stop_attempts WHERE execution=?1 AND json_extract(record,'$.kind')='agent'",
            [EXECUTION],
            |row| row.get(0),
        )
        .unwrap();
        // The supervisor restarts its job each time the fence kills it; F is 5 s, so a second
        // in, the fence is still freezing and killing.
        let mut jobs = std::collections::BTreeSet::new();
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            jobs.extend(
                marked(&attempt)
                    .into_iter()
                    .filter(|(pid, _)| {
                        fs::read(format!("/proc/{pid}/cmdline"))
                            .is_ok_and(|command| command.starts_with(b"/bin/sleep\0"))
                    })
                    .map(|(pid, _)| pid),
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        drop(stream);
        node.kill();
        let mut replacement = launch(&fixture, &config);
        let (settled, ended) = runtime.block_on(async {
            let (mut stream, _) = greet(&fixture, /*turn_cancel*/ true).await;
            let mut settled = None;
            loop {
                match receive(&mut stream).await {
                    NodeToControllerMessage::TurnCancelSettled(event) => {
                        settled = Some(event.payload.outcome);
                    }
                    NodeToControllerMessage::AgentSessionEnded(event) => {
                        let AgentSessionResult::AgentSessionEnded(ended) = event.payload;
                        return (settled, ended.reason);
                    }
                    _ => {}
                }
            }
        });
        let remaining = marked(&attempt);
        replacement.terminate();
        until(|| marked(&attempt).is_empty());
        assert_eq!(
            (jobs.len() > 1, settled, ended, remaining),
            (
                true,
                Some(TurnCancelOutcome::SessionEnding),
                AgentSessionEndReason::Interrupted,
                Vec::new()
            )
        );
    });
}
