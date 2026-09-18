use super::support::*;
use super::*;
use gitlancer::{CliGitRunner, Git, GitCommand, GitExecError, GitIntent, GitOutput, GitRunner};
use pretty_assertions::assert_eq;
use std::sync::{
    Arc, Barrier, Mutex,
    atomic::{AtomicUsize, Ordering},
};

struct CountingRunner(Arc<AtomicUsize>);
impl ExecutionGitRunner for CountingRunner {
    /// Synchronous test runs cannot outlive this fixture's command return.
    fn begin_execution(
        &self,
        _record: &ora_node_db::Execution,
    ) -> Result<(), ora_node_protocol::WorktreeFailure> {
        Ok(())
    }
    /// The counter has no remote execution context.
    fn end_execution(&self) {}
}
impl GitRunner for CountingRunner {
    /// Counts real external mutations across concurrent callers using one shared Node.
    fn run(&self, command: &GitCommand) -> Result<GitOutput, GitExecError> {
        if command.intent == GitIntent::Mutating {
            self.0.fetch_add(/*val*/ 1, Ordering::SeqCst);
        }
        CliGitRunner.run(command)
    }
}

/// Concurrent retransmissions serialize behind one execution and all return its original complete result.
#[test]
fn concurrent_retransmissions_mutate_once() {
    traced(|| {
        let fixture = Fixture::new();
        let seed = fixture.open();
        let command = fixture.ensure(&seed);
        drop(seed);
        let mutations = Arc::new(AtomicUsize::new(/*v*/ 0));
        let node = Node::open_with_dependencies(
            fixture.config(),
            Git::new(CountingRunner(mutations.clone())),
            DurableWrites,
            FixedClock,
        )
        .unwrap();
        let node = Arc::new(Mutex::new(node));
        let start = Arc::new(Barrier::new(/*n*/ 4));
        // Spawn the entire barrier cohort before joining any thread.
        #[allow(clippy::needless_collect)]
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let node = node.clone();
                let command = command.clone();
                let start = start.clone();
                std::thread::spawn(move || {
                    let mut result = None;
                    traced(|| {
                        start.wait();
                        result = Some(node.lock().unwrap().submit(command).unwrap());
                    });
                    result.unwrap()
                })
            })
            .collect();
        let results: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert_eq!(results, vec![results[0].clone(); 4]);
        assert_eq!(mutations.load(Ordering::SeqCst), 1);
        assert_eq!(node.lock().unwrap().pending_events().unwrap().len(), 1);
    });
}

/// Recovery uses the same mutation lock as new commands, so a queued retry observes its completed result.
#[test]
fn recovery_and_new_submission_share_one_mutation_owner() {
    traced(|| {
        let fixture = Fixture::new();
        let mut seed = fixture.open();
        let command = fixture.ensure(&seed);
        fixture.faults.write.set(Some(WritePoint::Progress));
        assert!(seed.submit(command.clone()).is_err());
        drop(seed);
        let mutations = Arc::new(AtomicUsize::new(/*v*/ 0));
        let node = Node::open_with_dependencies(
            fixture.config(),
            Git::new(CountingRunner(mutations.clone())),
            DurableWrites,
            FixedClock,
        )
        .unwrap();
        let node = Arc::new(Mutex::new(node));
        let (entered, wait_entered) = std::sync::mpsc::channel();
        let (release, wait_release) = std::sync::mpsc::channel();
        let recovery_node = node.clone();
        let recovery = std::thread::spawn(move || {
            traced(|| {
                let mut node = recovery_node.lock().unwrap();
                entered.send(()).unwrap();
                wait_release.recv().unwrap();
                assert_eq!(node.recover().unwrap(), NodeState::Ready);
            })
        });
        wait_entered.recv().unwrap();
        let queued_node = node;
        let retry = std::thread::spawn(move || {
            traced(|| {
                assert!(matches!(
                    queued_node.lock().unwrap().submit(command).unwrap().state,
                    ora_node_protocol::ExecutionState::Completed(_)
                ));
            })
        });
        release.send(()).unwrap();
        recovery.join().unwrap();
        retry.join().unwrap();
        assert_eq!(mutations.load(Ordering::SeqCst), 1);
    });
}
