use super::support::*;
use super::*;
use ora_node_protocol::*;
use pretty_assertions::assert_eq;

/// Real Git creation, dirty-task force removal and exact event acknowledgement form one durable loop.
#[test]
fn real_create_remove_replay_and_acknowledgement() {
    traced(|| {
        let fixture = Fixture::new();
        let mut node = fixture.open();
        let command = fixture.ensure(&node);
        let base = cli(&fixture.main, &["rev-parse", "HEAD"]);
        let ready = node.submit(command.clone()).unwrap();
        assert_eq!(
            ready,
            ExecutionStatus {
                node: node.identity().clone(),
                state: ExecutionState::Completed(ora_node_protocol::ExecutionResult::Worktree(
                    WorktreeExecutionResult::Ready(WorktreeReady {
                        node: node.identity().clone(),
                        workspace_id: command.spec().workspace_id.clone(),
                        worktree_id: command.spec().worktree_id.clone(),
                        facts: WorktreeFacts {
                            path: NodePath::new(fixture.root.join("task").to_str().unwrap()),
                            branch: command.spec().expected_branch.clone(),
                            base_commit: CommitId::new(base)
                        }
                    })
                ))
            }
        );
        let events = node.pending_events().unwrap();
        drop(node);
        let mut node = fixture.open();
        fixture.faults.git.set(GitFault::Read);
        assert_eq!(node.submit(command.clone()).unwrap().state, ready.state);
        assert_eq!(
            node.status(&query(&command)).unwrap().payload,
            ExecutionStatus {
                node: node.identity().clone(),
                state: ready.state.clone()
            }
        );
        assert_eq!(node.pending_events().unwrap(), events);
        let mut ack = EventAckMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: command.operation_id().clone(),
            execution_id: command.execution_id().clone(),
            sequence: Sequence::new(/*value*/ 1),
            payload: EventAck {
                node_id: node.node_id().clone(),
            },
        };
        ack.sequence = Sequence::new(/*value*/ 2);
        assert!(node.acknowledge(&ack).is_err());
        ack.sequence = Sequence::new(/*value*/ 1);
        ack.payload.node_id = NodeId::new("wrong");
        assert!(node.acknowledge(&ack).is_err());
        assert_eq!(node.pending_events().unwrap(), events);
        ack.payload.node_id = node.node_id().clone();
        node.acknowledge(&ack).unwrap();
        node.acknowledge(&ack).unwrap();
        assert_eq!(node.pending_events().unwrap(), vec![]);
        fixture.faults.git.set(GitFault::None);
        let task = fixture.root.join("task");
        cli(
            &task,
            &[
                "-c",
                "user.name=Node Test",
                "-c",
                "user.email=node@example.test",
                "commit",
                "--allow-empty",
                "-m",
                "task changes",
            ],
        );
        std::fs::write(task.join("uncommitted"), "dirty task").unwrap();
        let remove = removal(&command);
        assert_eq!(
            node.submit(remove).unwrap().state,
            ExecutionState::Completed(ora_node_protocol::ExecutionResult::Worktree(
                WorktreeExecutionResult::Removed(WorktreeRemoved {
                    node: node.identity().clone(),
                    workspace_id: command.spec().workspace_id.clone(),
                    worktree_id: command.spec().worktree_id.clone(),
                    outcome: WorktreeRemovalOutcome::Removed
                })
            ))
        );
        assert!(!task.exists());
        assert_eq!(
            cli(&fixture.main, &["branch", "--format=%(refname:short)"]),
            "main"
        );
        assert_eq!(
            *fixture.faults.calls.borrow(),
            vec!["create", "remove_worktree", "remove_branch"]
        );
        assert_eq!(
            node.status(&query(&command)).unwrap().payload.state,
            ready.state
        );
    });
}

/// Portable names and ownership mismatches fail before mutations and retain structured failures.
#[test]
fn invalid_paths_bindings_and_unowned_resources_never_mutate_git() {
    traced(|| {
        for directory_name in [
            "../escape",
            "a/b",
            "a\\b",
            "/absolute",
            "CON",
            "C:\\drive",
            ".",
            "..",
        ] {
            let fixture = Fixture::new();
            let mut node = fixture.open();
            let Command::Ensure(mut command) = fixture.ensure(&node) else {
                unreachable!()
            };
            command.payload.spec.path_policy = WorktreePathPolicy::NodeManaged {
                directory_name: directory_name.into(),
            };
            assert!(matches!(
                node.submit(Command::Ensure(command)).unwrap().state,
                ExecutionState::Completed(ora_node_protocol::ExecutionResult::Worktree(
                    WorktreeExecutionResult::Failed(WorktreeFailed {
                        failure: WorktreeFailure {
                            code: WorktreeFailureCode::PathOutsideAuthorizedRoot,
                            ..
                        },
                        ..
                    })
                ))
            ));
            assert_eq!(*fixture.faults.calls.borrow(), Vec::<&str>::new());
            assert_eq!(node.pending_events().unwrap().len(), 1);
        }
        let fixture = Fixture::new();
        let mut node = fixture.open();
        let command = fixture.ensure(&node);
        assert!(matches!(
            node.submit(removal(&command)).unwrap().state,
            ExecutionState::Completed(ora_node_protocol::ExecutionResult::Worktree(
                WorktreeExecutionResult::RemovalFailed(_)
            ))
        ));
        let Command::Ensure(mut invalid) = command else {
            unreachable!()
        };
        invalid.payload.spec.node_id = NodeId::new("wrong");
        assert!(matches!(
            node.submit(Command::Ensure(invalid.clone())),
            Err(Error::Storage(ora_node_db::Error::NodeMismatch))
        ));
        invalid.payload.spec.node_id = node.node_id().clone();
        invalid.execution_id = ExecutionId::new("");
        assert!(matches!(
            node.submit(Command::Ensure(invalid)),
            Err(Error::Validation(_))
        ));
        assert_eq!(*fixture.faults.calls.borrow(), Vec::<&str>::new());
    });
}

/// Existing directories and branches cannot become owned just because a request names them.
#[test]
fn existing_targets_and_main_workspace_are_protected() {
    traced(|| {
        for mode in ["directory", "branch", "main", "repository", "binding"] {
            let fixture = Fixture::new();
            let mut node = fixture.open();
            let Command::Ensure(mut message) = fixture.ensure(&node) else {
                unreachable!()
            };
            match mode {
                "directory" => {
                    std::fs::create_dir(fixture.root.join("task")).unwrap();
                    std::fs::write(fixture.root.join("task").join("user-file"), "keep").unwrap();
                }
                "branch" => {
                    cli(&fixture.main, &["branch", "ora/task"]);
                }
                "main" => {
                    message.payload.spec.workspace_id =
                        message.payload.spec.main_workspace.workspace_id.clone()
                }
                "repository" => message.payload.spec.repository = RepositoryRef::new("missing"),
                "binding" => {
                    message.payload.spec.main_workspace.workspace_id = WorkspaceId::new("wrong")
                }
                _ => unreachable!(),
            }
            assert!(matches!(
                node.submit(Command::Ensure(message)).unwrap().state,
                ExecutionState::Completed(ora_node_protocol::ExecutionResult::Worktree(
                    WorktreeExecutionResult::Failed(_)
                ))
            ));
            assert_eq!(*fixture.faults.calls.borrow(), Vec::<&str>::new());
            assert!(fixture.main.join(".git").is_dir());
            if mode == "directory" {
                assert_eq!(
                    std::fs::read_to_string(fixture.root.join("task").join("user-file")).unwrap(),
                    "keep"
                );
            }
        }
    });
}

/// Static symbolic links are rejected even when they point to an otherwise authorized directory.
#[cfg(unix)]
#[test]
fn static_symlink_escape_is_rejected() {
    traced(|| {
        let fixture = Fixture::new();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), fixture.root.join("task")).unwrap();
        let mut node = fixture.open();
        let command = fixture.ensure(&node);
        assert!(matches!(
            node.submit(command).unwrap().state,
            ExecutionState::Completed(ora_node_protocol::ExecutionResult::Worktree(
                WorktreeExecutionResult::Failed(_)
            ))
        ));
        assert_eq!(*fixture.faults.calls.borrow(), Vec::<&str>::new());
        assert!(outside.path().is_dir());
    });
}

/// Same-identity retries and changed input cannot cause another Git mutation.
#[test]
fn duplicate_inputs_and_resource_conflicts_preserve_original_result() {
    traced(|| {
        let fixture = Fixture::new();
        let mut node = fixture.open();
        let command = fixture.ensure(&node);
        let result = node.submit(command.clone()).unwrap();
        assert_eq!(node.submit(command.clone()).unwrap(), result);
        let Command::Ensure(mut changed) = command.clone() else {
            unreachable!()
        };
        changed.payload.spec.base_ref = GitRef::new("moved");
        assert!(matches!(
            node.submit(Command::Ensure(changed)),
            Err(Error::Storage(ora_node_db::Error::IdentityConflict))
        ));
        let Command::Ensure(mut changed) = command.clone() else {
            unreachable!()
        };
        changed.execution_id = ExecutionId::new("other-execution");
        assert!(matches!(
            node.submit(Command::Ensure(changed.clone())),
            Err(Error::Storage(ora_node_db::Error::IdentityConflict))
        ));
        changed.operation_id = OperationId::new("other-operation");
        assert!(matches!(
            node.submit(Command::Ensure(changed)).unwrap().state,
            ExecutionState::Completed(ora_node_protocol::ExecutionResult::Worktree(
                WorktreeExecutionResult::Failed(_)
            ))
        ));
        assert_eq!(node.submit(command).unwrap(), result);
        assert_eq!(*fixture.faults.calls.borrow(), vec!["create"]);
    });
}

/// A tag with the same short name must never hide the owned local branch from cleanup verification.
#[test]
fn branch_cleanup_uses_exact_local_names_even_with_ambiguous_tags() {
    traced(|| {
        let fixture = Fixture::new();
        let mut node = fixture.open();
        let command = fixture.ensure(&node);
        node.submit(command.clone()).unwrap();
        cli(&fixture.main, &["tag", "ora/task"]);
        cli(
            &fixture.main,
            &[
                "worktree",
                "remove",
                fixture.root.join("task").to_str().unwrap(),
            ],
        );
        assert_eq!(
            node.submit(removal(&command)).unwrap().state,
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
            cli(
                &fixture.main,
                &["for-each-ref", "--format=%(refname)", "refs/heads"]
            ),
            "refs/heads/main"
        );
        assert_eq!(cli(&fixture.main, &["tag"]), "ora/task");
    });
}

/// A separate Git directory whose main registration differs from the bound checkout cannot authorize mutations.
#[test]
fn inconsistent_main_registration_is_rejected() {
    traced(|| {
        let fixture = Fixture::new();
        cli(
            &fixture.main,
            &[
                "init",
                "--separate-git-dir",
                fixture.directory.path().join("metadata").to_str().unwrap(),
            ],
        );
        let mut node = fixture.open();
        let command = fixture.ensure(&node);
        assert!(matches!(
            node.submit(command).unwrap().state,
            ExecutionState::Completed(ora_node_protocol::ExecutionResult::Worktree(
                WorktreeExecutionResult::Failed(WorktreeFailed {
                    failure: WorktreeFailure {
                        code: WorktreeFailureCode::InvalidMainWorkspace,
                        ..
                    },
                    ..
                })
            ))
        ));
        assert_eq!(*fixture.faults.calls.borrow(), Vec::<&str>::new());
        assert!(fixture.main.join(".git").is_file());
        assert!(fixture.directory.path().join("metadata").is_dir());
    });
}

/// A NodeManaged path resolving to the actual main checkout is rejected for both create and delete.
#[test]
fn main_checkout_overlap_is_rejected_even_for_a_distinct_task_identity() {
    traced(|| {
        let fixture = Fixture::new();
        let mut node = fixture.open();
        node.repositories[0].worktree_root = fixture.directory.path().to_path_buf();
        let Command::Ensure(mut message) = fixture.ensure(&node) else {
            unreachable!()
        };
        message.payload.spec.expected_branch = BranchName::new("main");
        message.payload.spec.path_policy = WorktreePathPolicy::NodeManaged {
            directory_name: "main".into(),
        };
        let command = Command::Ensure(message);
        assert!(matches!(
            node.submit(command.clone()).unwrap().state,
            ExecutionState::Completed(ora_node_protocol::ExecutionResult::Worktree(
                WorktreeExecutionResult::Failed(WorktreeFailed {
                    failure: WorktreeFailure {
                        code: WorktreeFailureCode::PathOutsideAuthorizedRoot,
                        ..
                    },
                    ..
                })
            ))
        ));
        assert!(matches!(
            node.submit(removal(&command)).unwrap().state,
            ExecutionState::Completed(ora_node_protocol::ExecutionResult::Worktree(
                WorktreeExecutionResult::RemovalFailed(WorktreeRemovalFailed {
                    failure: WorktreeFailure {
                        code: WorktreeFailureCode::PathOutsideAuthorizedRoot,
                        ..
                    },
                    ..
                })
            ))
        ));
        assert_eq!(*fixture.faults.calls.borrow(), Vec::<&str>::new());
        assert!(fixture.main.join(".git").is_dir());
    });
}
