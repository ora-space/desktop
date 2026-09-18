use super::support::*;
use super::*;
use ora_node_protocol::*;
use pretty_assertions::assert_eq;

/// Acceptance, Running intent, and completion writes fail closed around real Git side effects.
#[test]
fn sqlite_failures_gate_mutations_and_recover_on_reopen() {
    traced(|| {
        for point in [
            WritePoint::Accept,
            WritePoint::Progress,
            WritePoint::Complete,
            WritePoint::Outbox,
        ] {
            let fixture = Fixture::new();
            let mut node = fixture.open();
            let command = fixture.ensure(&node);
            fixture.faults.write.set(Some(point));
            assert!(node.submit(command.clone()).is_err(), "{point:?}");
            let mutated = matches!(point, WritePoint::Complete | WritePoint::Outbox);
            assert_eq!(fixture.root.join("task").exists(), mutated);
            assert_eq!(node.pending_events().unwrap(), vec![]);
            drop(node);
            fixture.faults.write.set(/*val*/ None);
            let mut node = fixture.open();
            if point == WritePoint::Accept {
                assert_eq!(node.state(), NodeState::Ready);
                assert_eq!(
                    node.status(&query(&command)).unwrap().payload.state,
                    ExecutionState::Unknown
                );
                node.submit(command.clone()).unwrap();
            } else {
                assert_eq!(node.state(), NodeState::RecoveryPending);
                assert_eq!(node.recover().unwrap(), NodeState::Ready);
            }
            assert!(matches!(
                node.status(&query(&command)).unwrap().payload.state,
                ExecutionState::Completed(ora_node_protocol::ExecutionResult::Worktree(
                    WorktreeExecutionResult::Ready(_)
                ))
            ));
            assert_eq!(*fixture.faults.calls.borrow(), vec!["create"]);
            assert_eq!(node.pending_events().unwrap().len(), 1);
        }
    });
}

/// Restart reconciles both sides of the add command using the pre-resolved base, never the moved ref.
#[test]
fn process_stops_before_and_after_create_keep_frozen_identity_and_base() {
    traced(|| {
        for fault in [GitFault::BeforeCreate, GitFault::AfterCreate] {
            let fixture = Fixture::new();
            let mut node = fixture.open();
            let command = fixture.ensure(&node);
            let original_base = cli(&fixture.main, &["rev-parse", "HEAD"]);
            fixture.faults.git.set(fault);
            assert!(
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                    || node.submit(command.clone())
                ))
                .is_err()
            );
            drop(node);
            cli(
                &fixture.main,
                &[
                    "-c",
                    "user.name=Node Test",
                    "-c",
                    "user.email=node@example.test",
                    "commit",
                    "--allow-empty",
                    "-m",
                    "move main",
                ],
            );
            fixture.faults.git.set(GitFault::None);
            let mut node = fixture.open();
            assert_eq!(node.recover().unwrap(), NodeState::Ready);
            let result = node.status(&query(&command)).unwrap().payload.state;
            assert_eq!(
                result,
                ExecutionState::Completed(ora_node_protocol::ExecutionResult::Worktree(
                    WorktreeExecutionResult::Ready(WorktreeReady {
                        node: node.identity().clone(),
                        workspace_id: command.spec().workspace_id.clone(),
                        worktree_id: command.spec().worktree_id.clone(),
                        facts: WorktreeFacts {
                            path: NodePath::new(fixture.root.join("task").to_str().unwrap()),
                            branch: command.spec().expected_branch.clone(),
                            base_commit: CommitId::new(original_base.clone())
                        }
                    })
                ))
            );
            assert_eq!(
                cli(&fixture.root.join("task"), &["rev-parse", "HEAD"]),
                original_base
            );
            let calls = fixture.faults.calls.borrow().clone();
            node.recover().unwrap();
            assert_eq!(*fixture.faults.calls.borrow(), calls);
        }
    });
}

/// Owned branch-only effects retry under the original identity while retaining unrelated results.
#[test]
fn branch_only_creation_recovers_and_preserves_other_results() {
    traced(|| {
        let fixture = Fixture::new();
        let mut node = fixture.open();
        let command = fixture.ensure(&node);
        let Command::Ensure(mut failed) = command.clone() else {
            unreachable!()
        };
        failed.operation_id = OperationId::new("failed");
        failed.execution_id = ExecutionId::new("failed");
        failed.payload.spec.repository = RepositoryRef::new("missing");
        let failed = Command::Ensure(failed);
        let failure = node.submit(failed.clone()).unwrap().state;
        fixture.faults.git.set(GitFault::BranchOnly);
        assert_eq!(
            node.submit(command.clone()).unwrap().state,
            ExecutionState::Unknown
        );
        drop(node);
        fixture.faults.git.set(GitFault::None);
        let mut node = fixture.open();
        for _ in 0..2 {
            assert_eq!(node.recover().unwrap(), NodeState::Ready);
        }
        assert!(matches!(
            node.status(&query(&command)).unwrap().payload.state,
            ExecutionState::Completed(ora_node_protocol::ExecutionResult::Worktree(
                WorktreeExecutionResult::Ready(_)
            ))
        ));
        assert_eq!(node.submit(failed).unwrap().state, failure);
        assert_eq!(node.pending_events().unwrap().len(), 2);
        assert_eq!(
            *fixture.faults.calls.borrow(),
            vec!["create", "remove_branch", "create"]
        );
        assert!(cli(&fixture.main, &["branch", "--format=%(refname:short)"]).contains("ora/task"));
    });
}

/// Failed branch deletion leaves durable Unknown evidence; restart only removes the remaining branch.
#[test]
fn partial_removal_continues_only_the_owned_branch() {
    traced(|| {
        let fixture = Fixture::new();
        let mut node = fixture.open();
        let command = fixture.ensure(&node);
        node.submit(command.clone()).unwrap();
        let remove = removal(&command);
        fixture.faults.git.set(GitFault::BeforeBranch);
        assert_eq!(
            node.submit(remove.clone()).unwrap().state,
            ExecutionState::Unknown
        );
        assert!(!fixture.root.join("task").exists());
        assert_eq!(node.pending_events().unwrap().len(), 1);
        drop(node);
        fixture.faults.git.set(GitFault::None);
        let mut node = fixture.open();
        assert_eq!(node.recover().unwrap(), NodeState::Ready);
        assert_eq!(
            node.status(&query(&remove)).unwrap().payload.state,
            ExecutionState::Completed(ora_node_protocol::ExecutionResult::Worktree(
                WorktreeExecutionResult::Removed(WorktreeRemoved {
                    node: node.identity().clone(),
                    workspace_id: command.spec().workspace_id.clone(),
                    worktree_id: command.spec().worktree_id.clone(),
                    outcome: WorktreeRemovalOutcome::Removed
                })
            ))
        );
        assert_eq!(
            *fixture.faults.calls.borrow(),
            vec![
                "create",
                "remove_worktree",
                "remove_branch",
                "remove_branch"
            ]
        );
        assert_eq!(
            cli(&fixture.main, &["branch", "--format=%(refname:short)"]),
            "main"
        );
    });
}

/// AlreadyAbsent requires all cleanup targets to be absent, not just the linked checkout.
#[test]
fn missing_checkout_still_cleans_branch_and_empty_owned_directory() {
    traced(|| {
        for residual in ["branch", "empty", "none", "nonempty", "elsewhere"] {
            let fixture = Fixture::new();
            let mut node = fixture.open();
            let command = fixture.ensure(&node);
            node.submit(command.clone()).unwrap();
            cli(
                &fixture.main,
                &[
                    "worktree",
                    "remove",
                    fixture.root.join("task").to_str().unwrap(),
                    "--force",
                ],
            );
            match residual {
                "none" => {
                    cli(&fixture.main, &["branch", "-D", "ora/task"]);
                }
                "empty" => std::fs::create_dir(fixture.root.join("task")).unwrap(),
                "nonempty" => {
                    std::fs::create_dir(fixture.root.join("task")).unwrap();
                    std::fs::write(fixture.root.join("task").join("user"), "keep").unwrap();
                }
                "elsewhere" => {
                    cli(
                        &fixture.main,
                        &[
                            "worktree",
                            "add",
                            fixture.root.join("other").to_str().unwrap(),
                            "ora/task",
                        ],
                    );
                }
                "branch" => {}
                _ => unreachable!(),
            }
            let result = node.submit(removal(&command)).unwrap().state;
            if matches!(residual, "nonempty" | "elsewhere") {
                assert_eq!(result, ExecutionState::Unknown);
                assert_eq!(*fixture.faults.calls.borrow(), vec!["create"]);
            } else {
                assert_eq!(
                    result,
                    ExecutionState::Completed(ora_node_protocol::ExecutionResult::Worktree(
                        WorktreeExecutionResult::Removed(WorktreeRemoved {
                            node: node.identity().clone(),
                            workspace_id: command.spec().workspace_id.clone(),
                            worktree_id: command.spec().worktree_id.clone(),
                            outcome: if residual == "none" {
                                WorktreeRemovalOutcome::AlreadyAbsent
                            } else {
                                WorktreeRemovalOutcome::Removed
                            }
                        })
                    ))
                );
                assert_eq!(
                    cli(&fixture.main, &["branch", "--format=%(refname:short)"]),
                    "main"
                );
                assert!(!fixture.root.join("task").exists());
            }
        }
    });
}

/// Changed bindings cannot redirect accepted executions; restoring configuration makes recovery possible.
#[test]
fn configuration_change_keeps_recovery_pending_without_redirecting_mutations() {
    traced(|| {
        let fixture = Fixture::new();
        let mut node = fixture.open();
        let command = fixture.ensure(&node);
        fixture.faults.write.set(Some(WritePoint::Progress));
        assert!(node.submit(command.clone()).is_err());
        drop(node);
        fixture.faults.write.set(/*val*/ None);
        let mut config = fixture.config();
        let replacement = fixture.directory.path().join("other-root");
        std::fs::create_dir(&replacement).unwrap();
        config.repositories[0].worktree_root = replacement;
        let mut node = Node::open_with_dependencies(
            config,
            ControlledGit(fixture.faults.clone()),
            fixture.faults.clone(),
            FixedClock,
        )
        .unwrap();
        assert_eq!(node.recover().unwrap(), NodeState::RecoveryPending);
        assert_eq!(
            node.status(&query(&command)).unwrap().payload.state,
            ExecutionState::Unknown
        );
        assert_eq!(*fixture.faults.calls.borrow(), Vec::<&str>::new());
        drop(node);
        let mut node = fixture.open();
        assert_eq!(node.recover().unwrap(), NodeState::Ready);
        assert_eq!(*fixture.faults.calls.borrow(), vec!["create"]);
    });
}

/// Missing or contradictory registration facts cannot be converted into successful creation after restart.
#[test]
fn changed_checkout_and_unavailable_git_keep_unknown_evidence() {
    traced(|| {
        for mode in ["read", "directory"] {
            let fixture = Fixture::new();
            let mut node = fixture.open();
            let command = fixture.ensure(&node);
            fixture.faults.git.set(GitFault::AfterCreate);
            assert!(
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                    || node.submit(command.clone())
                ))
                .is_err()
            );
            drop(node);
            fixture.faults.git.set(GitFault::None);
            match mode {
                "read" => fixture.faults.git.set(GitFault::Read),
                "directory" => {
                    std::fs::rename(fixture.root.join("task"), fixture.root.join("moved")).unwrap()
                }
                _ => unreachable!(),
            }
            let mut node = fixture.open();
            assert_eq!(node.recover().unwrap(), NodeState::RecoveryPending);
            assert_eq!(
                node.status(&query(&command)).unwrap().payload.state,
                ExecutionState::Unknown
            );
            assert_eq!(*fixture.faults.calls.borrow(), vec!["create"]);
            assert_eq!(node.pending_events().unwrap(), vec![]);
        }
    });
}

/// Reopened completed requests never depend on the present base ref or registration configuration.
#[test]
fn completed_retry_ignores_moved_base_and_missing_configuration() {
    traced(|| {
        let fixture = Fixture::new();
        let mut node = fixture.open();
        let command = fixture.ensure(&node);
        let result = node.submit(command.clone()).unwrap().state;
        drop(node);
        cli(
            &fixture.main,
            &[
                "-c",
                "user.name=Node Test",
                "-c",
                "user.email=node@example.test",
                "commit",
                "--allow-empty",
                "-m",
                "move main",
            ],
        );
        let mut config = fixture.config();
        config.repositories.clear();
        fixture.faults.git.set(GitFault::Read);
        let mut node = Node::open_with_dependencies(
            config,
            ControlledGit(fixture.faults.clone()),
            fixture.faults.clone(),
            FixedClock,
        )
        .unwrap();
        assert_eq!(node.submit(command).unwrap().state, result);
        assert_eq!(*fixture.faults.calls.borrow(), vec!["create"]);
    });
}
