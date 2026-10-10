use super::session::node;
use super::support::*;
use ora_node_protocol::*;
use serde_json::{Value, json};

const OPERATION: &str = "run-1";
const EXECUTION: &str = "execution-delivery";
const BASE: &str = "0123456789abcdef0123456789abcdef01234567";
const FINAL: &str = "89abcdef0123456789abcdef0123456789abcdef";
const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const BUNDLE_KEY: &str = "revisions/tenant-1/run-1/execution-delivery/revision.bundle";
const HISTORY_KEY: &str = "revisions/tenant-1/run-1/execution-delivery/session.jsonl";

/// A delivery command with independent JSON.
pub(super) fn deliver() -> Case {
    Case {
        message: Message::Controller(ControllerToNodeMessage::DeliverRevision(
            DeliverRevisionMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: OperationId::new(OPERATION),
                execution_id: ExecutionId::new(EXECUTION),
                payload: DeliverRevision {
                    spec: DeliverRevisionSpec {
                        node_id: NodeId::new("node-1"),
                        session_execution_id: ExecutionId::new("execution-session"),
                        checkout_execution_id: ExecutionId::new("execution-clone"),
                        base_commit: CommitId::new(BASE),
                        revision_ref: RevisionRef::new("refs/ora/revisions/run-1"),
                        bundle_key: ObjectKey::new(BUNDLE_KEY),
                        history_key: ObjectKey::new(HISTORY_KEY),
                        prior_revision: None,
                    },
                },
            },
        )),
        wire: json!({"message_type": "deliver_revision", "protocol_version": 1,
        "operation_id": OPERATION, "execution_id": EXECUTION,
        "payload": {"spec": {
            "node_id": "node-1",
            "session_execution_id": "execution-session",
            "checkout_execution_id": "execution-clone",
            "base_commit": BASE,
            "revision_ref": "refs/ora/revisions/run-1",
            "bundle_key": BUNDLE_KEY,
            "history_key": HISTORY_KEY
        }}}),
    }
}

/// Builds the object description the Node reports for one key.
fn stored(key: &str, size: u64) -> StoredObject {
    StoredObject {
        key: ObjectKey::new(key),
        size,
        sha256: Sha256Digest::new(DIGEST),
    }
}

/// Delivered, unchanged and failed results with independent JSON.
fn results() -> Vec<(RevisionExecutionResult, Value)> {
    let node_wire = json!({"node_id": "node-1", "incarnation_id": "incarnation-1"});
    let delivered = RevisionExecutionResult::RevisionDelivered(RevisionDelivered {
        node: node(),
        final_commit: CommitId::new(FINAL),
        base_commit: CommitId::new(BASE),
        revision_ref: RevisionRef::new("refs/ora/revisions/run-1"),
        bundle: stored(BUNDLE_KEY, /*size*/ 2048),
        history: stored(HISTORY_KEY, /*size*/ 512),
    });
    let delivered_wire = json!({"kind": "revision_delivered", "result": {
        "node": node_wire, "final_commit": FINAL, "base_commit": BASE,
        "revision_ref": "refs/ora/revisions/run-1",
        "bundle": {"key": BUNDLE_KEY, "size": 2048, "sha256": DIGEST},
        "history": {"key": HISTORY_KEY, "size": 512, "sha256": DIGEST}}});
    let unchanged = RevisionExecutionResult::RevisionUnchanged(RevisionUnchanged {
        node: node(),
        final_commit: CommitId::new(BASE),
        base_commit: CommitId::new(BASE),
        revision_ref: RevisionRef::new("refs/ora/revisions/run-1"),
        history: stored(HISTORY_KEY, /*size*/ 512),
    });
    let unchanged_wire = json!({"kind": "revision_unchanged", "result": {
        "node": node_wire, "final_commit": BASE, "base_commit": BASE,
        "revision_ref": "refs/ora/revisions/run-1",
        "history": {"key": HISTORY_KEY, "size": 512, "sha256": DIGEST}}});
    let failed = RevisionExecutionResult::RevisionFailed(RevisionFailed {
        node: node(),
        failure: RevisionFailureCode::UploadFailed,
    });
    let failed_wire = json!({"kind": "revision_failed", "result": {
        "node": node_wire, "failure": "upload_failed"}});
    vec![
        (delivered, delivered_wire),
        (unchanged, unchanged_wire),
        (failed, failed_wire),
    ]
}

/// Delivers one result as the retained event and as a Completed status answer.
fn result_cases(result: RevisionExecutionResult, wire: Value) -> [Case; 2] {
    [
        Case {
            message: Message::Node(NodeToControllerMessage::RevisionResult(
                RevisionResultMessage {
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    operation_id: OperationId::new(OPERATION),
                    execution_id: ExecutionId::new(EXECUTION),
                    sequence: Sequence::new(/*value*/ 1),
                    payload: result.clone(),
                },
            )),
            wire: json!({"message_type": "revision_result", "protocol_version": 1,
                "operation_id": OPERATION, "execution_id": EXECUTION, "sequence": 1,
                "payload": wire}),
        },
        Case {
            message: Message::Node(NodeToControllerMessage::ExecutionStatus(
                ExecutionStatusMessage {
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    operation_id: OperationId::new(OPERATION),
                    execution_id: ExecutionId::new(EXECUTION),
                    payload: ExecutionStatus {
                        node: node(),
                        state: ExecutionState::Completed(ExecutionResult::Revision(Box::new(
                            result,
                        ))),
                    },
                },
            )),
            wire: json!({"message_type": "execution_status", "protocol_version": 1,
                "operation_id": OPERATION, "execution_id": EXECUTION,
                "payload": {"node": {"node_id": "node-1", "incarnation_id": "incarnation-1"},
                    "state": {"state": "completed", "result": wire}}}),
        },
    ]
}

/// The delivery command has a fixed wire shape and the shared envelope guarantees.
#[tokio::test]
async fn deliver_revision_round_trips() -> Result<(), TestError> {
    let case = deliver();
    case.assert_wire().await?;
    case.assert_round_trip().await?;
    case.assert_envelope_rejections().await?;
    case.assert_fields(
        &[
            "/payload/spec/session_execution_id",
            "/payload/spec/base_commit",
            "/payload/spec/revision_ref",
            "/payload/spec/bundle_key",
            "/payload/spec/history_key",
        ],
        &[
            ("/operation_id", "operation_id"),
            ("/execution_id", "execution_id"),
            ("/payload/spec/node_id", "node_id"),
            ("/payload/spec/session_execution_id", "session_execution_id"),
            (
                "/payload/spec/checkout_execution_id",
                "checkout_execution_id",
            ),
        ],
    )
    .await
}

/// A delivery can never move a branch, write outside its keys, or name a partial commit.
#[tokio::test]
async fn deliver_revision_rejects_refs_keys_and_commits_outside_the_contract()
-> Result<(), TestError> {
    let cases = [
        (
            "/payload/spec/revision_ref",
            json!("refs/heads/main"),
            MessageValidationError::InvalidRevisionRef,
        ),
        (
            "/payload/spec/revision_ref",
            json!("refs/ora/revisions/../heads/main"),
            MessageValidationError::InvalidRevisionRef,
        ),
        (
            "/payload/spec/bundle_key",
            json!("/revisions/run-1/revision.bundle"),
            MessageValidationError::InvalidObjectKey,
        ),
        (
            "/payload/spec/bundle_key",
            json!("revisions/../other-tenant/revision.bundle"),
            MessageValidationError::InvalidObjectKey,
        ),
        (
            "/payload/spec/bundle_key",
            json!(HISTORY_KEY),
            MessageValidationError::InvalidObjectKey,
        ),
        (
            "/payload/spec/base_commit",
            json!("0123456"),
            MessageValidationError::InvalidCommit,
        ),
    ];
    for (path, value, expected) in cases {
        let mut wire = deliver().wire;
        replace(&mut wire, path, value);
        reject_semantics(Peer::Controller, &wire, path, expected).await?;
    }
    Ok(())
}

/// Revision results decode as the delivery family, and "changed" must agree with the commits.
#[tokio::test]
async fn revision_results_round_trip_and_keep_commits_consistent() -> Result<(), TestError> {
    for (result, wire) in results() {
        let [event, status] = result_cases(result, wire);
        for case in [&event, &status] {
            case.assert_wire().await?;
            case.assert_round_trip().await?;
            case.assert_envelope_rejections().await?;
        }
        status.assert_historical_node().await?;
    }
    let mut rejected = results();
    let (unchanged, unchanged_wire) = rejected.remove(1);
    let (delivered, delivered_wire) = rejected.remove(0);
    // An unchanged result may name the final commit of the Revision its session resumed; only
    // the Controller holds the delivery input that says whether it does.
    let RevisionExecutionResult::RevisionUnchanged(mut resumed) = unchanged else {
        unreachable!("the second fixture result is unchanged");
    };
    resumed.final_commit = CommitId::new(FINAL);
    let mut resumed_wire = unchanged_wire;
    replace(&mut resumed_wire, "/result/final_commit", json!(FINAL));
    let [resumed_event, _] = result_cases(
        RevisionExecutionResult::RevisionUnchanged(resumed),
        resumed_wire,
    );
    resumed_event.assert_wire().await?;
    resumed_event.assert_round_trip().await?;
    let mut wire = resumed_event.wire;
    replace(&mut wire, "/payload/result/base_commit", json!("0123456"));
    reject_semantics(
        Peer::Node,
        &wire,
        "unchanged with a partial base",
        MessageValidationError::InvalidCommit,
    )
    .await?;
    let [delivered_event, _] = result_cases(delivered, delivered_wire);
    let mut wire = delivered_event.wire;
    replace(&mut wire, "/payload/result/final_commit", json!(BASE));
    reject_semantics(
        Peer::Node,
        &wire,
        "delivered without a new commit",
        MessageValidationError::RevisionCommitMismatch,
    )
    .await
}

/// Upload grants are unsequenced messages, and their URL never appears in debug output.
#[tokio::test]
async fn upload_grants_round_trip_without_exposing_the_url() -> Result<(), TestError> {
    let url = "https://objects.example.com/bucket/revision.bundle?X-Amz-Signature=secret";
    let grant = Case {
        message: Message::Controller(ControllerToNodeMessage::UploadGrant(UploadGrantMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: OperationId::new(OPERATION),
            execution_id: ExecutionId::new(EXECUTION),
            payload: UploadGrant {
                grants: vec![ObjectUploadGrant {
                    object_key: ObjectKey::new(BUNDLE_KEY),
                    url: PresignedUrl::new(url),
                    method: UploadMethod::Put,
                    headers: [(
                        "x-amz-sdk-checksum-algorithm".to_owned(),
                        "SHA256".to_owned(),
                    )]
                    .into(),
                    expires_at: time::macros::datetime!(2026-09-28 12:15:00 UTC),
                }],
            },
        })),
        wire: json!({"message_type": "upload_grant", "protocol_version": 1,
            "operation_id": OPERATION, "execution_id": EXECUTION,
            "payload": {"grants": [{"object_key": BUNDLE_KEY, "url": url, "method": "PUT",
                "headers": {"x-amz-sdk-checksum-algorithm": "SHA256"},
                "expires_at": "2026-09-28T12:15:00Z"}]}}),
    };
    grant.assert_wire().await?;
    grant.assert_round_trip().await?;
    grant.assert_envelope_rejections().await?;
    let Message::Controller(message) = &grant.message else {
        panic!("expected Controller message")
    };
    assert!(!format!("{message:?}").contains("secret"));

    let mut wire = grant.wire.clone();
    replace(&mut wire, "/payload/grants", json!([]));
    reject_semantics(
        Peer::Controller,
        &wire,
        "no grants",
        MessageValidationError::EmptyUploadGrant,
    )
    .await?;
    replace(
        &mut wire,
        "/payload/grants",
        grant.wire["payload"]["grants"].clone(),
    );
    replace(&mut wire, "/payload/grants/0/url", json!("revision.bundle"));
    reject_semantics(
        Peer::Controller,
        &wire,
        "relative url",
        MessageValidationError::InvalidUploadGrant,
    )
    .await?;

    let needed = Case {
        message: Message::Node(NodeToControllerMessage::UploadGrantNeeded(
            UploadGrantNeededMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: OperationId::new(OPERATION),
                execution_id: ExecutionId::new(EXECUTION),
                payload: UploadGrantNeeded {
                    node_id: NodeId::new("node-1"),
                    checksums: [(ObjectKey::new(BUNDLE_KEY), Sha256Digest::new(DIGEST))].into(),
                },
            },
        )),
        wire: json!({"message_type": "upload_grant_needed", "protocol_version": 1,
            "operation_id": OPERATION, "execution_id": EXECUTION,
            "payload": {"node_id": "node-1", "checksums": {BUNDLE_KEY: DIGEST}}}),
    };
    needed.assert_wire().await?;
    needed.assert_round_trip().await?;
    needed.assert_envelope_rejections().await?;
    needed
        .assert_fields(
            &["/payload/node_id", "/payload/checksums"],
            &[
                ("/operation_id", "operation_id"),
                ("/execution_id", "execution_id"),
                ("/payload/node_id", "node_id"),
            ],
        )
        .await?;
    // A delivery asks for grants only while it still has an object to upload, and every digest is
    // the canonical one it froze, so Cloud can bind the grant to exactly those bytes.
    let mut wire = needed.wire.clone();
    replace(&mut wire, "/payload/checksums", json!({}));
    reject_semantics(
        Peer::Node,
        &wire,
        "no objects",
        MessageValidationError::EmptyUploadGrant,
    )
    .await?;
    replace(
        &mut wire,
        "/payload/checksums",
        json!({BUNDLE_KEY: DIGEST.to_uppercase()}),
    );
    reject_semantics(
        Peer::Node,
        &wire,
        "uppercase digest",
        MessageValidationError::InvalidSha256,
    )
    .await?;
    replace(
        &mut wire,
        "/payload/checksums",
        json!({"/absolute": DIGEST}),
    );
    reject_semantics(
        Peer::Node,
        &wire,
        "absolute key",
        MessageValidationError::InvalidObjectKey,
    )
    .await
}

/// A controlled delivery carries a runtime permit for exactly this execution, operation and Node.
#[tokio::test]
async fn controlled_delivery_requires_a_matching_runtime_permit() -> Result<(), TestError> {
    let Message::Controller(ControllerToNodeMessage::DeliverRevision(command)) = deliver().message
    else {
        unreachable!("the delivery case is a controller DeliverRevision message");
    };
    let binding = RuntimeBinding {
        tenant_id: "tenant".into(),
        workspace_id: "workspace".into(),
        sandbox_id: "sandbox".into(),
        runtime_generation: 1,
        node_id: command.payload.spec.node_id.as_str().into(),
        node_incarnation_id: "incarnation".into(),
        node_instance_id: "instance".into(),
        controller_epoch: 1,
        control_epoch: 1,
        control_version: 1,
        session_id: "session".into(),
        actor_user_id: "actor".into(),
        operation_id: String::new(),
        execution_id: command.execution_id.as_str().into(),
        node_operation_id: command.operation_id.as_str().into(),
        input_closed: false,
        issued_at_ms: 1000,
        expires_at_ms: 31_000,
    };
    let envelope = ControlledDeliverRevision { binding, command };
    let message = ControllerToNodeMessage::ControlledDeliverRevision(Box::new(envelope.clone()));
    assert!(message.validate().is_ok());
    pretty_assertions::assert_eq!(round_trip_controller(message.clone()).await?, message);
    let mut changed = envelope.clone();
    changed.binding.node_operation_id = "wrong".into();
    assert!(changed.validate().is_err());
    changed = envelope.clone();
    changed.binding.execution_id = "wrong".into();
    assert!(changed.validate().is_err());
    changed = envelope.clone();
    changed.binding.node_id = "wrong".into();
    assert!(changed.validate().is_err());
    changed = envelope;
    changed.binding.input_closed = true;
    assert!(changed.validate().is_err());
    Ok(())
}
