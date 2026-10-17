use super::*;
use ora_node::{LocalClock, WriteGuard, WritePoint};
use ora_node_db::NodeDatabase;
use pretty_assertions::assert_eq;

struct FailWrite(WritePoint);
impl WriteGuard for FailWrite {
    /// Stops exactly at the selected durable boundary without replacing Git or process supervision.
    fn before_write(&self, point: WritePoint) -> Result<(), ora_node_db::Error> {
        if point == self.0 {
            Err(ora_node_db::Error::Injected(point))
        } else {
            Ok(())
        }
    }
}

/// A completed real Git Run can produce its original result after Node's terminal transaction failed.
#[test]
fn clone_recovers_completed_git_after_terminal_or_outbox_write_failure() {
    ora_logging::with_trace_logging(|| {
        for point in [WritePoint::Complete, WritePoint::Outbox] {
            let fixture = Fixture::new();
            let commit = CommitId::new(fixture.git(&["rev-parse", "HEAD"]));
            let server =
                GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
            let config = configuration(&fixture, &server);
            let command = request(&server, "commit", "main");
            let mut node = Node::open_managed_with_dependencies(
                fixture.config(),
                fixture.process(),
                Shutdown::default(),
                FailWrite(point),
                LocalClock,
            )
            .unwrap();
            node.configure_clone(config.clone()).unwrap();
            node.configure_git_access(server.access());
            assert!(
                matches!(node.submit_clone(command.clone()), Err(ora_node::Error::Storage(ora_node_db::Error::Injected(actual))) if actual == point)
            );
            assert!(node.pending_events().unwrap().is_empty());
            drop(node);
            server.reject_auth(true);
            let mut node =
                Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap();
            node.configure_clone(config).unwrap();
            node.configure_git_access(server.access());
            assert_eq!(node.recover_clones().unwrap(), ora_node::NodeState::Ready);
            let result = node.submit_clone(command.clone()).unwrap();
            let Some(CloneExecutionResult::CloneReady(ready)) = clone_result(&result.state) else {
                panic!("lost completed acquisition: {result:?}");
            };
            assert_eq!(ready.commit, commit);
            assert_eq!(node.pending_events().unwrap().len(), 1);
            drop(node);
            let database = NodeDatabase::open(
                &fixture.config().home_directory.join("ora-node.sqlite3"),
                fixture.config().identity,
            )
            .unwrap();
            assert_eq!(
                database
                    .process_journal()
                    .unwrap()
                    .attempts(&command.execution_id)
                    .unwrap()
                    .len(),
                // One Run per workload step; recovery repeated none of them.
                3
            );
        }
    });
}

/// Directory creation without a committed native identity cannot be recovered by guessing ownership.
#[test]
fn clone_preserves_directory_when_creation_evidence_write_fails() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let command = request(&server, "directory-write", "main");
        let mut node = Node::open_managed_with_dependencies(
            fixture.config(),
            fixture.process(),
            Shutdown::default(),
            FailWrite(WritePoint::Progress),
            LocalClock,
        )
        .unwrap();
        node.configure_clone(config.clone()).unwrap();
        node.configure_git_access(server.access());
        assert!(node.submit_clone(command.clone()).is_err());
        drop(node);
        assert_eq!(fs::read_dir(&config.repository_root).unwrap().count(), 1);
        let mut node =
            Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap();
        node.configure_clone(config.clone()).unwrap();
        node.configure_git_access(server.access());
        assert_eq!(
            node.recover_clones().unwrap(),
            ora_node::NodeState::RecoveryPending
        );
        assert_eq!(
            node.submit_clone(command).unwrap().state,
            ExecutionState::Unknown
        );
        assert_eq!(fs::read_dir(&config.repository_root).unwrap().count(), 1);
        assert!(node.pending_events().unwrap().is_empty());
    });
}

/// Refuses only the `nth` write at `point`, so a clone stops right after the step before it.
struct FailNth {
    point: WritePoint,
    nth: usize,
    seen: std::sync::atomic::AtomicUsize,
}

impl FailNth {
    fn new(point: WritePoint, nth: usize) -> Self {
        Self {
            point,
            nth,
            seen: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

impl WriteGuard for FailNth {
    /// Counts the writes at the selected boundary and refuses exactly one of them.
    fn before_write(&self, point: WritePoint) -> Result<(), ora_node_db::Error> {
        if point == self.point && self.seen.fetch_add(1, Ordering::SeqCst) + 1 == self.nth {
            return Err(ora_node_db::Error::Injected(point));
        }
        Ok(())
    }
}

/// Runs one Node process over `fixture` until the clone stops at its `nth` progress write.
fn interrupt_at_progress_write(
    fixture: &Fixture,
    server: &GitServerFixture,
    config: &CloneConfig,
    command: &CloneRepositoryMessage,
    nth: usize,
) {
    let mut node = Node::open_managed_with_dependencies(
        fixture.config(),
        fixture.process(),
        Shutdown::default(),
        FailNth::new(WritePoint::Progress, nth),
        LocalClock,
    )
    .unwrap();
    node.configure_clone(config.clone()).unwrap();
    node.configure_git_access(server.access());
    // A first process admits the clone; a later one resumes it from recovery.
    let stopped = node
        .recover_clones()
        .and_then(|_| node.submit_clone(command.clone()).map(drop));
    assert!(matches!(
        stopped,
        Err(ora_node::Error::Storage(ora_node_db::Error::Injected(
            WritePoint::Progress
        )))
    ));
}

/// The durable phase the clone of `command` rests in.
fn phase(fixture: &Fixture, command: &CloneRepositoryMessage) -> ora_node_db::ClonePhase {
    let database = NodeDatabase::open(
        &fixture.config().home_directory.join("ora-node.sqlite3"),
        fixture.config().identity,
    )
    .unwrap();
    let record = database
        .find_clone(&command.operation_id, &command.execution_id)
        .unwrap()
        .unwrap();
    match record.progress {
        ora_node_db::CloneProgress::Pending(phase) => phase,
        other => panic!("clone is not pending: {other:?}"),
    }
}

/// How many snapshot fetch passes reached the Git Server: each starts with one ref
/// advertisement.
fn fetch_passes(server: &GitServerFixture) -> usize {
    server
        .server
        .served()
        .iter()
        .filter(|request| request.method == "GET" && request.path.ends_with("/info/refs"))
        .count()
}

/// A restart between the workload steps keeps the transport repository the next step reads
/// from: the clone completes without fetching the snapshot again.
#[test]
fn restart_inside_the_workload_sequence_keeps_its_transport_repository() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let command = request(&server, "workload-restart", "main");
        // Writes: directory created, fetch pass, step 0, step 1 — refused after step 0 ran.
        interrupt_at_progress_write(&fixture, &server, &config, &command, 4);
        assert!(matches!(
            phase(&fixture, &command),
            ora_node_db::ClonePhase::Dispatched { step: 0, .. }
        ));
        let passes = fetch_passes(&server);
        let mut node =
            Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap();
        node.configure_clone(config.clone()).unwrap();
        node.configure_git_access(server.access());
        node.recover_clones().unwrap();
        let state = node.submit_clone(command.clone()).unwrap().state;
        let Some(CloneExecutionResult::CloneReady(ready)) = clone_result(&state) else {
            panic!("clone did not complete after the restart: {state:?}");
        };
        assert_eq!(ready.commit, server.snapshot());
        assert_eq!(fetch_passes(&server), passes);
        assert_eq!(
            fs::read_dir(config.repository_root.join(".ora-transport"))
                .unwrap()
                .count(),
            0
        );
    });
}

/// A restart during the fetch pass discards that pass and fetches again, and after three
/// passes the clone ends as `SourceUnavailable` instead of fetching forever.
#[test]
fn restart_during_the_fetch_pass_fetches_again_up_to_three_passes() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let command = request(&server, "fetch-restart", "main");
        // Writes: directory created, fetch pass 1, step 0 — refused after the fetch.
        interrupt_at_progress_write(&fixture, &server, &config, &command, 3);
        for pass in 2..=3 {
            // Writes: fetch pass n, step 0 — refused after the fetch again.
            interrupt_at_progress_write(&fixture, &server, &config, &command, 2);
            assert!(matches!(
                phase(&fixture, &command),
                ora_node_db::ClonePhase::Fetching { attempts, .. } if attempts == pass
            ));
        }
        assert_eq!(fetch_passes(&server), 3);
        let mut node =
            Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap();
        node.configure_clone(config.clone()).unwrap();
        node.configure_git_access(server.access());
        node.recover_clones().unwrap();
        let state = node.submit_clone(command.clone()).unwrap().state;
        let Some(CloneExecutionResult::CloneFailed(failed)) = clone_result(&state) else {
            panic!("clone did not fail after three passes: {state:?}");
        };
        assert_eq!(failed.failure, CloneFailureCode::SourceUnavailable);
        assert_eq!(fetch_passes(&server), 3);
    });
}

/// A pass that was cut short is simply fetched again when passes remain.
#[test]
fn restart_during_the_first_fetch_pass_completes_on_the_second() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let command = request(&server, "fetch-again", "main");
        interrupt_at_progress_write(&fixture, &server, &config, &command, 3);
        let mut node =
            Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap();
        node.configure_clone(config).unwrap();
        node.configure_git_access(server.access());
        node.recover_clones().unwrap();
        let state = node.submit_clone(command).unwrap().state;
        assert!(
            matches!(
                clone_result(&state),
                Some(CloneExecutionResult::CloneReady(_))
            ),
            "{state:?}"
        );
        assert_eq!(fetch_passes(&server), 2);
    });
}
