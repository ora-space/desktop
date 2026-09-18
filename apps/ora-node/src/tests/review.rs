use super::support::*;
use super::*;
use ora_node_protocol::*;
use pretty_assertions::assert_eq;

/// Recovery preserves task commits and the original comparison baseline without extra Git mutations.
#[test]
fn task_commits_before_result_commit_still_complete_original_creation() {
    traced(|| {
        let fixture = Fixture::new();
        let mut node = fixture.open();
        let command = fixture.ensure(&node);
        let base = cli(&fixture.main, &["rev-parse", "HEAD"]);
        fixture.faults.write.set(Some(WritePoint::Complete));
        assert!(node.submit(command.clone()).is_err());
        drop(node);
        let task = fixture.root.join("task");
        std::fs::write(task.join("work"), "new task content").unwrap();
        cli(&task, &["add", "work"]);
        cli(
            &task,
            &[
                "-c",
                "user.name=Node Test",
                "-c",
                "user.email=node@example.test",
                "commit",
                "-m",
                "task work",
            ],
        );
        let head = cli(&task, &["rev-parse", "HEAD"]);
        assert_ne!(head, base);
        fixture.faults.write.set(/*val*/ None);
        let mut node = fixture.open();
        assert_eq!(node.recover().unwrap(), NodeState::Ready);
        let result = WorktreeExecutionResult::Ready(WorktreeReady {
            node: node.identity().clone(),
            workspace_id: command.spec().workspace_id.clone(),
            worktree_id: command.spec().worktree_id.clone(),
            facts: WorktreeFacts {
                path: NodePath::new(task.to_str().unwrap()),
                branch: command.spec().expected_branch.clone(),
                base_commit: CommitId::new(base),
            },
        });
        assert_eq!(
            node.status(&query(&command)).unwrap().payload.state,
            ExecutionState::Completed(ora_node_protocol::ExecutionResult::Worktree(result.clone()))
        );
        assert_eq!(
            node.pending_events().unwrap(),
            vec![command.event(result.clone())]
        );
        assert_eq!(cli(&task, &["rev-parse", "HEAD"]), head);
        assert_eq!(
            std::fs::read_to_string(task.join("work")).unwrap(),
            "new task content"
        );
        assert_eq!(*fixture.faults.calls.borrow(), vec!["create"]);
        drop(node);
        let mut node = fixture.open();
        assert_eq!(
            node.submit(command).unwrap().state,
            ExecutionState::Completed(ora_node_protocol::ExecutionResult::Worktree(result))
        );
        assert_eq!(*fixture.faults.calls.borrow(), vec!["create"]);
    });
}

/// Historical creation and deletion envelopes survive together and acknowledge independently.
#[test]
fn unacknowledged_creation_survives_deletion_and_independent_acknowledgements() {
    traced(|| {
        let fixture = Fixture::new();
        let mut node = fixture.open();
        let create = fixture.ensure(&node);
        let ready = node.submit(create.clone()).unwrap().state;
        let remove = removal(&create);
        let removed = node.submit(remove.clone()).unwrap().state;
        let events = node.pending_events().unwrap();
        assert_eq!(events.len(), 2);
        drop(node);
        let mut node = fixture.open();
        assert_eq!(node.pending_events().unwrap(), events);
        assert_eq!(node.status(&query(&create)).unwrap().payload.state, ready);
        assert_eq!(node.status(&query(&remove)).unwrap().payload.state, removed);
        for (command, remaining) in [(&remove, vec![events[0].clone()]), (&create, vec![])] {
            let ack = EventAckMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: command.operation_id().clone(),
                execution_id: command.execution_id().clone(),
                sequence: Sequence::new(/*value*/ 1),
                payload: EventAck {
                    node_id: node.node_id().clone(),
                },
            };
            node.acknowledge(&ack).unwrap();
            node.acknowledge(&ack).unwrap();
            assert_eq!(node.pending_events().unwrap(), remaining);
        }
        drop(node);
        let node = fixture.open();
        assert_eq!(node.status(&query(&create)).unwrap().payload.state, ready);
        assert_eq!(node.status(&query(&remove)).unwrap().payload.state, removed);
        assert_eq!(node.pending_events().unwrap(), vec![]);
        assert_eq!(
            *fixture.faults.calls.borrow(),
            vec!["create", "remove_worktree", "remove_branch"]
        );
    });
}

/// Retired ownership cannot authorize deleting a replacement that reuses the same path and branch.
#[test]
fn retired_resource_replay_and_new_cleanup_preserve_replacement() {
    traced(|| {
        let fixture = Fixture::new();
        let mut node = fixture.open();
        let old = fixture.ensure(&node);
        node.submit(old.clone()).unwrap();
        let delete = removal(&old);
        let deleted = node.submit(delete.clone()).unwrap().state;
        let Command::Ensure(mut replacement) = old.clone() else {
            unreachable!()
        };
        replacement.operation_id = OperationId::new("replacement-create");
        replacement.execution_id = ExecutionId::new("replacement-create-execution");
        replacement.payload.spec.workspace_id = WorkspaceId::new("replacement-workspace");
        replacement.payload.spec.worktree_id = WorktreeId::new("replacement-worktree");
        let replacement = Command::Ensure(replacement);
        let ready = node.submit(replacement.clone()).unwrap().state;
        let task = fixture.root.join("task");
        std::fs::write(task.join("replacement"), "keep replacement").unwrap();
        cli(&task, &["add", "replacement"]);
        cli(
            &task,
            &[
                "-c",
                "user.name=Node Test",
                "-c",
                "user.email=node@example.test",
                "commit",
                "-m",
                "replacement content",
            ],
        );
        let head = cli(&task, &["rev-parse", "HEAD"]);
        let calls = fixture.faults.calls.borrow().clone();
        let original_events = node.pending_events().unwrap();
        drop(node);
        let mut node = fixture.open();
        assert_eq!(node.submit(delete.clone()).unwrap().state, deleted);
        let Command::Remove(mut stale) = delete else {
            unreachable!()
        };
        stale.operation_id = OperationId::new("new-stale-delete");
        stale.execution_id = ExecutionId::new("new-stale-delete-execution");
        let stale = Command::Remove(stale);
        fixture.faults.write.set(Some(WritePoint::Complete));
        assert!(node.submit(stale.clone()).is_err());
        assert_eq!(node.pending_events().unwrap(), original_events);
        drop(node);
        fixture.faults.write.set(/*val*/ None);
        fixture.faults.git.set(GitFault::Read);
        let mut node = fixture.open();
        assert_eq!(node.recover().unwrap(), NodeState::RecoveryPending);
        assert_eq!(
            node.status(&query(&stale)).unwrap().payload.state,
            ExecutionState::Unknown
        );
        assert_eq!(node.pending_events().unwrap(), original_events);
        assert_eq!(*fixture.faults.calls.borrow(), calls);
        drop(node);
        fixture.faults.git.set(GitFault::None);
        let mut node = fixture.open();
        assert_eq!(node.recover().unwrap(), NodeState::Ready);
        let rejected = ExecutionState::Completed(ora_node_protocol::ExecutionResult::Worktree(
            WorktreeExecutionResult::RemovalFailed(WorktreeRemovalFailed {
                node: node.identity().clone(),
                workspace_id: old.spec().workspace_id.clone(),
                worktree_id: old.spec().worktree_id.clone(),
                failure: WorktreeFailure {
                    code: WorktreeFailureCode::WorktreeConflict,
                    message: "resources appeared after their ownership was retired".into(),
                },
            }),
        ));
        assert_eq!(node.submit(stale.clone()).unwrap().state, rejected);
        assert_eq!(node.state(), NodeState::Ready);
        drop(node);
        let mut node = fixture.open();
        assert_eq!(node.submit(stale).unwrap().state, rejected);
        assert_eq!(
            node.status(&query(&replacement)).unwrap().payload.state,
            ready
        );
        assert_eq!(
            std::fs::read_to_string(task.join("replacement")).unwrap(),
            "keep replacement"
        );
        assert_eq!(cli(&task, &["rev-parse", "HEAD"]), head);
        assert_eq!(cli(&task, &["symbolic-ref", "--short", "HEAD"]), "ora/task");
        assert_eq!(*fixture.faults.calls.borrow(), calls);
    });
}
