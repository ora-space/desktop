use super::session::node;
use super::support::*;
use ora_node_protocol::*;
use pretty_assertions::assert_eq;
use serde_json::json;

/// Pairs each clone result with independent JSON, including both failure residue shapes.
fn results() -> Vec<(CloneExecutionResult, serde_json::Value)> {
    let spec = CloneExecutionSpec {
        node_id: NodeId::new("node-1"),
        repository: CloneRepositoryUrl::parse("ssh://git@example.com/team/repo.git")
            .unwrap_or_else(|error| panic!("invalid fixture: {error}")),
        branch: BranchName::new("feature/clone"),
    };
    let ready = CloneExecutionResult::CloneReady(CloneReady {
        node: node(),
        spec: spec.clone(),
        repository_id: RepositoryId::new("repository-1"),
        path: NodePath::new("/node/repositories/repository-1"),
        commit: CommitId::new("0123456789abcdef0123456789abcdef01234567"),
    });
    let ready_wire = json!({"kind":"clone_ready", "result": {
        "node":{"node_id":"node-1","incarnation_id":"incarnation-1"},
        "spec":{"node_id":"node-1","repository":"ssh://git@example.com/team/repo.git","branch":"feature/clone"},
        "repository_id":"repository-1","path":"/node/repositories/repository-1",
        "commit":"0123456789abcdef0123456789abcdef01234567"
    }});
    let failed = CloneExecutionResult::CloneFailed(CloneFailed {
        node: node(),
        spec: spec.clone(),
        failure: CloneFailureCode::BranchNotFound,
        residual: CloneResidual::Retained {
            repository_id: RepositoryId::new("repository-1"),
            path: NodePath::new("/node/repositories/repository-1"),
        },
    });
    let failed_wire = json!({"kind":"clone_failed", "result": {
        "node":{"node_id":"node-1","incarnation_id":"incarnation-1"},
        "spec":{"node_id":"node-1","repository":"ssh://git@example.com/team/repo.git","branch":"feature/clone"},
        "failure":"branch_not_found",
        "residual":{"kind":"retained","repository_id":"repository-1","path":"/node/repositories/repository-1"}
    }});
    let no_directory = CloneExecutionResult::CloneFailed(CloneFailed {
        node: node(),
        spec,
        failure: CloneFailureCode::DestinationConflict,
        residual: CloneResidual::NoDirectory {},
    });
    let no_directory_wire = json!({"kind":"clone_failed", "result": {
        "node":{"node_id":"node-1","incarnation_id":"incarnation-1"},
        "spec":{"node_id":"node-1","repository":"ssh://git@example.com/team/repo.git","branch":"feature/clone"},
        "failure":"destination_conflict","residual":{"kind":"no_directory"}
    }});
    vec![
        (ready, ready_wire),
        (failed, failed_wire),
        (no_directory, no_directory_wire),
    ]
}

/// Reuses one business-owned expectation through event delivery and generic status query.
fn cases(result: CloneExecutionResult, wire: serde_json::Value) -> [Case; 2] {
    [
        Case {
            message: Message::Node(NodeToControllerMessage::CloneResult(CloneResultMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                request_id: Some(RequestId::new("request-clone")),
                operation_id: OperationId::new("operation-clone"),
                execution_id: ExecutionId::new("execution-clone"),
                sequence: Sequence::new(/*value*/ 7),
                payload: result.clone(),
            })),
            wire: json!({"message_type":"clone_result","protocol_version":1,
                "request_id":"request-clone","operation_id":"operation-clone",
                "execution_id":"execution-clone","sequence":7,"payload":wire}),
        },
        Case {
            message: Message::Node(NodeToControllerMessage::ExecutionStatus(
                ExecutionStatusMessage {
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    operation_id: OperationId::new("operation-clone"),
                    execution_id: ExecutionId::new("execution-clone"),
                    payload: ExecutionStatus {
                        node: node(),
                        state: ExecutionState::Completed(ExecutionResult::Clone(result)),
                    },
                },
            )),
            wire: json!({"message_type":"execution_status","protocol_version":1,
                "operation_id":"operation-clone","execution_id":"execution-clone",
                "payload":{"node":{"node_id":"node-1","incarnation_id":"incarnation-1"},
                    "state":{"state":"completed","result":wire}}}),
        },
    ]
}

/// specs/test-cases/node/repository/minimal-clone-execution.md#clone-capability-coexists-with-historical-worktree-state
/// Proves clone owns both delivery shapes and keeps historical incarnation semantics.
#[tokio::test]
async fn clone_results_round_trip_as_events_and_completed_status() -> Result<(), TestError> {
    for (result, wire) in results() {
        let [event, status] = cases(result, wire);
        for case in [&event, &status] {
            case.assert_wire().await?;
            case.assert_round_trip().await?;
            case.assert_envelope_rejections().await?;
        }
        event
            .assert_fields(
                &[
                    "/operation_id",
                    "/execution_id",
                    "/sequence",
                    "/payload/kind",
                    "/payload/result",
                ],
                &[
                    ("/operation_id", "operation_id"),
                    ("/execution_id", "execution_id"),
                    ("/request_id", "request_id"),
                ],
            )
            .await?;
        status.assert_historical_node().await?;
        // Status carries no event position: only the retained event can be acknowledged.
        assert_eq!(event.wire["sequence"], json!(7));
    }
    Ok(())
}

/// Enforces concrete business invariants identically on events and Completed results.
#[tokio::test]
async fn clone_result_validation_cannot_be_bypassed_by_status_queries() -> Result<(), TestError> {
    for (result, wire) in results() {
        for (case, prefix) in cases(result, wire)
            .into_iter()
            .zip(["/payload/result", "/payload/state/result/result"])
        {
            for (suffix, value, expected) in [
                (
                    "/node/node_id",
                    json!(""),
                    MessageValidationError::EmptyField {
                        field: "node.node_id",
                    },
                ),
                (
                    "/node/incarnation_id",
                    json!(""),
                    MessageValidationError::EmptyField {
                        field: "node.incarnation_id",
                    },
                ),
                (
                    "/spec/node_id",
                    json!("node-2"),
                    MessageValidationError::CloneTargetMismatch,
                ),
                (
                    "/spec/branch",
                    json!("HEAD~1"),
                    MessageValidationError::InvalidCloneBranch,
                ),
            ] {
                let mut bad = case.wire.clone();
                replace(&mut bad, &format!("{prefix}{suffix}"), value);
                reject_semantics(Peer::Node, &bad, suffix, expected).await?;
            }
            for suffix in ["/node", "/spec", "/spec/repository", "/spec/branch"] {
                let mut bad = case.wire.clone();
                replace(&mut bad, &format!("{prefix}{suffix}"), json!(null));
                reject_structure(Peer::Node, &bad, suffix).await;
            }
            let mut bad = case.wire.clone();
            bad.pointer_mut(prefix)
                .unwrap_or_else(|| panic!("missing fixture"))["stderr"] = json!("secret");
            reject_structure(Peer::Node, &bad, "raw diagnostics forbidden").await;
        }
    }
    Ok(())
}

/// Result fields cannot disappear or become empty while the envelope still claims completion.
#[tokio::test]
async fn clone_results_require_complete_destination_and_commit_facts() -> Result<(), TestError> {
    for (result, wire) in results() {
        let ready = matches!(result, CloneExecutionResult::CloneReady(_));
        let retained = matches!(
            &result,
            CloneExecutionResult::CloneFailed(CloneFailed {
                residual: CloneResidual::Retained { .. },
                ..
            })
        );
        for (case, prefix) in cases(result, wire)
            .into_iter()
            .zip(["/payload/result", "/payload/state/result/result"])
        {
            if ready || retained {
                let base = if ready {
                    prefix.to_owned()
                } else {
                    format!("{prefix}/residual")
                };
                for (suffix, field) in [("repository_id", "repository_id"), ("path", "path")] {
                    let mut bad = case.wire.clone();
                    replace(&mut bad, &format!("{base}/{suffix}"), json!(""));
                    reject_semantics(
                        Peer::Node,
                        &bad,
                        suffix,
                        MessageValidationError::EmptyField { field },
                    )
                    .await?;
                    replace(&mut bad, &format!("{base}/{suffix}"), json!(null));
                    reject_structure(Peer::Node, &bad, suffix).await;
                }
            }
            if ready {
                for commit in [
                    "",
                    "HEAD",
                    "01234567",
                    "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
                ] {
                    let mut bad = case.wire.clone();
                    replace(&mut bad, &format!("{prefix}/commit"), json!(commit));
                    reject_semantics(
                        Peer::Node,
                        &bad,
                        "commit",
                        MessageValidationError::InvalidCloneCommit,
                    )
                    .await?;
                }
            } else {
                for (suffix, value) in [
                    ("failure", json!("result_unknown")),
                    ("residual", json!({"kind":"no_directory","path":"unowned"})),
                ] {
                    let mut bad = case.wire.clone();
                    replace(&mut bad, &format!("{prefix}/{suffix}"), value);
                    reject_structure(Peer::Node, &bad, suffix).await;
                }
            }
        }
    }
    Ok(())
}

/// Clone tags remain disjoint from all historical Worktree terminal tags.
#[test]
fn clone_results_are_not_worktree_results() -> Result<(), serde_json::Error> {
    for (result, wire) in results() {
        assert!(serde_json::from_value::<WorktreeExecutionResult>(wire.clone()).is_err());
        assert_eq!(
            serde_json::from_value::<ExecutionResult>(wire)?,
            ExecutionResult::Clone(result)
        );
    }
    Ok(())
}
