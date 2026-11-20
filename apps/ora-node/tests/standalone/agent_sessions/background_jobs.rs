//! A tool job that left every Node process group, and closed the attempt lock as Bun's children
//! do, dies before the Node reports a stop, on every path that ends a session; the ledger never
//! shows the attempt confirmed while the job runs, and the job never writes the checkout.
use super::*;
use crate::support::until;
use ora_utils::process::{LinuxPidFd, linux_process};
use pretty_assertions::assert_eq;
use std::{
    path::Path,
    process::{Child, Command},
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tokio::net::UnixStream;

/// The jobs' delay before writing; every stop path except a restart ends the session first.
const DELAY_MS: u64 = 4_000;

/// Leaves a restarted Node time to recover the session before its predecessor's job writes.
const RESTART_DELAY_MS: u64 = 12_000;

/// One job the fixture left in the session checkout, pinned against PID reuse.
struct Job {
    handle: LinuxPidFd,
    checkout: PathBuf,
    writes_at: Instant,
}

impl Job {
    /// Pins the job once it recorded itself, while it is certainly still running.
    fn pin(checkout: &Path, delay_ms: u64) -> (Self, LinuxPidFd) {
        let record = checkout.join("background.pid");
        until(|| record.exists());
        let pid = fs::read_to_string(&record).unwrap().trim().parse().unwrap();
        let pin = || LinuxPidFd::from_observation(&linux_process(pid).unwrap()).unwrap();
        let job = Self {
            handle: pin(),
            checkout: checkout.to_path_buf(),
            writes_at: Instant::now() + Duration::from_millis(delay_ms),
        };
        (job, pin())
    }

    /// Whether the job is still running.
    fn alive(&self) -> bool {
        !self.handle.has_exited().unwrap()
    }

    /// Waits past the job's own write time, so a job that escaped would have written by now.
    fn assert_never_wrote(&self) {
        let margin = self.writes_at + Duration::from_millis(500);
        std::thread::sleep(margin.saturating_duration_since(Instant::now()));
        assert!(!self.checkout.join("background-leak").exists());
    }
}

/// Samples the ledger and the job together until the attempt is confirmed, and fails if any
/// sample shows the attempt confirmed while the job still runs. Returns the recorded proof.
fn watch(fixture: &Fixture, job: LinuxPidFd) -> JoinHandle<Result<String, String>> {
    let database = fixture.config().home_directory.join("ora-node.sqlite3");
    std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(90);
        let db = rusqlite::Connection::open(&database).map_err(|error| error.to_string())?;
        while Instant::now() < deadline {
            let observed = {
                db.query_row(
                    "SELECT state, evidence FROM session_stop_attempts WHERE execution=?1 AND json_extract(record,'$.kind')='agent'",
                    [EXECUTION],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
                )
            };
            // The state is read before the job, so a running job seen afterwards was running
            // when the attempt was already confirmed.
            if let Ok((state, evidence)) = observed
                && state == "confirmed"
            {
                return if job.has_exited().unwrap() {
                    Ok(evidence.unwrap_or_default())
                } else {
                    Err("the attempt was confirmed while its job was running".into())
                };
            }
            // Short reads between the Node's writes; its busy timeout absorbs any overlap.
            std::thread::sleep(Duration::from_millis(5));
        }
        Err("the attempt was never confirmed".into())
    })
}

/// Starts a session and waits for the event that shows the prompt's job has started.
async fn start_job(stream: &mut UnixStream, prompt: &str) {
    send(
        stream,
        ControllerToNodeMessage::StartAgentSession(start(prompt)),
    )
    .await;
    let held = prompt.contains("[hold]");
    loop {
        if let NodeToControllerMessage::ThreadEvent(event) = receive(stream).await
            && (held
                || event
                    .payload
                    .record
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    == Some("turnEnded"))
        {
            return;
        }
    }
}

/// Ends the session and returns the end, checking the job's state at the moment it arrives.
async fn end_and_observe(stream: &mut UnixStream, job: &Job) -> (AgentSessionEndReason, bool) {
    send(stream, end()).await;
    loop {
        if let NodeToControllerMessage::AgentSessionEnded(event) = receive(stream).await {
            let AgentSessionResult::AgentSessionEnded(ended) = event.payload;
            return (ended.reason, job.alive());
        }
    }
}

/// The attempt of the one Agent session these tests run.
fn attempt(fixture: &Fixture) -> String {
    rusqlite::Connection::open(fixture.config().home_directory.join("ora-node.sqlite3"))
        .unwrap()
        .query_row(
            "SELECT attempt FROM session_stop_attempts WHERE execution=?1 AND json_extract(record,'$.kind')='agent'",
            [EXECUTION],
            |row| row.get(0),
        )
        .unwrap()
}

/// Starts a sleeper that carries `mark` as its attempt mark, if any, outside the Node.
fn sleeper(mark: Option<&str>) -> Child {
    let mut command = Command::new("/bin/sleep");
    command.arg("60");
    if let Some(mark) = mark {
        command.env("ORA_SESSION_ATTEMPT", mark);
    }
    command.spawn().unwrap()
}

/// EndSession after a finished turn kills the turn's job before the end is reported. Processes
/// the attempt never started survive, unless they carry its exact mark: a forged mark claims only
/// processes its forger started after the attempt began.
#[test]
fn end_session_kills_a_detached_tool_job_and_spares_unmarked_processes() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        prepare(&fixture, &config, &server);
        let checkout = PathBuf::from(checkout(&fixture).path.as_str());
        let mut child = launch(&fixture, &config);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (mut stream, _) = runtime.block_on(connect(&fixture));
        runtime.block_on(start_job(
            &mut stream,
            &format!("[background {DELAY_MS}] go"),
        ));
        let (job, sampled) = Job::pin(&checkout, DELAY_MS);
        let watcher = watch(&fixture, sampled);
        let attempt = attempt(&fixture);
        let mut strangers = [
            sleeper(None),
            sleeper(Some(&ora_node_db::AttemptId::generate().to_string())),
            sleeper(Some(&format!("{attempt}-other"))),
        ];
        let mut forged = sleeper(Some(&attempt));
        let ended = runtime.block_on(end_and_observe(&mut stream, &job));
        // The Node finishes serving a connected Controller before it exits.
        drop(stream);
        let survivors = strangers
            .iter_mut()
            .map(|stranger| stranger.try_wait().unwrap().is_none())
            .collect::<Vec<_>>();
        for stranger in &mut strangers {
            stranger.kill().unwrap();
            stranger.wait().unwrap();
        }
        let forged_killed = forged.try_wait().unwrap().is_some() || {
            forged.kill().unwrap();
            false
        };
        forged.wait().unwrap();
        child.terminate();
        assert_eq!(
            (ended, survivors, forged_killed, watcher.join().unwrap()),
            (
                (AgentSessionEndReason::UserEnded, false),
                vec![true; 3],
                true,
                Ok("lock_processes_and_marks".to_owned())
            )
        );
        job.assert_never_wrote();
    });
}

/// Ending a session while its turn still runs takes the same stop path: the job dies first. On
/// main this is how a running turn ends; a turn cancel without a verified boundary ends the
/// session through the same path.
#[test]
fn ending_a_running_turn_kills_its_detached_tool_job() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        prepare(&fixture, &config, &server);
        let checkout = PathBuf::from(checkout(&fixture).path.as_str());
        let mut child = launch(&fixture, &config);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (mut stream, _) = runtime.block_on(connect(&fixture));
        runtime.block_on(start_job(
            &mut stream,
            &format!("[background {DELAY_MS}] [hold]"),
        ));
        let (job, sampled) = Job::pin(&checkout, DELAY_MS);
        let watcher = watch(&fixture, sampled);
        let ended = runtime.block_on(end_and_observe(&mut stream, &job));
        // The Node finishes serving a connected Controller before it exits.
        drop(stream);
        child.terminate();
        assert_eq!(
            (ended, watcher.join().unwrap()),
            (
                (AgentSessionEndReason::UserEnded, false),
                Ok("lock_processes_and_marks".to_owned())
            )
        );
        job.assert_never_wrote();
    });
}

/// A killed Node leaves its job running; the restarted Node kills it before it records the
/// attempt confirmed and reports the session interrupted.
#[test]
fn restart_recovery_kills_the_job_a_killed_node_left_behind() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        prepare(&fixture, &config, &server);
        let checkout = PathBuf::from(checkout(&fixture).path.as_str());
        let mut child = launch(&fixture, &config);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (mut stream, _) = connect(&fixture).await;
            start_job(
                &mut stream,
                &format!("[background {RESTART_DELAY_MS}] [hold]"),
            )
            .await;
        });
        let (job, sampled) = Job::pin(&checkout, RESTART_DELAY_MS);
        let watcher = watch(&fixture, sampled);
        child.kill();
        let survived_node = job.alive();
        let mut replacement = launch(&fixture, &config);
        let ended = runtime.block_on(async {
            let (mut stream, _) = connect(&fixture).await;
            loop {
                if let NodeToControllerMessage::AgentSessionEnded(event) =
                    receive(&mut stream).await
                {
                    let AgentSessionResult::AgentSessionEnded(ended) = event.payload;
                    return (ended.reason, job.alive());
                }
            }
        });
        replacement.terminate();
        assert_eq!(
            (survived_node, ended, watcher.join().unwrap()),
            (
                true,
                (AgentSessionEndReason::Interrupted, false),
                Ok("lock_processes_and_marks".to_owned())
            )
        );
        job.assert_never_wrote();
    });
}

/// Graceful shutdown ends the session and kills its job before the Node exits.
#[test]
fn graceful_shutdown_kills_detached_tool_jobs_before_the_node_exits() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        prepare(&fixture, &config, &server);
        let checkout = PathBuf::from(checkout(&fixture).path.as_str());
        let mut child = launch(&fixture, &config);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (mut stream, _) = connect(&fixture).await;
                start_job(&mut stream, &format!("[background {DELAY_MS}] [hold]")).await;
            });
        let (job, sampled) = Job::pin(&checkout, DELAY_MS);
        let watcher = watch(&fixture, sampled);
        child.terminate();
        let alive_at_exit = job.alive();
        let db = ora_node_db::NodeDatabase::open(
            &fixture.config().home_directory.join("ora-node.sqlite3"),
            fixture.config().identity,
        )
        .unwrap();
        let ExecutionState::Completed(ExecutionResult::AgentSession(
            AgentSessionResult::AgentSessionEnded(ended),
        )) = db
            .execution_state(&start("").operation_id, &start("").execution_id)
            .unwrap()
        else {
            panic!("not ended")
        };
        assert_eq!(
            (alive_at_exit, ended.reason, watcher.join().unwrap()),
            (
                false,
                AgentSessionEndReason::Cancelled,
                Ok("lock_processes_and_marks".to_owned())
            )
        );
        job.assert_never_wrote();
    });
}

/// A job that made its environment unreadable is still the attempt's while the marked shell that
/// started it waits for it, and dies with that shell.
#[test]
fn an_unreadable_job_under_a_waiting_tool_shell_is_killed_with_it() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        prepare(&fixture, &config, &server);
        let checkout = PathBuf::from(checkout(&fixture).path.as_str());
        let mut child = launch(&fixture, &config);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (mut stream, _) = runtime.block_on(connect(&fixture));
        runtime.block_on(start_job(
            &mut stream,
            &format!("[background-hidden {DELAY_MS}] go"),
        ));
        let (job, sampled) = Job::pin(&checkout, DELAY_MS);
        // SAFETY: geteuid only reads this process's credentials.
        let root = unsafe { libc::geteuid() } == 0;
        let pid = fs::read_to_string(checkout.join("background.pid")).unwrap();
        let unreadable = root || fs::read(format!("/proc/{}/environ", pid.trim())).is_err();
        let watcher = watch(&fixture, sampled);
        let ended = runtime.block_on(end_and_observe(&mut stream, &job));
        // The Node finishes serving a connected Controller before it exits.
        drop(stream);
        child.terminate();
        assert_eq!(
            (unreadable, ended, watcher.join().unwrap()),
            (
                true,
                (AgentSessionEndReason::UserEnded, false),
                Ok("lock_processes_and_marks".to_owned())
            )
        );
        job.assert_never_wrote();
    });
}
