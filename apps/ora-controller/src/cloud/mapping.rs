//! Translation between the Node protocol's clone types and the fixed shapes the Cloud contract
//! persists. The contract carries less than the wire result on purpose (no echoed spec, no
//! Node-local repository identity), so the mapping is total towards Cloud and reconstructs only
//! what the contract promises on the way back.
use crate::*;
use ora_controller_proto::v1 as proto;

/// Maps the snapshot clone input Cloud froze onto the Node command's spec, bound to the Node it
/// is dispatched to. Every field must satisfy the Node protocol and the snapshot ref must name
/// the base commit: anything else is a disagreement with the authority, never something a retry
/// could fix, so it is refused instead of being dispatched. The Node types keep the strings as
/// given, so [`input`] rebuilds exactly the input Cloud froze.
pub(super) fn clone_spec(
    clone: &proto::CloneSpec,
    node: &NodeId,
) -> Result<CloneExecutionSpec, Error> {
    let repository_url =
        GitRepositoryUrl::parse(&clone.repository_url).map_err(|_| Error::Conflict)?;
    let snapshot_ref = SnapshotRef::parse(&clone.snapshot_ref).map_err(|_| Error::Conflict)?;
    let workspace_ref = WorkspaceRef::parse(&clone.workspace_ref).map_err(|_| Error::Conflict)?;
    // The ref's last segment is a full lowercase commit, so equality also validates the commit.
    if snapshot_ref.commit() != clone.base_commit {
        return Err(Error::Conflict);
    }
    Ok(CloneExecutionSpec {
        node_id: node.clone(),
        repository_url,
        snapshot_ref,
        base_commit: CommitId::new(clone.base_commit.clone()),
        workspace_ref,
    })
}

/// Rebuilds the spec of a recorded clone execution; any other input is not a clone.
fn spec(input: Option<&proto::ExecutionInput>, node: &NodeId) -> Result<CloneExecutionSpec, Error> {
    let Some(proto::ExecutionInput {
        spec: Some(proto::execution_input::Spec::Clone(clone)),
    }) = input
    else {
        return Err(Error::Conflict);
    };
    clone_spec(clone, node)
}

/// The dispatched input as Cloud records it; the target Node travels beside it, not inside it.
pub(super) fn input(spec: &CloneExecutionSpec) -> proto::ExecutionInput {
    proto::ExecutionInput {
        spec: Some(proto::execution_input::Spec::Clone(proto::CloneSpec {
            repository_url: spec.repository_url.as_str().into(),
            snapshot_ref: spec.snapshot_ref.as_str().into(),
            base_commit: spec.base_commit.as_str().into(),
            workspace_ref: spec.workspace_ref.as_str().into(),
        })),
    }
}

/// Whether a reported clone result may be taken over for its dispatch. A ready checkout must sit
/// at the dispatched base commit: that commit becomes every later delivery's base, so a mismatch
/// (only ever a defect, since the Node checks it first) must not reach Cloud.
pub(super) fn matches_dispatch(
    command: &CloneRepositoryMessage,
    result: &CloneExecutionResult,
) -> bool {
    match result {
        CloneExecutionResult::CloneReady(ready) => ready.commit == command.payload.spec.base_commit,
        CloneExecutionResult::CloneFailed(_) => true,
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
        // This projection serves the clone family only. Session, delivery and plugin records are
        // resolved through their own families before any caller reads a clone outcome, so reaching
        // one here is a disagreement with the authority.
        proto::execution_result::Outcome::PluginsResult(_)
        | proto::execution_result::Outcome::PluginsFailed(_)
        | proto::execution_result::Outcome::AgentSessionEnded(_)
        | proto::execution_result::Outcome::RevisionDelivered(_)
        | proto::execution_result::Outcome::RevisionUnchanged(_)
        | proto::execution_result::Outcome::RevisionFailed(_) => return Err(Error::Conflict),
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
        operation_id: OperationId::new(record.node_operation_id.clone()),
        execution_id: ExecutionId::new(record.execution_id.clone()),
        payload: CloneRepository {
            spec: spec(record.input.as_ref(), node)?,
        },
    };
    command.validate()?;
    Ok(command)
}

/// Converts a grant's expiry; grants without one, or outside the representable range, are refused
/// rather than treated as never expiring.
pub(super) fn expiry(
    timestamp: Option<prost_types::Timestamp>,
) -> Result<time::OffsetDateTime, Error> {
    let expires = timestamp.ok_or(Error::Conflict)?;
    let nanos = i128::from(expires.seconds) * 1_000_000_000 + i128::from(expires.nanos);
    time::OffsetDateTime::from_unix_timestamp_nanos(nanos).map_err(|_| Error::Conflict)
}

/// Names a Node failure category in the contract's closed set.
fn reason(failure: CloneFailureCode) -> proto::CloneFailureReason {
    match failure {
        CloneFailureCode::SourceUnavailable => proto::CloneFailureReason::SourceUnavailable,
        // Retired with branch clones; the Node codec rejects a new result carrying it. Should one
        // slip through, Cloud refuses the unspecified reason rather than recording a wrong one.
        CloneFailureCode::BranchNotFound => proto::CloneFailureReason::Unspecified,
        CloneFailureCode::DestinationConflict => proto::CloneFailureReason::DestinationConflict,
        CloneFailureCode::OperationFailed => proto::CloneFailureReason::OperationFailed,
        CloneFailureCode::Interrupted => proto::CloneFailureReason::Interrupted,
        CloneFailureCode::SnapshotUnavailable => proto::CloneFailureReason::SnapshotUnavailable,
        CloneFailureCode::BaseCommitMismatch => proto::CloneFailureReason::BaseCommitMismatch,
    }
}

/// Reads a recorded failure category back; an unknown or unspecified one is a conflict.
fn failure(reason: i32) -> Result<CloneFailureCode, Error> {
    match proto::CloneFailureReason::try_from(reason).map_err(|_| Error::Conflict)? {
        proto::CloneFailureReason::SourceUnavailable => Ok(CloneFailureCode::SourceUnavailable),
        proto::CloneFailureReason::DestinationConflict => Ok(CloneFailureCode::DestinationConflict),
        proto::CloneFailureReason::OperationFailed => Ok(CloneFailureCode::OperationFailed),
        proto::CloneFailureReason::Interrupted => Ok(CloneFailureCode::Interrupted),
        proto::CloneFailureReason::SnapshotUnavailable => Ok(CloneFailureCode::SnapshotUnavailable),
        proto::CloneFailureReason::BaseCommitMismatch => Ok(CloneFailureCode::BaseCommitMismatch),
        proto::CloneFailureReason::Unspecified => Err(Error::Conflict),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    const BASE: &str = "0123456789abcdef0123456789abcdef01234567";
    const REPOSITORY: &str = "https://git.ora.test/6f1c2a3b-4d5e-4f60-8a7b-9c0d1e2f3a4b/0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d.git";
    const SNAPSHOT_REF: &str = "refs/ora/upstreams/9a8b7c6d-5e4f-4a3b-9c2d-1e0f9a8b7c6d/snapshots/0123456789abcdef0123456789abcdef01234567";
    const WORKSPACE_REF: &str = "refs/heads/workspaces/4b3c2d1e-0f9a-4b8c-9d7e-6f5a4b3c2d1e/main";

    fn node() -> NodeRuntimeIdentity {
        NodeRuntimeIdentity {
            node_id: NodeId::new("node"),
            incarnation_id: NodeIncarnationId::new("incarnation"),
        }
    }

    /// The snapshot input Cloud froze for a clone step.
    fn frozen() -> proto::CloneSpec {
        proto::CloneSpec {
            repository_url: REPOSITORY.into(),
            snapshot_ref: SNAPSHOT_REF.into(),
            base_commit: BASE.into(),
            workspace_ref: WORKSPACE_REF.into(),
        }
    }

    fn clone_execution_spec() -> CloneExecutionSpec {
        CloneExecutionSpec {
            node_id: NodeId::new("node"),
            repository_url: GitRepositoryUrl::parse(REPOSITORY).unwrap(),
            snapshot_ref: SnapshotRef::parse(SNAPSHOT_REF).unwrap(),
            base_commit: CommitId::new(BASE),
            workspace_ref: WorkspaceRef::parse(WORKSPACE_REF).unwrap(),
        }
    }

    fn ready(commit: &str) -> CloneExecutionResult {
        CloneExecutionResult::CloneReady(CloneReady {
            node: node(),
            spec: clone_execution_spec(),
            repository_id: RepositoryId::new("repository"),
            path: NodePath::new("/node/checkout"),
            commit: CommitId::new(commit),
        })
    }

    /// The frozen input maps onto the Node spec field for field and rebuilds into exactly the
    /// input Cloud froze, so what RecordDispatch registers and the Node receives is that input.
    #[test]
    fn frozen_clone_inputs_are_forwarded_verbatim() {
        let spec = clone_spec(&frozen(), &NodeId::new("node")).unwrap();
        assert_eq!(spec, clone_execution_spec());
        assert_eq!(
            input(&spec),
            proto::ExecutionInput {
                spec: Some(proto::execution_input::Spec::Clone(frozen())),
            }
        );
    }

    /// An input outside the Git Server contract, or whose snapshot ref names another commit than
    /// its base, is refused instead of becoming a Node command.
    #[test]
    fn clone_inputs_outside_the_contract_are_refused() {
        for change in [
            (|clone: &mut proto::CloneSpec| clone.repository_url = String::new())
                as fn(&mut proto::CloneSpec),
            |clone| clone.repository_url = "http://git.ora.test/t/p.git".into(),
            |clone| clone.repository_url = "https://user@git.ora.test/t/p.git".into(),
            |clone| clone.repository_url = "https://git.ora.test/t/p.git?ref=main".into(),
            |clone| clone.snapshot_ref = String::new(),
            |clone| clone.snapshot_ref = "refs/heads/main".into(),
            |clone| {
                clone.snapshot_ref = SNAPSHOT_REF.replace(BASE, &BASE.to_uppercase());
                clone.base_commit = BASE.to_uppercase();
            },
            |clone| clone.base_commit = "1123456789abcdef0123456789abcdef01234567".into(),
            |clone| clone.base_commit = String::new(),
            |clone| clone.workspace_ref = String::new(),
            |clone| clone.workspace_ref = "refs/heads/main".into(),
            |clone| {
                clone.workspace_ref =
                    "refs/heads/workspaces/4b3c2d1e-0f9a-4b8c-9d7e-6f5a4b3c2d1e/HEAD".into();
            },
        ] {
            let mut invalid = frozen();
            change(&mut invalid);
            assert!(matches!(
                clone_spec(&invalid, &NodeId::new("node")),
                Err(Error::Conflict)
            ));
        }
    }

    /// A recorded dispatch round-trips to the exact command, without a request identity; a record
    /// for another Node, with an invalid input or with none is a conflict.
    #[test]
    fn records_rebuild_the_original_command() {
        let record = proto::ExecutionRecord {
            node_operation_id: "operation".into(),
            operation_id: "operation".into(),
            execution_id: "execution".into(),
            node_id: "node".into(),
            input: Some(input(&clone_execution_spec())),
            result: None,
        };
        assert_eq!(
            command(&record, &NodeId::new("node")).unwrap(),
            CloneRepositoryMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                request_id: None,
                operation_id: OperationId::new("operation"),
                execution_id: ExecutionId::new("execution"),
                payload: CloneRepository {
                    spec: clone_execution_spec()
                },
            }
        );
        assert!(matches!(
            command(&record, &NodeId::new("other")),
            Err(Error::Conflict)
        ));
        let mut foreign = record.clone();
        let mut outside = frozen();
        outside.repository_url = "file:///tmp/repo".into();
        foreign.input = Some(proto::ExecutionInput {
            spec: Some(proto::execution_input::Spec::Clone(outside)),
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

    /// Only a ready checkout at the dispatched base commit may be taken over; a failure carries
    /// no commit to compare.
    #[test]
    fn ready_results_must_sit_at_the_dispatched_base_commit() {
        let command = CloneRepositoryMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            request_id: None,
            operation_id: OperationId::new("operation"),
            execution_id: ExecutionId::new("execution"),
            payload: CloneRepository {
                spec: clone_execution_spec(),
            },
        };
        let failed = CloneExecutionResult::CloneFailed(CloneFailed {
            node: node(),
            spec: clone_execution_spec(),
            failure: CloneFailureCode::BaseCommitMismatch,
            residual: CloneResidual::NoDirectory {},
        });
        assert_eq!(
            [
                matches_dispatch(&command, &ready(BASE)),
                matches_dispatch(&command, &ready("1123456789abcdef0123456789abcdef01234567")),
                matches_dispatch(&command, &failed),
            ],
            [true, false, true]
        );
    }

    /// Both terminal facts project onto the contract and read back as the same outcome the local
    /// adapter would report for the wire result.
    #[test]
    fn results_project_onto_the_contract_outcome() {
        let ready = ready(BASE);
        assert_eq!(
            outcome(result(&ready)).unwrap(),
            ExecutionOutcome::from(&ready)
        );
        let failed = CloneExecutionResult::CloneFailed(CloneFailed {
            node: node(),
            spec: clone_execution_spec(),
            failure: CloneFailureCode::SnapshotUnavailable,
            residual: CloneResidual::Retained {
                repository_id: RepositoryId::new("repository"),
                path: NodePath::new("/node/partial"),
            },
        });
        assert_eq!(
            outcome(result(&failed)).unwrap(),
            ExecutionOutcome::from(&failed)
        );
        // Every current failure category survives the contract round trip, so no reason Cloud
        // stores decays into another category or a conflict when read back.
        for code in [
            CloneFailureCode::SourceUnavailable,
            CloneFailureCode::DestinationConflict,
            CloneFailureCode::OperationFailed,
            CloneFailureCode::Interrupted,
            CloneFailureCode::SnapshotUnavailable,
            CloneFailureCode::BaseCommitMismatch,
        ] {
            assert_eq!(failure(reason(code) as i32).unwrap(), code);
        }
        // The retired category has no contract value; Cloud refuses the unspecified reason.
        assert_eq!(
            reason(CloneFailureCode::BranchNotFound),
            proto::CloneFailureReason::Unspecified
        );
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
