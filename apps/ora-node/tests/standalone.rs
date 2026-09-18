#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "standalone/repository.rs"]
mod repository;
#[path = "standalone/support.rs"]
mod support;

use ora_node::{Node, Shutdown};
use ora_node_protocol::*;
use ora_utils::process::{LinuxPidFd, ProcessSignal, linux_process};
use pretty_assertions::assert_eq;
use support::*;

/// The actual executable recovers through host/guardian after SIGKILL, retaining the original result identity.
#[test]
fn node_kill_and_immediate_restart_wait_for_old_git_cleanup() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let command = fixture.seed();
        fixture.install_hook();
        let mut node = fixture.launch("first");
        until(|| fixture.path().join("hook-ready").exists());
        let hook = fixture.pin("hook-pid");
        node.kill();
        let mut replacement = fixture.launch("replacement");
        until(|| fixture.log("replacement").contains("Ready"));
        assert!(hook.has_exited().unwrap());
        std::fs::write(fixture.path().join("release"), "continue").unwrap();
        replacement.terminate();
        let node = Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap();
        let state = node.status(&query(&command)).unwrap().payload.state;
        let ExecutionState::Completed(ora_node_protocol::ExecutionResult::Worktree(
            WorktreeExecutionResult::Ready(result),
        )) = state
        else {
            panic!("creation not recovered")
        };
        assert_eq!(
            result.facts,
            WorktreeFacts {
                path: NodePath::new(fixture.path().join("trees").join("task").to_str().unwrap()),
                branch: BranchName::new("ora/task"),
                base_commit: CommitId::new(fixture.git(&["rev-parse", "main"])),
            }
        );
        assert_eq!(
            node.pending_events().unwrap(),
            vec![command.event(WorktreeExecutionResult::Ready(result))]
        );
        assert!(!fixture.path().join("late-write").exists());
        assert!(
            fixture
                .path()
                .join("node")
                .join("ora-node.sqlite3")
                .is_file()
        );
        assert!(
            !fixture
                .path()
                .join("different-home")
                .join("ora-node.sqlite3")
                .exists()
        );
    });
}

/// Owner death alone triggers guardian cleanup, even when no replacement Node starts.
#[test]
fn guardian_stops_git_without_node_restart() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        fixture.seed();
        fixture.install_hook();
        let mut node = fixture.launch("owner");
        until(|| fixture.path().join("hook-ready").exists());
        let hook = fixture.pin("hook-pid");
        node.kill();
        until(|| hook.has_exited().unwrap());
        assert!(!fixture.path().join("late-write").exists());
    });
}

/// Graceful shutdown waits for the configured bound, then stops the original Run before exiting.
#[test]
fn shutdown_bounds_a_blocked_git_hook() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        fixture.seed();
        fixture.install_hook();
        let mut node = fixture.launch("stopping");
        until(|| fixture.path().join("hook-ready").exists());
        let hook = fixture.pin("hook-pid");
        let handle = LinuxPidFd::from_observation(&linux_process(node.0.id()).unwrap()).unwrap();
        handle.signal(ProcessSignal::Terminate).unwrap();
        until(|| node.0.try_wait().unwrap().is_some());
        assert!(hook.has_exited().unwrap());
        assert!(!fixture.path().join("late-write").exists());
    });
}

/// An operation that finishes within the normal-stop grace is not discarded or forced to restart.
#[test]
fn shutdown_allows_git_to_finish_within_grace() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let command = fixture.seed();
        fixture.install_hook();
        let mut node = fixture.launch("finishing");
        until(|| fixture.path().join("hook-ready").exists());
        let hook = fixture.pin("hook-pid");
        std::fs::write(fixture.path().join("release"), "finish").unwrap();
        node.terminate();
        assert!(hook.has_exited().unwrap());
        assert_eq!(
            std::fs::read_to_string(fixture.path().join("late-write")).unwrap(),
            "late"
        );
        let mut node =
            Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap();
        assert_eq!(node.recover().unwrap(), ora_node::NodeState::Ready);
        assert!(matches!(
            node.status(&query(&command)).unwrap().payload.state,
            ExecutionState::Completed(ora_node_protocol::ExecutionResult::Worktree(
                WorktreeExecutionResult::Ready(_)
            ))
        ));
        node.shutdown().unwrap();
    });
}

/// Missing guardian evidence blocks the affected execution, not a distinct task using the same repository.
#[test]
fn guardian_loss_keeps_conflicting_recovery_unknown_but_allows_other_work() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let command = fixture.seed();
        fixture.install_hook();
        let mut child = fixture.launch("lost");
        until(|| fixture.path().join("hook-ready").exists());
        let hook = fixture.pin("hook-pid");
        fixture.kill_guardians();
        child.kill();
        std::fs::remove_file(
            fixture
                .path()
                .join("main")
                .join(".git")
                .join("hooks")
                .join("post-checkout"),
        )
        .unwrap();
        let mut node =
            Node::open(fixture.config(), fixture.process(), Shutdown::default()).unwrap();
        assert_eq!(
            node.recover().unwrap(),
            ora_node::NodeState::RecoveryPending
        );
        assert_eq!(
            node.status(&query(&command)).unwrap().payload.state,
            ExecutionState::Unknown
        );
        assert!(!hook.has_exited().unwrap());
        let ora_node::Command::Ensure(mut other) = command.clone() else {
            unreachable!()
        };
        other.operation_id = OperationId::new("other");
        other.execution_id = ExecutionId::new("other-execution");
        other.payload.spec.worktree_id = WorktreeId::new("other-tree");
        other.payload.spec.workspace_id = WorkspaceId::new("other-workspace");
        other.payload.spec.expected_branch = BranchName::new("ora/other");
        other.payload.spec.path_policy = WorktreePathPolicy::NodeManaged {
            directory_name: "other".into(),
        };
        assert!(matches!(
            node.submit(ora_node::Command::Ensure(other)).unwrap().state,
            ExecutionState::Completed(ora_node_protocol::ExecutionResult::Worktree(
                WorktreeExecutionResult::Ready(_)
            ))
        ));
        assert_eq!(
            node.status(&query(&command)).unwrap().payload.state,
            ExecutionState::Unknown
        );
        assert!(!hook.has_exited().unwrap());
        assert!(node.shutdown().is_err());
    });
}

/// An alias of host state is rejected before Node creates a business database there.
#[test]
fn node_data_directory_cannot_alias_host_state() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let alias = fixture.path().join("alias");
        std::os::unix::fs::symlink(fixture.path().join("host"), &alias).unwrap();
        let mut config = fixture.config();
        config.home_directory = alias;
        assert!(Node::open(config, fixture.process(), Shutdown::default()).is_err());
        assert!(
            !fixture
                .path()
                .join("host")
                .join("ora-node.sqlite3")
                .exists()
        );
    });
}
