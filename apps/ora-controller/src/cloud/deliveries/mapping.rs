//! Lossless translation between Cloud's Revision delivery contract and the Node protocol. Upload
//! grants pass through here too; nothing in this module formats them.
use crate::*;
use ora_controller_proto::v1 as proto;
use std::collections::{BTreeMap, HashMap};

/// Identifies only delivery work; sessions and plugins keep their own families.
pub(in crate::cloud) fn is_delivery(record: &proto::ExecutionRecord) -> bool {
    matches!(
        record.input.as_ref().and_then(|v| v.spec.as_ref()),
        Some(proto::execution_input::Spec::DeliverRevision(_))
    )
}

/// Reconstructs a recorded delivery. Cloud's spec carries no Node because the record names it
/// beside the input; the Node protocol repeats it inside the spec, so it is added here.
pub(in crate::cloud) fn deliver(
    record: &proto::ExecutionRecord,
    node: &NodeId,
) -> Result<DeliverRevisionMessage, Error> {
    if record.node_id != node.as_str() {
        return Err(Error::Conflict);
    }
    let Some(proto::execution_input::Spec::DeliverRevision(spec)) =
        record.input.as_ref().and_then(|v| v.spec.as_ref())
    else {
        return Err(Error::Conflict);
    };
    let command = DeliverRevisionMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: OperationId::new(record.node_operation_id.clone()),
        execution_id: ExecutionId::new(record.execution_id.clone()),
        payload: DeliverRevision {
            spec: DeliverRevisionSpec {
                node_id: node.clone(),
                session_execution_id: ExecutionId::new(spec.session_execution_id.clone()),
                checkout_execution_id: ExecutionId::new(spec.checkout_execution_id.clone()),
                base_commit: CommitId::new(spec.base_commit.clone()),
                revision_ref: RevisionRef::new(spec.revision_ref.clone()),
                bundle_key: ObjectKey::new(spec.bundle_key.clone()),
                history_key: ObjectKey::new(spec.history_key.clone()),
            },
        },
    };
    // An input the Node protocol refuses is a disagreement with the authority, not a retry.
    command.validate()?;
    Ok(command)
}

/// The Node incarnation that produced a delivery result.
pub(in crate::cloud) fn producer(result: &RevisionExecutionResult) -> &NodeRuntimeIdentity {
    match result {
        RevisionExecutionResult::RevisionDelivered(v) => &v.node,
        RevisionExecutionResult::RevisionUnchanged(v) => &v.node,
        RevisionExecutionResult::RevisionFailed(v) => &v.node,
    }
}

/// A successful result must describe the dispatched base, ref and object keys; a failure carries
/// nothing to compare.
pub(super) fn accepts(command: &DeliverRevisionMessage, result: &RevisionExecutionResult) -> bool {
    let spec = &command.payload.spec;
    match result {
        RevisionExecutionResult::RevisionDelivered(v) => {
            v.base_commit == spec.base_commit
                && v.revision_ref == spec.revision_ref
                && v.bundle.key == spec.bundle_key
                && v.history.key == spec.history_key
        }
        RevisionExecutionResult::RevisionUnchanged(v) => {
            v.base_commit == spec.base_commit
                && v.revision_ref == spec.revision_ref
                && v.history.key == spec.history_key
        }
        RevisionExecutionResult::RevisionFailed(_) => true,
    }
}

fn stored(object: &StoredObject) -> proto::StoredObject {
    proto::StoredObject {
        key: object.key.as_str().into(),
        size: object.size,
        sha256: object.sha256.as_str().into(),
    }
}

/// Projects the Node's terminal fact onto the contract, preserving the producing incarnation.
/// `VERIFICATION_FAILED` has no Node counterpart: only Cloud records it after checking objects.
pub(super) fn result(value: &RevisionExecutionResult) -> proto::ExecutionResult {
    let node = producer(value);
    let outcome = match value {
        RevisionExecutionResult::RevisionDelivered(v) => {
            proto::execution_result::Outcome::RevisionDelivered(proto::RevisionDelivered {
                final_commit: v.final_commit.as_str().into(),
                base_commit: v.base_commit.as_str().into(),
                revision_ref: v.revision_ref.as_str().into(),
                bundle: Some(stored(&v.bundle)),
                history: Some(stored(&v.history)),
            })
        }
        RevisionExecutionResult::RevisionUnchanged(v) => {
            proto::execution_result::Outcome::RevisionUnchanged(proto::RevisionUnchanged {
                final_commit: v.final_commit.as_str().into(),
                base_commit: v.base_commit.as_str().into(),
                revision_ref: v.revision_ref.as_str().into(),
                history: Some(stored(&v.history)),
            })
        }
        RevisionExecutionResult::RevisionFailed(v) => {
            proto::execution_result::Outcome::RevisionFailed(proto::RevisionFailed {
                reason: match v.failure {
                    RevisionFailureCode::SessionNotSettled => {
                        proto::RevisionFailureReason::SessionNotSettled
                    }
                    RevisionFailureCode::CheckoutUnavailable => {
                        proto::RevisionFailureReason::CheckoutUnavailable
                    }
                    RevisionFailureCode::SnapshotFailed => {
                        proto::RevisionFailureReason::SnapshotFailed
                    }
                    RevisionFailureCode::BundleFailed => proto::RevisionFailureReason::BundleFailed,
                    RevisionFailureCode::HistoryUnavailable => {
                        proto::RevisionFailureReason::HistoryUnavailable
                    }
                    RevisionFailureCode::UploadFailed => proto::RevisionFailureReason::UploadFailed,
                } as i32,
            })
        }
    };
    proto::ExecutionResult {
        node: Some(proto::NodeIdentity {
            node_id: node.node_id.as_str().into(),
            node_incarnation_id: node.incarnation_id.as_str().into(),
        }),
        outcome: Some(outcome),
    }
}

/// The request map Cloud signs into each grant's checksum header.
pub(super) fn checksums(value: &BTreeMap<ObjectKey, Sha256Digest>) -> HashMap<String, String> {
    value
        .iter()
        .map(|(key, digest)| (key.as_str().into(), digest.as_str().into()))
        .collect()
}

/// Hands Cloud's grants to the Node unchanged: headers verbatim (the signed `If-None-Match: *` and
/// checksum header included) and the same expiry. Only grants for requested keys are accepted, so
/// Cloud can never redirect an upload to another object.
pub(super) fn grants(
    response: proto::GrantRevisionUploadResponse,
    command: &DeliverRevisionMessage,
    requested: &BTreeMap<ObjectKey, Sha256Digest>,
) -> Result<UploadGrantMessage, Error> {
    let grants = response
        .grants
        .into_iter()
        .map(|grant| {
            let object_key = ObjectKey::new(grant.object_key);
            if !requested.contains_key(&object_key) || grant.method != "PUT" {
                return Err(Error::Conflict);
            }
            let expires = grant.expires_at.ok_or(Error::Conflict)?;
            let nanos = i128::from(expires.seconds) * 1_000_000_000 + i128::from(expires.nanos);
            Ok(ObjectUploadGrant {
                object_key,
                url: PresignedUrl::new(grant.url),
                method: UploadMethod::Put,
                headers: grant.headers.into_iter().collect(),
                expires_at: time::OffsetDateTime::from_unix_timestamp_nanos(nanos)
                    .map_err(|_| Error::Conflict)?,
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let message = UploadGrantMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: command.operation_id.clone(),
        execution_id: command.execution_id.clone(),
        payload: UploadGrant { grants },
    };
    message.validate()?;
    Ok(message)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    const BASE: &str = "0123456789abcdef0123456789abcdef01234567";
    const FINAL: &str = "89abcdef0123456789abcdef0123456789abcdef";
    const DIGEST: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    fn record() -> proto::ExecutionRecord {
        proto::ExecutionRecord {
            operation_id: "run".into(),
            node_operation_id: "node-operation".into(),
            execution_id: "execution".into(),
            node_id: "node".into(),
            input: Some(proto::ExecutionInput {
                spec: Some(proto::execution_input::Spec::DeliverRevision(
                    proto::DeliverRevisionSpec {
                        session_execution_id: "session".into(),
                        checkout_execution_id: "checkout".into(),
                        base_commit: BASE.into(),
                        revision_ref: "refs/ora/revisions/run".into(),
                        bundle_key: "runs/run/revision.bundle".into(),
                        history_key: "runs/run/history.jsonl".into(),
                    },
                )),
            }),
            result: None,
        }
    }

    fn identity() -> NodeRuntimeIdentity {
        NodeRuntimeIdentity {
            node_id: NodeId::new("node"),
            incarnation_id: NodeIncarnationId::new("incarnation"),
        }
    }

    fn object(key: &str) -> StoredObject {
        StoredObject {
            key: ObjectKey::new(key),
            size: 7,
            sha256: Sha256Digest::new(DIGEST),
        }
    }

    /// The recorded input becomes the exact Node command, with the target Node added and the
    /// Node-local operation identity read back from Cloud.
    #[test]
    fn records_rebuild_the_delivery_for_the_target_node() {
        let node = NodeId::new("node");
        assert_eq!(
            deliver(&record(), &node).unwrap(),
            DeliverRevisionMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: OperationId::new("node-operation"),
                execution_id: ExecutionId::new("execution"),
                payload: DeliverRevision {
                    spec: DeliverRevisionSpec {
                        node_id: node.clone(),
                        session_execution_id: ExecutionId::new("session"),
                        checkout_execution_id: ExecutionId::new("checkout"),
                        base_commit: CommitId::new(BASE),
                        revision_ref: RevisionRef::new("refs/ora/revisions/run"),
                        bundle_key: ObjectKey::new("runs/run/revision.bundle"),
                        history_key: ObjectKey::new("runs/run/history.jsonl"),
                    },
                },
            }
        );
        assert!(is_delivery(&record()));
        assert!(matches!(
            deliver(&record(), &NodeId::new("other")),
            Err(Error::Conflict)
        ));
        let mut outside = record();
        if let Some(proto::execution_input::Spec::DeliverRevision(spec)) =
            outside.input.as_mut().and_then(|i| i.spec.as_mut())
        {
            spec.revision_ref = "refs/heads/main".into();
        }
        assert!(matches!(
            deliver(&outside, &node),
            Err(Error::Validation(_))
        ));
    }

    /// Every Node outcome keeps its producer and evidence; the Node can never claim the
    /// Cloud-only verification failure.
    #[test]
    fn results_project_onto_the_contract() {
        let delivered = RevisionExecutionResult::RevisionDelivered(RevisionDelivered {
            node: identity(),
            final_commit: CommitId::new(FINAL),
            base_commit: CommitId::new(BASE),
            revision_ref: RevisionRef::new("refs/ora/revisions/run"),
            bundle: object("runs/run/revision.bundle"),
            history: object("runs/run/history.jsonl"),
        });
        assert_eq!(
            result(&delivered),
            proto::ExecutionResult {
                node: Some(proto::NodeIdentity {
                    node_id: "node".into(),
                    node_incarnation_id: "incarnation".into(),
                }),
                outcome: Some(proto::execution_result::Outcome::RevisionDelivered(
                    proto::RevisionDelivered {
                        final_commit: FINAL.into(),
                        base_commit: BASE.into(),
                        revision_ref: "refs/ora/revisions/run".into(),
                        bundle: Some(proto::StoredObject {
                            key: "runs/run/revision.bundle".into(),
                            size: 7,
                            sha256: DIGEST.into(),
                        }),
                        history: Some(proto::StoredObject {
                            key: "runs/run/history.jsonl".into(),
                            size: 7,
                            sha256: DIGEST.into(),
                        }),
                    }
                )),
            }
        );
        let command = deliver(&record(), &NodeId::new("node")).unwrap();
        assert!(accepts(&command, &delivered));
        let RevisionExecutionResult::RevisionDelivered(mut foreign) = delivered else {
            unreachable!()
        };
        foreign.bundle = object("runs/other/revision.bundle");
        assert!(!accepts(
            &command,
            &RevisionExecutionResult::RevisionDelivered(foreign)
        ));
        for (code, reason) in [
            (
                RevisionFailureCode::SessionNotSettled,
                proto::RevisionFailureReason::SessionNotSettled,
            ),
            (
                RevisionFailureCode::CheckoutUnavailable,
                proto::RevisionFailureReason::CheckoutUnavailable,
            ),
            (
                RevisionFailureCode::SnapshotFailed,
                proto::RevisionFailureReason::SnapshotFailed,
            ),
            (
                RevisionFailureCode::BundleFailed,
                proto::RevisionFailureReason::BundleFailed,
            ),
            (
                RevisionFailureCode::HistoryUnavailable,
                proto::RevisionFailureReason::HistoryUnavailable,
            ),
            (
                RevisionFailureCode::UploadFailed,
                proto::RevisionFailureReason::UploadFailed,
            ),
        ] {
            let failed = RevisionExecutionResult::RevisionFailed(RevisionFailed {
                node: identity(),
                failure: code,
            });
            assert_eq!(
                result(&failed).outcome,
                Some(proto::execution_result::Outcome::RevisionFailed(
                    proto::RevisionFailed {
                        reason: reason as i32
                    }
                ))
            );
            assert_ne!(reason, proto::RevisionFailureReason::VerificationFailed);
        }
    }

    /// Grants reach the Node with headers and expiry verbatim, and only for requested keys.
    #[test]
    fn grants_pass_through_verbatim_for_requested_keys() {
        let command = deliver(&record(), &NodeId::new("node")).unwrap();
        let requested = BTreeMap::from([(
            ObjectKey::new("runs/run/history.jsonl"),
            Sha256Digest::new(DIGEST),
        )]);
        let headers = HashMap::from([
            ("If-None-Match".to_owned(), "*".to_owned()),
            ("x-amz-checksum-sha256".to_owned(), "signed".to_owned()),
        ]);
        let grant = |key: &str| proto::UploadGrant {
            object_key: key.into(),
            url: "https://store.example/upload?X-Amz-Signature=s".into(),
            method: "PUT".into(),
            headers: headers.clone(),
            expires_at: Some(prost_types::Timestamp {
                seconds: 1_800_000_000,
                nanos: 5,
            }),
        };
        let message = grants(
            proto::GrantRevisionUploadResponse {
                grants: vec![grant("runs/run/history.jsonl")],
            },
            &command,
            &requested,
        )
        .unwrap();
        assert_eq!(
            message,
            UploadGrantMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: OperationId::new("node-operation"),
                execution_id: ExecutionId::new("execution"),
                payload: UploadGrant {
                    grants: vec![ObjectUploadGrant {
                        object_key: ObjectKey::new("runs/run/history.jsonl"),
                        url: PresignedUrl::new("https://store.example/upload?X-Amz-Signature=s"),
                        method: UploadMethod::Put,
                        headers: headers.clone().into_iter().collect(),
                        expires_at: time::OffsetDateTime::from_unix_timestamp_nanos(
                            1_800_000_000_000_000_005
                        )
                        .unwrap(),
                    }],
                },
            }
        );
        assert!(matches!(
            grants(
                proto::GrantRevisionUploadResponse {
                    grants: vec![grant("runs/run/revision.bundle")],
                },
                &command,
                &requested,
            ),
            Err(Error::Conflict)
        ));
        assert!(matches!(
            grants(
                proto::GrantRevisionUploadResponse { grants: vec![] },
                &command,
                &requested,
            ),
            Err(Error::Validation(_))
        ));
    }
}
