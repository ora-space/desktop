//! What the turn scope reaches with real processes: descendants that dropped the mark, and
//! processes that carry a forged mark.
//!
//! Evidence for specs/test-cases/node/protocol/turn-process-fence.md; each test names the case it
//! covers.
use super::*;
use pretty_assertions::assert_eq;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};

/// Whether a live `/bin/sleep 60` that the fixture started in `checkout` has this PID; a PID
/// reused by anything else does not count.
fn sleeping(pid: u32, checkout: &Path) -> bool {
    let proc = Path::new("/proc").join(pid.to_string());
    std::fs::read(proc.join("cmdline")).is_ok_and(|command| command == b"/bin/sleep\x0060\x00")
        && std::fs::read_link(proc.join("cwd")).is_ok_and(|cwd| cwd == checkout)
        && std::fs::read_to_string(proc.join("stat"))
            .is_ok_and(|stat| !stat.contains(") Z ") && !stat.contains(") X "))
}

/// Case 3: a `setsid` job that keeps forking children with an empty environment loses every one
/// of them to the fence: the children carry no mark, but the fence freezes their parent first, so
/// each child still names a claimed parent when it is killed, and none is left as an orphan.
#[tokio::test(flavor = "multi_thread")]
async fn children_that_cleared_their_environment_die_with_the_turn() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[cleared-children] [hold]", CANCEL);
    let record = fixture.checkout().join("cleared.pids");
    let read = || -> Vec<u32> {
        std::fs::read_to_string(&record)
            .unwrap_or_default()
            .lines()
            .filter_map(|pid| pid.trim().parse().ok())
            .collect()
    };
    until(|| (read().len() >= 3).then_some(())).await;
    prompted(&fixture, "[hold]").await;
    cancel(&fixture, &sessions, "cancel-1");
    let outcome = until(|| {
        fixture
            .ledger
            .cancels()
            .into_iter()
            .find_map(|(id, outcome)| (id == "cancel-1").then_some(outcome))
    })
    .await;
    let checkout = fixture.checkout();
    let survivors: Vec<u32> = read()
        .into_iter()
        .filter(|pid| sleeping(*pid, &checkout))
        .collect();
    assert_eq!(
        (outcome, survivors, end(&fixture, &sessions).await),
        (
            TurnCancelOutcome::Stopped,
            Vec::new(),
            AgentSessionEndReason::UserEnded
        )
    );
}

/// The attempt the session registered, named by its mark scope record.
fn attempt(fixture: &Fixture) -> String {
    std::fs::read_dir(fixture.home().join("session-attempts"))
        .unwrap()
        .flatten()
        .find_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            name.strip_suffix(".mark").map(str::to_owned)
        })
        .expect("the attempt recorded its mark scope")
}

/// A process outside the Node that carries `mark` as its attempt mark, if any, killed and
/// reaped however the test ends.
struct Forged(Child);

impl Forged {
    /// Starts `script` under `/bin/sh` in `session`.
    fn start(mark: Option<&str>, session: Session, script: &str) -> Self {
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", script])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(mark) = mark {
            command.env("ORA_SESSION_ATTEMPT", mark);
        }
        if matches!(session, Session::New) {
            // SAFETY: setsid is async-signal-safe and only changes the forked child.
            unsafe {
                command.pre_exec(|| {
                    if libc::setsid() < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        Self(command.spawn().unwrap())
    }

    /// Pins the forged process itself.
    fn pin(&self) -> LinuxPidFd {
        LinuxPidFd::from_observation(&linux_process(self.0.id()).unwrap()).unwrap()
    }
}

impl Drop for Forged {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Which session a forged process starts in.
#[derive(Clone, Copy)]
enum Session {
    /// The test's own, which the Node and the Agent share.
    Inherited,
    New,
}

/// Case 8: a process that forges this attempt's mark gains no reach over what predates the
/// turn: a forger started before the turn, in a session of its own or in the test's, is never
/// signalled, nor is what it starts during the turn in its own session. What it starts during
/// the turn in a new session, and a forged process started during the turn in a new session, are
/// the forger's own turn processes, and only those die. An unmarked process is never touched.
#[tokio::test(flavor = "multi_thread")]
async fn a_forged_mark_only_kills_the_forgers_own_turn_processes() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "earlier", CANCEL);
    turns_ended(&fixture, 1).await;
    let mark = attempt(&fixture);
    // Both predate the turn. The forger leads its own session and, once asked, starts one child
    // in that session and one in a new session, printing their PIDs.
    let mut forger = Forged::start(
        Some(&mark),
        Session::New,
        "read line; /bin/sleep 600 & echo $!; /usr/bin/setsid /bin/sleep 600 & echo $!; wait",
    );
    let inherited = Forged::start(Some(&mark), Session::Inherited, "exec /bin/sleep 600");
    // Start times have clock-tick granularity; the turn must start several ticks later.
    tokio::time::sleep(Duration::from_millis(100)).await;
    fixture.command(
        &sessions,
        "command-2",
        SessionCommand::SubmitUserTurn(turn("turn-2", "[hold] later")),
    );
    prompted(&fixture, "later").await;
    writeln!(forger.0.stdin.take().unwrap()).unwrap();
    let mut lines = BufReader::new(forger.0.stdout.take().unwrap()).lines();
    let mut child = || {
        let pid = lines.next().unwrap().unwrap().trim().parse().unwrap();
        LinuxPidFd::from_observation(&linux_process(pid).unwrap()).unwrap()
    };
    let (in_its_session, in_a_new_session) = (child(), child());
    let late = Forged::start(Some(&mark), Session::New, "exec /bin/sleep 600");
    let unmarked = Forged::start(None, Session::New, "exec /bin/sleep 600");
    let watched = [
        forger.pin(),
        inherited.pin(),
        in_its_session,
        in_a_new_session,
        late.pin(),
        unmarked.pin(),
    ];
    fixture.command(
        &sessions,
        "cancel-2",
        SessionCommand::CancelTurn(TurnId::new("turn-2")),
    );
    let outcome = until(|| {
        fixture
            .ledger
            .cancels()
            .into_iter()
            .find_map(|(id, outcome)| (id == "cancel-2").then_some(outcome))
    })
    .await;
    let alive: Vec<bool> = watched
        .iter()
        .map(|process| !process.has_exited().unwrap())
        .collect();
    // A survivor that was frozen but not killed would show `T`.
    let frozen = [forger.0.id(), inherited.0.id()].iter().any(|pid| {
        std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| stat.contains(") T "))
    });
    assert_eq!(
        (outcome, alive, frozen),
        (
            TurnCancelOutcome::Stopped,
            vec![true, true, true, false, false, true],
            false
        )
    );
    drop((forger, inherited, late, unmarked));
    assert_eq!(
        end(&fixture, &sessions).await,
        AgentSessionEndReason::UserEnded
    );
}
