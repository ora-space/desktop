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
                history_key: ObjectKey::new(spec.history_key.clone()),
                prior_revision: spec.prior_revision.as_ref().map(prior_commit).transpose()?,
                // A repository or ref the Node protocol refuses is a disagreement with the
                // authority, like any other invalid input.
                git_repository_url: GitRepositoryUrl::parse(&spec.git_repository_url)
                    .map_err(|_| Error::Conflict)?,
                delivery_ref: DeliveryRef::parse(&spec.delivery_ref)
                    .map_err(|_| Error::Conflict)?,
                workspace_ref: WorkspaceRef::parse(&spec.workspace_ref)
                    .map_err(|_| Error::Conflict)?,
            },
        },
    };
    // An input the Node protocol refuses is a disagreement with the authority, not a retry.
    command.validate()?;
    Ok(command)
}

/// A delivery only compares against the prior final commit. The fetch location and base belong to
/// the session input alone; a delivery input naming them disagrees with the contract, so it is
/// refused rather than silently narrowed.
fn prior_commit(prior: &proto::PriorRevision) -> Result<PriorRevisionCommit, Error> {
    if !prior.git_repository_url.is_empty()
        || !prior.delivery_ref.is_empty()
        || !prior.base_commit.is_empty()
    {
        return Err(Error::Conflict);
    }
    Ok(PriorRevisionCommit {
        revision_id: RevisionId::new(prior.revision_id.clone()),
        final_commit: CommitId::new(prior.final_commit.clone()),
    })
}

/// The Node incarnation that produced a delivery result.
pub(in crate::cloud) fn producer(result: &RevisionExecutionResult) -> &NodeRuntimeIdentity {
    match result {
        RevisionExecutionResult::RevisionDelivered(v) => &v.node,
        RevisionExecutionResult::RevisionUnchanged(v) => &v.node,
        RevisionExecutionResult::RevisionFailed(v) => &v.node,
    }
}

/// A successful result must describe the dispatched base, revision ref and history key; a failure
/// carries nothing to compare. An unchanged result stops at the base or, for a resumed session, at
/// the prior Revision's final commit, and a delivered one at neither: the Node codec cannot tell,
/// because only the input names the prior commit. Whether the delivery ref holds the final commit
/// is Cloud's to verify through a ref query; this check never reaches the Git Server.
pub(super) fn accepts(command: &DeliverRevisionMessage, result: &RevisionExecutionResult) -> bool {
    let spec = &command.payload.spec;
    let unchanged = |final_commit: &CommitId| {
        *final_commit == spec.base_commit
            || spec
                .prior_revision
                .as_ref()
                .is_some_and(|prior| prior.final_commit == *final_commit)
    };
    match result {
        RevisionExecutionResult::RevisionDelivered(v) => {
            v.base_commit == spec.base_commit
                && v.revision_ref == spec.revision_ref
                && v.history.key == spec.history_key
                && !unchanged(&v.final_commit)
        }
        RevisionExecutionResult::RevisionUnchanged(v) => {
            v.base_commit == spec.base_commit
                && v.revision_ref == spec.revision_ref
                && v.history.key == spec.history_key
                && unchanged(&v.final_commit)
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
                    // Retired: the Node codec rejects a new result carrying it, so it never
                    // reaches here, and mapping it to itself keeps history readable.
                    RevisionFailureCode::BundleFailed => proto::RevisionFailureReason::BundleFailed,
                    RevisionFailureCode::HistoryUnavailable => {
                        proto::RevisionFailureReason::HistoryUnavailable
                    }
                    RevisionFailureCode::UploadFailed => proto::RevisionFailureReason::UploadFailed,
                    RevisionFailureCode::PushRejected => proto::RevisionFailureReason::PushRejected,
                    RevisionFailureCode::PushFailed => proto::RevisionFailureReason::PushFailed,
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
            Ok(ObjectUploadGrant {
                object_key,
                url: PresignedUrl::new(grant.url),
                method: UploadMethod::Put,
                headers: grant.headers.into_iter().collect(),
                expires_at: super::super::mapping::expiry(grant.expires_at)?,
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
    const PRIOR: &str = "fedcba9876543210fedcba9876543210fedcba98";
    const OTHER: &str = "00112233445566778899aabbccddeeff00112233";
    const DIGEST: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    const REPOSITORY: &str = "https://git.ora.test/6f1c2a3b-4d5e-4f60-8a7b-9c0d1e2f3a4b/0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d.git";
    const DELIVERY_REF: &str = "refs/ora/deliveries/run";
    const WORKSPACE_REF: &str = "refs/heads/workspaces/4b3c2d1e-0f9a-4b8c-9d7e-6f5a4b3c2d1e/main";

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
                        history_key: "runs/run/history.jsonl".into(),
                        prior_revision: None,
                        git_repository_url: REPOSITORY.into(),
                        delivery_ref: DELIVERY_REF.into(),
                        workspace_ref: WORKSPACE_REF.into(),
                    },
                )),
            }),
            result: None,
        }
    }

    /// Edits the recorded delivery input in place.
    fn edit(
        record: &mut proto::ExecutionRecord,
        edit: impl FnOnce(&mut proto::DeliverRevisionSpec),
    ) {
        if let Some(proto::execution_input::Spec::DeliverRevision(spec)) =
            record.input.as_mut().and_then(|i| i.spec.as_mut())
        {
            edit(spec);
        }
    }

    /// The same delivery resuming the prior Revision whose final commit is `PRIOR`.
    fn resumed() -> proto::ExecutionRecord {
        let mut record = record();
        edit(&mut record, |spec| {
            spec.prior_revision = Some(proto::PriorRevision {
                revision_id: "revision-1".into(),
                final_commit: PRIOR.into(),
                git_repository_url: String::new(),
                delivery_ref: String::new(),
                base_commit: String::new(),
            });
        });
        record
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

    fn delivered(final_commit: &str) -> RevisionExecutionResult {
        RevisionExecutionResult::RevisionDelivered(RevisionDelivered {
            node: identity(),
            final_commit: CommitId::new(final_commit),
            base_commit: CommitId::new(BASE),
            revision_ref: RevisionRef::new("refs/ora/revisions/run"),
            history: object("runs/run/history.jsonl"),
        })
    }

    fn unchanged(final_commit: &str) -> RevisionExecutionResult {
        RevisionExecutionResult::RevisionUnchanged(RevisionUnchanged {
            node: identity(),
            final_commit: CommitId::new(final_commit),
            base_commit: CommitId::new(BASE),
            revision_ref: RevisionRef::new("refs/ora/revisions/run"),
            history: object("runs/run/history.jsonl"),
        })
    }

    /// The recorded input becomes the exact Node command, with the Git Server repository and refs
    /// forwarded verbatim, the target Node added and the Node-local operation identity read back.
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
                        history_key: ObjectKey::new("runs/run/history.jsonl"),
                        prior_revision: None,
                        git_repository_url: GitRepositoryUrl::parse(REPOSITORY).unwrap(),
                        delivery_ref: DeliveryRef::parse(DELIVERY_REF).unwrap(),
                        workspace_ref: WorkspaceRef::parse(WORKSPACE_REF).unwrap(),
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
        edit(&mut outside, |spec| {
            spec.revision_ref = "refs/heads/main".into()
        });
        assert!(matches!(
            deliver(&outside, &node),
            Err(Error::Validation(_))
        ));
        for change in [
            (|spec: &mut proto::DeliverRevisionSpec| spec.git_repository_url = String::new())
                as fn(&mut proto::DeliverRevisionSpec),
            |spec| spec.git_repository_url = "file:///srv/repo.git".into(),
            |spec| spec.delivery_ref = "refs/ora/revisions/run".into(),
            |spec| spec.workspace_ref = String::new(),
            |spec| spec.workspace_ref = "refs/heads/main".into(),
        ] {
            let mut invalid = record();
            edit(&mut invalid, change);
            assert!(matches!(deliver(&invalid, &node), Err(Error::Conflict)));
        }
    }

    /// Every Node outcome keeps its producer and evidence; the Node can never claim the
    /// Cloud-only verification failure.
    #[test]
    fn results_project_onto_the_contract() {
        assert_eq!(
            result(&delivered(FINAL)),
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
                        history: Some(proto::StoredObject {
                            key: "runs/run/history.jsonl".into(),
                            size: 7,
                            sha256: DIGEST.into(),
                        }),
                    }
                )),
            }
        );
        assert_eq!(
            result(&unchanged(BASE)).outcome,
            Some(proto::execution_result::Outcome::RevisionUnchanged(
                proto::RevisionUnchanged {
                    final_commit: BASE.into(),
                    base_commit: BASE.into(),
                    revision_ref: "refs/ora/revisions/run".into(),
                    history: Some(proto::StoredObject {
                        key: "runs/run/history.jsonl".into(),
                        size: 7,
                        sha256: DIGEST.into(),
                    }),
                }
            ))
        );
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
            (
                RevisionFailureCode::PushRejected,
                proto::RevisionFailureReason::PushRejected,
            ),
            (
                RevisionFailureCode::PushFailed,
                proto::RevisionFailureReason::PushFailed,
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

    /// The terminal check compares the dispatched base, revision ref and history key; an
    /// unchanged final commit is the base or the prior final commit, a delivered one neither.
    #[test]
    fn terminal_results_must_match_the_dispatched_delivery() {
        let node = NodeId::new("node");
        let fresh = deliver(&record(), &node).unwrap();
        let resumed = deliver(&resumed(), &node).unwrap();
        assert_eq!(
            resumed.payload.spec.prior_revision,
            Some(PriorRevisionCommit {
                revision_id: RevisionId::new("revision-1"),
                final_commit: CommitId::new(PRIOR),
            })
        );
        assert_eq!(
            [BASE, PRIOR, OTHER].map(|commit| accepts(&resumed, &unchanged(commit))),
            [true, true, false]
        );
        assert_eq!(
            [BASE, PRIOR].map(|commit| accepts(&fresh, &unchanged(commit))),
            [true, false]
        );
        assert_eq!(
            [FINAL, PRIOR].map(|commit| accepts(&resumed, &delivered(commit))),
            [true, false]
        );
        assert_eq!(
            [FINAL, PRIOR].map(|commit| accepts(&fresh, &delivered(commit))),
            [true, true]
        );
        let foreign = |change: fn(&mut RevisionDelivered)| {
            let RevisionExecutionResult::RevisionDelivered(mut value) = delivered(FINAL) else {
                unreachable!("the fixture is delivered")
            };
            change(&mut value);
            accepts(&fresh, &RevisionExecutionResult::RevisionDelivered(value))
        };
        assert_eq!(
            [
                foreign(|v| v.base_commit = CommitId::new(OTHER)),
                foreign(|v| v.revision_ref = RevisionRef::new("refs/ora/revisions/other")),
                foreign(|v| v.history = object("runs/other/history.jsonl")),
            ],
            [false, false, false]
        );
        assert!(accepts(
            &fresh,
            &RevisionExecutionResult::RevisionFailed(RevisionFailed {
                node: identity(),
                failure: RevisionFailureCode::PushRejected,
            })
        ));
    }

    /// The fetch location and base of a prior Revision belong to the session input only; a
    /// delivery input carrying any of them is refused.
    #[test]
    fn delivery_inputs_naming_a_prior_fetch_location_are_refused() {
        let node = NodeId::new("node");
        for change in [
            (|prior: &mut proto::PriorRevision| prior.git_repository_url = REPOSITORY.into())
                as fn(&mut proto::PriorRevision),
            |prior| prior.delivery_ref = "refs/ora/deliveries/prior".into(),
            |prior| prior.base_commit = BASE.into(),
        ] {
            let mut record = resumed();
            edit(&mut record, |spec| {
                if let Some(prior) = spec.prior_revision.as_mut() {
                    change(prior);
                }
            });
            assert!(matches!(deliver(&record, &node), Err(Error::Conflict)));
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
                    grants: vec![grant("runs/run/other.jsonl")],
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
