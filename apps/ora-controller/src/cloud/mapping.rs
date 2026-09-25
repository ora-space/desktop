//! Translation between the Node protocol's clone types and the fixed shapes the Cloud contract
//! persists. The contract carries less than the wire result on purpose (no echoed spec, no
//! Node-local repository identity), so the mapping is total towards Cloud and reconstructs only
//! what the contract promises on the way back.
use crate::*;
use ora_controller_proto::v1 as proto;

/// Rebuilds the execution spec Cloud handed out as one work item, bound to this deployment's Node.
pub(super) fn spec(
    input: Option<proto::ExecutionInput>,
    node: &NodeId,
) -> Result<CloneExecutionSpec, Error> {
    let Some(proto::ExecutionInput {
        spec: Some(proto::execution_input::Spec::Clone(clone)),
    }) = input
    else {
        return Err(Error::Conflict);
    };
    Ok(CloneExecutionSpec {
        node_id: node.clone(),
        // A source this Node protocol refuses is a disagreement with the authority, not a retry.
        repository: CloneRepositoryUrl::parse(&clone.repository).map_err(|_| Error::Conflict)?,
        branch: BranchName::new(clone.branch),
    })
}

/// The dispatched input as Cloud records it; the target Node travels beside it, not inside it.
pub(super) fn input(spec: &CloneExecutionSpec) -> proto::ExecutionInput {
    proto::ExecutionInput {
        spec: Some(proto::execution_input::Spec::Clone(proto::CloneSpec {
            repository: spec.repository.as_str().into(),
            branch: spec.branch.as_str().into(),
        })),
    }
}

/// Projects a Node's terminal fact onto the contract's result shape.
pub(super) fn result(result: &CloneExecutionResult) -> proto::ExecutionResult {
    let (node, outcome) = match result {
        CloneExecutionResult::CloneReady(ready) => (
            &ready.node,
            proto::execution_result::Outcome::CloneReady(proto::CloneReady {
                path: ready.path.as_str().into(),
                commit: ready.commit.as_str().into(),
            }),
        ),
        CloneExecutionResult::CloneFailed(failed) => (
            &failed.node,
            proto::execution_result::Outcome::CloneFailed(proto::CloneFailed {
                reason: reason(failed.failure) as i32,
                retained_path: match &failed.residual {
                    CloneResidual::NoDirectory {} => None,
                    CloneResidual::Retained { path, .. } => Some(path.as_str().into()),
                },
            }),
        ),
    };
    proto::ExecutionResult {
        node: Some(proto::NodeIdentity {
            node_id: node.node_id.as_str().into(),
            node_incarnation_id: node.incarnation_id.as_str().into(),
        }),
        outcome: Some(outcome),
    }
}

/// Reads a recorded result back; a record this build cannot interpret is a conflict with the
/// authority, never something to retry.
pub(super) fn outcome(result: proto::ExecutionResult) -> Result<ExecutionOutcome, Error> {
    let (Some(node), Some(outcome)) = (result.node, result.outcome) else {
        return Err(Error::Conflict);
    };
    let node = NodeRuntimeIdentity {
        node_id: NodeId::new(node.node_id),
        incarnation_id: NodeIncarnationId::new(node.node_incarnation_id),
    };
    Ok(match outcome {
        proto::execution_result::Outcome::CloneReady(ready) => ExecutionOutcome::Ready {
            node,
            path: NodePath::new(ready.path),
            commit: CommitId::new(ready.commit),
        },
        proto::execution_result::Outcome::CloneFailed(failed) => ExecutionOutcome::Failed {
            node,
            failure: failure(failed.reason)?,
            retained_path: failed.retained_path.map(NodePath::new),
        },
    })
}

/// Reconstructs the exact command a recorded dispatch stands for. Cloud never learns the caller's
/// request identity, so a cloud-dispatched command carries none, and the Node echoes none back.
pub(super) fn command(
    record: &proto::ExecutionRecord,
    node: &NodeId,
) -> Result<CloneRepositoryMessage, Error> {
    if record.node_id != node.as_str() {
        return Err(Error::Conflict);
    }
    let command = CloneRepositoryMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        request_id: None,
        operation_id: OperationId::new(record.operation_id.clone()),
        execution_id: ExecutionId::new(record.execution_id.clone()),
        payload: CloneRepository {
            spec: spec(record.input.clone(), node)?,
        },
    };
    command.validate()?;
    Ok(command)
}

fn reason(failure: CloneFailureCode) -> proto::CloneFailureReason {
    match failure {
        CloneFailureCode::SourceUnavailable => proto::CloneFailureReason::SourceUnavailable,
        CloneFailureCode::BranchNotFound => proto::CloneFailureReason::BranchNotFound,
        CloneFailureCode::DestinationConflict => proto::CloneFailureReason::DestinationConflict,
        CloneFailureCode::OperationFailed => proto::CloneFailureReason::OperationFailed,
        CloneFailureCode::Interrupted => proto::CloneFailureReason::Interrupted,
    }
}

fn failure(reason: i32) -> Result<CloneFailureCode, Error> {
    match proto::CloneFailureReason::try_from(reason).map_err(|_| Error::Conflict)? {
        proto::CloneFailureReason::SourceUnavailable => Ok(CloneFailureCode::SourceUnavailable),
        proto::CloneFailureReason::BranchNotFound => Ok(CloneFailureCode::BranchNotFound),
        proto::CloneFailureReason::DestinationConflict => Ok(CloneFailureCode::DestinationConflict),
        proto::CloneFailureReason::OperationFailed => Ok(CloneFailureCode::OperationFailed),
        proto::CloneFailureReason::Interrupted => Ok(CloneFailureCode::Interrupted),
        proto::CloneFailureReason::Unspecified => Err(Error::Conflict),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn node() -> NodeRuntimeIdentity {
        NodeRuntimeIdentity {
            node_id: NodeId::new("node"),
            incarnation_id: NodeIncarnationId::new("incarnation"),
        }
    }

    fn clone_spec() -> CloneExecutionSpec {
        CloneExecutionSpec {
            node_id: NodeId::new("node"),
            repository: CloneRepositoryUrl::parse("https://example.com/repo.git").unwrap(),
            branch: BranchName::new("main"),
        }
    }

    /// A recorded dispatch round-trips to the exact command, without a request identity.
    #[test]
    fn records_rebuild_the_original_command() {
        let record = proto::ExecutionRecord {
            operation_id: "operation".into(),
            execution_id: "execution".into(),
            node_id: "node".into(),
            input: Some(input(&clone_spec())),
            result: None,
        };
        assert_eq!(
            command(&record, &NodeId::new("node")).unwrap(),
            CloneRepositoryMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                request_id: None,
                operation_id: OperationId::new("operation"),
                execution_id: ExecutionId::new("execution"),
                payload: CloneRepository { spec: clone_spec() },
            }
        );
        assert!(matches!(
            command(&record, &NodeId::new("other")),
            Err(Error::Conflict)
        ));
        let mut foreign = record.clone();
        foreign.input = Some(proto::ExecutionInput {
            spec: Some(proto::execution_input::Spec::Clone(proto::CloneSpec {
                repository: "file:///tmp/repo".into(),
                branch: "main".into(),
            })),
        });
        assert!(matches!(
            command(&foreign, &NodeId::new("node")),
            Err(Error::Conflict)
        ));
        let mut empty = record;
        empty.input = None;
        assert!(matches!(
            command(&empty, &NodeId::new("node")),
            Err(Error::Conflict)
        ));
    }

    /// Both terminal facts project onto the contract and read back as the same outcome the local
    /// adapter would report for the wire result.
    #[test]
    fn results_project_onto_the_contract_outcome() {
        let ready = CloneExecutionResult::CloneReady(CloneReady {
            node: node(),
            spec: clone_spec(),
            repository_id: RepositoryId::new("repository"),
            path: NodePath::new("/node/checkout"),
            commit: CommitId::new("0123456789abcdef0123456789abcdef01234567"),
        });
        assert_eq!(
            outcome(result(&ready)).unwrap(),
            ExecutionOutcome::from(&ready)
        );
        let failed = CloneExecutionResult::CloneFailed(CloneFailed {
            node: node(),
            spec: clone_spec(),
            failure: CloneFailureCode::BranchNotFound,
            residual: CloneResidual::Retained {
                repository_id: RepositoryId::new("repository"),
                path: NodePath::new("/node/partial"),
            },
        });
        assert_eq!(
            outcome(result(&failed)).unwrap(),
            ExecutionOutcome::from(&failed)
        );
        // Every failure category survives the contract round trip, so no reason Cloud stores
        // decays into another category or a conflict when read back.
        for code in [
            CloneFailureCode::SourceUnavailable,
            CloneFailureCode::BranchNotFound,
            CloneFailureCode::DestinationConflict,
            CloneFailureCode::OperationFailed,
            CloneFailureCode::Interrupted,
        ] {
            assert_eq!(failure(reason(code) as i32).unwrap(), code);
        }
        let mut unspecified = result(&failed);
        unspecified.outcome = Some(proto::execution_result::Outcome::CloneFailed(
            proto::CloneFailed {
                reason: proto::CloneFailureReason::Unspecified as i32,
                retained_path: None,
            },
        ));
        assert!(matches!(outcome(unspecified), Err(Error::Conflict)));
    }
}
