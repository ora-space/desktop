use super::{support::*, *};
use ora_node_protocol::*;
use pretty_assertions::assert_eq;

/// Recovery protects unowned files and a branch used by another checkout, but can remove an owned empty residual.
#[test]
fn residual_cleanup_respects_files_checkout_occupancy_and_write_failures() {
    traced(|| {
        for scenario in ["empty", "files", "checkout"] {
            let fixture = Fixture::new();
            let mut node = fixture.open();
            let command = fixture.ensure(&node);
            fixture.faults.git.set(GitFault::BranchOnly);
            node.submit(command.clone()).unwrap();
            fixture.faults.git.set(GitFault::None);
            match scenario {
                "empty" => std::fs::create_dir(fixture.root.join("task")).unwrap(),
                "files" => {
                    std::fs::create_dir(fixture.root.join("task")).unwrap();
                    std::fs::write(fixture.root.join("task").join("user-file"), "keep").unwrap();
                }
                "checkout" => {
                    cli(
                        &fixture.main,
                        &[
                            "worktree",
                            "add",
                            fixture.directory.path().join("foreign").to_str().unwrap(),
                            "ora/task",
                        ],
                    );
                }
                _ => unreachable!(),
            }
            if scenario == "empty" {
                fixture.faults.write.set(Some(WritePoint::Progress));
                assert!(node.recover().is_err());
                assert_eq!(*fixture.faults.calls.borrow(), vec!["create"]);
                fixture.faults.write.set(/*val*/ None);
                fixture.faults.git.set(GitFault::AfterBranch);
                assert!(
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| node.recover()))
                        .is_err()
                );
                drop(node);
                fixture.faults.git.set(GitFault::None);
                let mut node = fixture.open();
                assert_eq!(node.recover().unwrap(), NodeState::Ready);
                assert_eq!(
                    *fixture.faults.calls.borrow(),
                    vec!["create", "remove_branch", "remove_empty", "create"]
                );
            } else {
                assert_eq!(node.recover().unwrap(), NodeState::RecoveryPending);
                assert_eq!(*fixture.faults.calls.borrow(), vec!["create"]);
                if scenario == "files" {
                    assert_eq!(
                        std::fs::read_to_string(fixture.root.join("task").join("user-file"))
                            .unwrap(),
                        "keep"
                    );
                } else {
                    assert!(
                        fixture
                            .directory
                            .path()
                            .join("foreign")
                            .join(".git")
                            .is_file()
                    );
                }
            }
        }
    });
}

/// A blocked cleanup retains only its resource reservation; both same-repository and other-repository work proceeds.
#[test]
fn cleanup_failure_does_not_block_unrelated_work_or_release_conflicting_resources() {
    traced(|| {
        for separate_repository in [false, true] {
            let fixture = Fixture::new();
            let other = Fixture::new();
            let mut node = fixture.open();
            let original = fixture.ensure(&node);
            fixture.faults.git.set(GitFault::BranchOnly);
            assert_eq!(
                node.submit(original.clone()).unwrap().state,
                ExecutionState::Unknown
            );
            fixture.faults.git.set(GitFault::BeforeBranch);
            assert_eq!(node.recover().unwrap(), NodeState::RecoveryPending);
            drop(node);
            let mut config = fixture.config();
            let mut other_binding = other.config().repositories.remove(/*index*/ 0);
            other_binding.repository = RepositoryRef::new("other-repo");
            other_binding.main_workspace.workspace_id = WorkspaceId::new("other-main");
            config.repositories.push(other_binding.clone());
            fixture.faults.git.set(GitFault::None);
            let mut node = Node::open_with_dependencies(
                config,
                ControlledGit(fixture.faults.clone()),
                fixture.faults.clone(),
                FixedClock,
            )
            .unwrap();
            let Command::Ensure(mut next) = original.clone() else {
                unreachable!()
            };
            next.operation_id = OperationId::new("next");
            next.execution_id = ExecutionId::new("next-execution");
            next.payload.spec.workspace_id = WorkspaceId::new("next-workspace");
            next.payload.spec.worktree_id = WorktreeId::new("next-tree");
            next.payload.spec.expected_branch = BranchName::new("ora/next");
            next.payload.spec.path_policy = WorktreePathPolicy::NodeManaged {
                directory_name: "next".into(),
            };
            if separate_repository {
                next.payload.spec.repository = other_binding.repository;
                next.payload.spec.main_workspace = other_binding.main_workspace;
            }
            assert!(matches!(
                node.submit(Command::Ensure(next)).unwrap().state,
                ExecutionState::Completed(ora_node_protocol::ExecutionResult::Worktree(
                    WorktreeExecutionResult::Ready(_)
                ))
            ));
            assert_eq!(node.state(), NodeState::RecoveryPending);
            assert_eq!(
                node.status(&query(&original)).unwrap().payload.state,
                ExecutionState::Unknown
            );
            assert!(matches!(
                node.submit(removal(&original)).unwrap().state,
                ExecutionState::Completed(ora_node_protocol::ExecutionResult::Worktree(
                    WorktreeExecutionResult::RemovalFailed(_)
                ))
            ));
            assert!(
                cli(&fixture.main, &["branch", "--format=%(refname:short)"]).contains("ora/task")
            );
            assert_eq!(node.recover().unwrap(), NodeState::Ready);
            assert!(matches!(
                node.status(&query(&original)).unwrap().payload.state,
                ExecutionState::Completed(ora_node_protocol::ExecutionResult::Worktree(
                    WorktreeExecutionResult::Ready(_)
                ))
            ));
        }
    });
}

/// A crash after branch cleanup resumes the original execution at its frozen base, not a moved main ref.
#[test]
fn cleanup_crash_preserves_original_identity_and_frozen_base() {
    traced(|| {
        let fixture = Fixture::new();
        let mut node = fixture.open();
        let command = fixture.ensure(&node);
        let base = cli(&fixture.main, &["rev-parse", "HEAD"]);
        fixture.faults.git.set(GitFault::BranchOnly);
        node.submit(command.clone()).unwrap();
        fixture.faults.git.set(GitFault::AfterBranch);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| node.recover())).is_err());
        drop(node);
        cli(
            &fixture.main,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.test",
                "commit",
                "--allow-empty",
                "-m",
                "move main",
            ],
        );
        fixture.faults.git.set(GitFault::None);
        fixture.faults.write.set(Some(WritePoint::Complete));
        let mut node = fixture.open();
        assert!(node.recover().is_err());
        drop(node);
        fixture.faults.write.set(/*val*/ None);
        let mut node = fixture.open();
        assert_eq!(node.recover().unwrap(), NodeState::Ready);
        let result = WorktreeExecutionResult::Ready(WorktreeReady {
            node: node.identity().clone(),
            workspace_id: command.spec().workspace_id.clone(),
            worktree_id: command.spec().worktree_id.clone(),
            facts: WorktreeFacts {
                path: NodePath::new(fixture.root.join("task").to_str().unwrap()),
                branch: command.spec().expected_branch.clone(),
                base_commit: CommitId::new(base.clone()),
            },
        });
        assert_eq!(
            node.submit(command.clone()).unwrap().state,
            ExecutionState::Completed(ora_node_protocol::ExecutionResult::Worktree(result.clone()))
        );
        assert_eq!(node.pending_events().unwrap(), vec![command.event(result)]);
        assert_eq!(
            cli(&fixture.root.join("task"), &["rev-parse", "HEAD"]),
            base
        );
        assert_eq!(
            *fixture.faults.calls.borrow(),
            vec!["create", "remove_branch", "create"]
        );
    });
}

/// Repeated partial creation gets one retry per recovery pass; changed branch content is never discarded.
#[test]
fn repeated_partial_failure_is_bounded_and_changed_branch_is_preserved() {
    traced(|| {
        let fixture = Fixture::new();
        let mut node = fixture.open();
        let command = fixture.ensure(&node);
        fixture.faults.git.set(GitFault::BranchOnly);
        node.submit(command).unwrap();
        assert_eq!(node.recover().unwrap(), NodeState::RecoveryPending);
        assert_eq!(
            *fixture.faults.calls.borrow(),
            vec!["create", "remove_branch", "create"]
        );
        cli(
            &fixture.main,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.test",
                "commit",
                "--allow-empty",
                "-m",
                "move",
            ],
        );
        cli(&fixture.main, &["branch", "-f", "ora/task", "HEAD"]);
        let calls = fixture.faults.calls.borrow().clone();
        assert_eq!(node.recover().unwrap(), NodeState::RecoveryPending);
        assert_eq!(*fixture.faults.calls.borrow(), calls);
        assert_eq!(
            cli(&fixture.main, &["rev-parse", "ora/task"]),
            cli(&fixture.main, &["rev-parse", "HEAD"])
        );
    });
}
