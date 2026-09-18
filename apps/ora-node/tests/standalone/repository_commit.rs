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
            fixture.git(&["update-server-info"]);
            let commit = CommitId::new(fixture.git(&["rev-parse", "HEAD"]));
            let server =
                HttpsRepository::new(fixture.path(), fixture.path().join("main").join(".git"));
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
            assert!(
                matches!(node.submit_clone(command.clone()), Err(ora_node::Error::Storage(ora_node_db::Error::Injected(actual))) if actual == point)
            );
            assert!(node.pending_events().unwrap().is_empty());
            drop(node);
            server.reject_auth.store(true, Ordering::SeqCst);
            let mut node =
                Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap();
            node.configure_clone(config).unwrap();
            assert_eq!(node.recover_clones().unwrap(), ora_node::NodeState::Ready);
            let result = node.submit_clone(command.clone()).unwrap();
            let ExecutionState::Completed(ExecutionResult::Clone(
                CloneExecutionResult::CloneReady(ready),
            )) = result.state
            else {
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
                1
            );
        }
    });
}

/// Directory creation without a committed native identity cannot be recovered by guessing ownership.
#[test]
fn clone_preserves_directory_when_creation_evidence_write_fails() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server = HttpsRepository::new(fixture.path(), fixture.path().join("main").join(".git"));
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
        assert!(node.submit_clone(command.clone()).is_err());
        drop(node);
        assert_eq!(fs::read_dir(&config.repository_root).unwrap().count(), 1);
        let mut node =
            Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap();
        node.configure_clone(config.clone()).unwrap();
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
