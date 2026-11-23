//! Where the fence sits in the judgement of a stopped turn: before the quiet window, with a
//! scan-only re-check right before `stopped`, and how cancels and `EndSession` accepted while the
//! stop is judged are served.
//!
//! Evidence for specs/test-cases/node/protocol/turn-process-fence.md; each test names the case it
//! covers. None depends on how fast the machine runs: where a process must appear after the
//! fence, an event the fence causes triggers it, and every quiet window outlasts the processes it
//! must see gone.
use super::*;
use pretty_assertions::assert_eq;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Settles every cancel of the session and returns them in acceptance order once `count` did.
async fn cancels(fixture: &Fixture, count: usize) -> Vec<(String, TurnCancelOutcome)> {
    until(|| {
        let cancels = fixture.ledger.cancels();
        (cancels.len() == count).then_some(cancels)
    })
    .await
}

/// Cases 1 and 5: a helper in the Agent's own session that silently starts a `setsid` job while
/// the quiet window runs, a second after the fence killed its sentinel, is caught by the re-check
/// before the stop settles: the session ends with `turn_processes_unstopped` instead of being
/// kept, and its stop kills the job.
#[tokio::test(flavor = "multi_thread")]
async fn a_job_started_after_a_clean_fence_ends_the_session() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    fixture.turn_cancel.quiet = Duration::from_secs(4);
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[late-job 1000] [hold]", CANCEL);
    prompted(&fixture, "[hold]").await;
    cancel(&fixture, &sessions, "cancel-1");
    let job = pinned(&fixture.checkout(), "late-job.pid").await;
    let ended = fixture.ended().await;
    assert_eq!(
        (
            fixture.ledger.cancels(),
            ended.detail.as_deref(),
            job.has_exited().unwrap()
        ),
        (
            vec![("cancel-1".to_owned(), TurnCancelOutcome::SessionEnding)],
            Some("turn_processes_unstopped"),
            true
        )
    );
}

/// Case 5: the fence runs before the quiet window, so an Agent that reacts to its background
/// job dying with turn traffic, as Claude Code starts a turn by itself, ends the session with
/// `turn_not_quiescent`.
#[tokio::test(flavor = "multi_thread")]
async fn turn_traffic_caused_by_the_fence_ends_the_session() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    fixture.turn_cancel.quiet = Duration::from_secs(2);
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[watch-job] [hold]", CANCEL);
    let job = pinned(&fixture.checkout(), "watch-job.pid").await;
    prompted(&fixture, "[hold]").await;
    cancel(&fixture, &sessions, "cancel-1");
    let ended = fixture.ended().await;
    // The Agent's message is late turn traffic: it joins no turn, so only the end shows it.
    assert_eq!(
        (ended.detail.as_deref(), job.has_exited().unwrap()),
        (Some("turn_not_quiescent"), true)
    );
}

/// Case 5: a second cancel accepted while the stop is judged, during the fence or the quiet
/// window, merges into the stop and both settle `stopped`; the session is kept. The Agent keeps
/// restarting a 200 ms job for 600 ms after its answer, and the quiet window outlasts the last
/// one, so the re-check finds the scope clean however the fence's scans fell between restarts.
#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_accepted_while_the_stop_is_judged_settles_stopped() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    fixture.turn_cancel.quiet = Duration::from_secs(3);
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[respawn-for 600] [hold]", CANCEL);
    prompted(&fixture, "[hold]").await;
    cancel(&fixture, &sessions, "cancel-1");
    tokio::time::sleep(Duration::from_millis(300)).await;
    cancel(&fixture, &sessions, "cancel-2");
    let settled = cancels(&fixture, 2).await;
    let kept = fixture.ledger.ended().is_none();
    assert_eq!(
        (settled, kept, end(&fixture, &sessions).await),
        (
            vec![
                ("cancel-1".to_owned(), TurnCancelOutcome::Stopped),
                ("cancel-2".to_owned(), TurnCancelOutcome::Stopped)
            ],
            true,
            AgentSessionEndReason::UserEnded
        )
    );
}

/// Cases 4 and 5: a second cancel accepted while a fence that never gets clean runs shares the
/// stop's conclusion: both settle `session_ending` and the session ends with
/// `turn_processes_unstopped`.
#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_accepted_during_an_unclean_fence_shares_its_end() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    fixture.turn_cancel.process_fence = Duration::from_secs(1);
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[respawn] [hold]", CANCEL);
    prompted(&fixture, "[hold]").await;
    cancel(&fixture, &sessions, "cancel-1");
    tokio::time::sleep(Duration::from_millis(300)).await;
    cancel(&fixture, &sessions, "cancel-2");
    let ended = fixture.ended().await;
    assert_eq!(
        (fixture.ledger.cancels(), ended.detail.as_deref()),
        (
            vec![
                ("cancel-1".to_owned(), TurnCancelOutcome::SessionEnding),
                ("cancel-2".to_owned(), TurnCancelOutcome::SessionEnding)
            ],
            Some("turn_processes_unstopped")
        )
    );
}

/// Case 5: an `EndSession` accepted while the stop is judged takes precedence, whether the fence
/// comes back clean or not: the session ends as the user asked and the cancel settles
/// `session_ending`. The quiet window is long, so the end always arrives before it is over.
#[tokio::test(flavor = "multi_thread")]
async fn an_end_session_accepted_during_the_fence_takes_precedence() {
    for (marker, fence) in [
        ("[respawn-for 600] [hold]", Duration::from_secs(10)),
        ("[respawn] [hold]", Duration::from_secs(1)),
    ] {
        let mut fixture = Fixture::new();
        fixture.verify_turn_boundary();
        fixture.turn_cancel.process_fence = fence;
        fixture.turn_cancel.quiet = Duration::from_secs(3);
        let sessions = fixture.sessions(PLUGIN_VERSION);
        fixture.start_keeping(&sessions, marker, CANCEL);
        prompted(&fixture, "[hold]").await;
        cancel(&fixture, &sessions, "cancel-1");
        tokio::time::sleep(Duration::from_millis(300)).await;
        let reason = end(&fixture, &sessions).await;
        assert_eq!(
            (marker, fixture.ledger.cancels(), reason),
            (
                marker,
                vec![("cancel-1".to_owned(), TurnCancelOutcome::SessionEnding)],
                AgentSessionEndReason::UserEnded
            )
        );
    }
}

/// Holds the re-check of a stopped turn open while a command is accepted.
///
/// The fence and the re-check read the attempt's mark scope record on the stop worker, and
/// nothing else reads it while a session runs. Before the cancel, the record is replaced by a
/// FIFO: every read of it blocks in `open` until this helper opens the other end. The first read
/// is the fence's, served at once; the second is the re-check's, and `during` runs while it
/// waits. Then the record is put back as a plain file and the re-check goes on. If the fence
/// needed more than one observation, the second read is one of the fence's instead, and the
/// command lands earlier in the same stop.
///
/// A reader left blocked on the FIFO would hold the one stop worker of this test binary for
/// good, so dropping the helper, also when the test fails, puts the record back and serves
/// every reader still waiting on the FIFO.
struct HeldRecheck {
    record: PathBuf,
    side: PathBuf,
    content: Vec<u8>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<bool>>,
}

impl HeldRecheck {
    fn new(fixture: &Fixture, during: impl FnOnce() + Send + 'static) -> Self {
        let attempts = fixture.home().join("session-attempts");
        let record = std::fs::read_dir(&attempts)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .find(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "mark")
            })
            .expect("the attempt recorded its mark scope");
        let content = std::fs::read(&record).unwrap();
        let fifo = attempts.join("held.fifo");
        let path = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        // SAFETY: mkfifo reads one NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        std::fs::rename(&fifo, &record).unwrap();
        let side = attempts.join("released.fifo");
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (record, side, content, stop) = (
                record.clone(),
                side.clone(),
                content.clone(),
                Arc::clone(&stop),
            );
            std::thread::spawn(move || {
                let mut during = Some(during);
                let mut reads = 0;
                while !stop.load(Ordering::Acquire) {
                    let Some(writer) = waiting_reader(&record) else {
                        std::thread::sleep(Duration::from_millis(1));
                        continue;
                    };
                    reads += 1;
                    if reads == 2
                        && let Some(during) = during.take()
                    {
                        during();
                        restore(&record, &side, &content);
                    }
                    if reads == 2 {
                        // The record is back in place; the FIFO now lives at `side`.
                        serve(writer, &content, &side);
                        return true;
                    }
                    serve(writer, &content, &record);
                }
                false
            })
        };
        Self {
            record,
            side,
            content,
            stop,
            thread: Some(thread),
        }
    }

    /// Whether a second read was held.
    fn finish(mut self) -> bool {
        self.stop.store(true, Ordering::Release);
        self.thread
            .take()
            .is_some_and(|thread| thread.join().unwrap_or(false))
    }
}

impl Drop for HeldRecheck {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        if std::fs::symlink_metadata(&self.record).is_ok_and(|metadata| {
            use std::os::unix::fs::FileTypeExt;
            metadata.file_type().is_fifo()
        }) {
            restore(&self.record, &self.side, &self.content);
        }
        // A reader that reached the FIFO before it moved aside still waits on it.
        let until = Instant::now() + Duration::from_millis(200);
        while Instant::now() < until {
            if let Some(writer) = waiting_reader(&self.side) {
                serve(writer, &self.content, &self.side);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

/// Opens the writing end of the FIFO at `path` if a reader waits on it; never blocks.
fn waiting_reader(path: &Path) -> Option<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .ok()
}

/// Gives the reader `writer` was opened for the record, then waits until that reader closed the
/// FIFO at `path`: it keeps it open until it read the end, and a reader still open is the same
/// one, which must not get the record twice.
fn serve(mut writer: std::fs::File, content: &[u8], path: &Path) {
    // A reader gone meanwhile only breaks the pipe.
    let _ = writer.write_all(content);
    drop(writer);
    while waiting_reader(path).is_some() {
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Moves the FIFO at `record` aside to `side`, where waiting readers can still be served, and
/// puts the plain record back, so every later read finds the file.
fn restore(record: &Path, side: &Path, content: &[u8]) {
    let _ = std::fs::rename(record, side);
    let plain = record.with_extension("plain");
    std::fs::write(&plain, content).unwrap();
    std::fs::rename(&plain, record).unwrap();
}

/// Case 5: a cancel accepted while the re-check runs merges into this stop and settles `stopped`
/// with the first, instead of `already_ended` once the session moved on.
#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_accepted_during_the_recheck_merges_into_the_stop() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[hold]", CANCEL);
    prompted(&fixture, "[hold]").await;
    let ledger = fixture.ledger.clone();
    let held = HeldRecheck::new(&fixture, move || {
        ledger.accept(
            "cancel-2",
            SessionCommand::CancelTurn(TurnId::new("turn-1")),
        );
    });
    cancel(&fixture, &sessions, "cancel-1");
    let settled = cancels(&fixture, 2).await;
    let kept = fixture.ledger.ended().is_none();
    let was_held = held.finish();
    assert_eq!(
        (was_held, settled, kept, end(&fixture, &sessions).await),
        (
            true,
            vec![
                ("cancel-1".to_owned(), TurnCancelOutcome::Stopped),
                ("cancel-2".to_owned(), TurnCancelOutcome::Stopped)
            ],
            true,
            AgentSessionEndReason::UserEnded
        )
    );
}

/// Case 5: an `EndSession` accepted while the re-check runs takes precedence over the stop: the
/// cancel settles `session_ending`, never `stopped`, and the session ends as the user asked.
#[tokio::test(flavor = "multi_thread")]
async fn an_end_session_accepted_during_the_recheck_takes_precedence() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[hold]", CANCEL);
    prompted(&fixture, "[hold]").await;
    let ledger = fixture.ledger.clone();
    let held = HeldRecheck::new(&fixture, move || {
        ledger.accept(
            "end",
            SessionCommand::EndSession(EndSessionReason::UserEnded),
        );
    });
    cancel(&fixture, &sessions, "cancel-1");
    let ended = fixture.ended().await;
    let was_held = held.finish();
    assert_eq!(
        (was_held, fixture.ledger.cancels(), ended.reason),
        (
            true,
            vec![("cancel-1".to_owned(), TurnCancelOutcome::SessionEnding)],
            AgentSessionEndReason::UserEnded
        )
    );
}

/// Cases 4 and 5: a re-check whose budget R runs out, here because it is spent before the
/// re-check could even wait for the stop worker, ends the session with
/// `turn_processes_unstopped` although the turn left nothing running.
#[tokio::test(flavor = "multi_thread")]
async fn a_recheck_past_its_budget_ends_the_session() {
    let mut fixture = Fixture::new();
    fixture.verify_turn_boundary();
    fixture.turn_cancel.process_recheck = Duration::ZERO;
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_keeping(&sessions, "[hold]", CANCEL);
    prompted(&fixture, "[hold]").await;
    cancel(&fixture, &sessions, "cancel-1");
    let ended = fixture.ended().await;
    assert_eq!(
        (fixture.ledger.cancels(), ended.detail.as_deref()),
        (
            vec![("cancel-1".to_owned(), TurnCancelOutcome::SessionEnding)],
            Some("turn_processes_unstopped")
        )
    );
}
