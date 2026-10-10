//! Resuming from a prior Revision: the prior Revision in session and delivery inputs, and the
//! memory-only download grant exchange.
use super::support::*;
use ora_node_protocol::*;
use serde_json::json;

const OPERATION: &str = "run-2";
const EXECUTION: &str = "execution-session";
const PRIOR_FINAL: &str = "89abcdef0123456789abcdef0123456789abcdef";
const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const PRIOR_BUNDLE: &str = "revisions/tenant-1/run-1/execution-delivery/revision.bundle";

/// The prior Revision a resumed session restores.
fn prior() -> PriorRevision {
    PriorRevision {
        revision_id: RevisionId::new("revision-1"),
        final_commit: CommitId::new(PRIOR_FINAL),
        bundle: StoredObject {
            key: ObjectKey::new(PRIOR_BUNDLE),
            size: 2048,
            sha256: Sha256Digest::new(DIGEST),
        },
    }
}

/// A session start that resumes `prior()`, with independent JSON.
fn resumed_start() -> Case {
    let mut case = super::agent_session::start();
    let Message::Controller(ControllerToNodeMessage::StartAgentSession(message)) =
        &mut case.message
    else {
        unreachable!("the start fixture is a StartAgentSession message");
    };
    message.payload.spec.prior_revision = Some(prior());
    case.wire["payload"]["spec"]["prior_revision"] = json!({
        "revision_id": "revision-1",
        "final_commit": PRIOR_FINAL,
        "bundle": {"key": PRIOR_BUNDLE, "size": 2048, "sha256": DIGEST}
    });
    case
}

/// A delivery of a resumed session, which carries the prior final commit but no bundle.
fn resumed_delivery() -> Case {
    let mut case = super::revision::deliver();
    let Message::Controller(ControllerToNodeMessage::DeliverRevision(message)) = &mut case.message
    else {
        unreachable!("the delivery fixture is a DeliverRevision message");
    };
    message.payload.spec.prior_revision = Some(prior().commit());
    case.wire["payload"]["spec"]["prior_revision"] = json!({
        "revision_id": "revision-1",
        "final_commit": PRIOR_FINAL
    });
    case
}

/// The prior Revision is optional on the wire, fixed in shape, and validated like the rest of the
/// input, so a Node can verify the bytes and commit it restores against exactly these values.
#[tokio::test]
async fn prior_revision_round_trips_in_session_and_delivery_inputs() -> Result<(), TestError> {
    for case in [resumed_start(), resumed_delivery()] {
        case.assert_wire().await?;
        case.assert_round_trip().await?;
        case.assert_envelope_rejections().await?;
    }
    resumed_start()
        .assert_fields(
            &[
                "/payload/spec/prior_revision/revision_id",
                "/payload/spec/prior_revision/final_commit",
                "/payload/spec/prior_revision/bundle",
            ],
            &[(
                "/payload/spec/prior_revision/revision_id",
                "prior_revision.revision_id",
            )],
        )
        .await?;
    let start_cases = [
        (
            "/payload/spec/prior_revision/final_commit",
            json!("89abcdef"),
            MessageValidationError::InvalidCommit,
        ),
        (
            "/payload/spec/prior_revision/bundle/key",
            json!("../revision.bundle"),
            MessageValidationError::InvalidObjectKey,
        ),
        (
            "/payload/spec/prior_revision/bundle/sha256",
            json!(DIGEST.to_uppercase()),
            MessageValidationError::InvalidSha256,
        ),
    ];
    for (path, value, expected) in start_cases {
        let mut wire = resumed_start().wire;
        replace(&mut wire, path, value);
        reject_semantics(Peer::Controller, &wire, path, expected).await?;
    }
    let mut wire = resumed_delivery().wire;
    replace(
        &mut wire,
        "/payload/spec/prior_revision/final_commit",
        json!("89abcdef"),
    );
    reject_semantics(
        Peer::Controller,
        &wire,
        "partial prior commit",
        MessageValidationError::InvalidCommit,
    )
    .await?;
    // A delivery never downloads the prior bundle, so its input cannot name one.
    let mut wire = resumed_delivery().wire;
    wire["payload"]["spec"]["prior_revision"]["bundle"] =
        json!({"key": PRIOR_BUNDLE, "size": 2048, "sha256": DIGEST});
    reject_structure(Peer::Controller, &wire, "delivery prior with a bundle").await;
    Ok(())
}

/// Download grants are unsequenced, may be refused, and their URL never appears in debug output.
#[tokio::test]
async fn download_grants_round_trip_without_exposing_the_url() -> Result<(), TestError> {
    let url = "https://objects.example.com/bucket/revision.bundle?X-Amz-Signature=secret";
    let granted = Case {
        message: Message::Controller(ControllerToNodeMessage::DownloadGrant(
            DownloadGrantMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: OperationId::new(OPERATION),
                execution_id: ExecutionId::new(EXECUTION),
                payload: DownloadGrant::Granted {
                    grants: vec![ObjectDownloadGrant {
                        object_key: ObjectKey::new(PRIOR_BUNDLE),
                        url: PresignedUrl::new(url),
                        method: DownloadMethod::Get,
                        headers: [("x-amz-meta-proof".to_owned(), "secret".to_owned())].into(),
                        expires_at: time::macros::datetime!(2026-10-10 12:15:00 UTC),
                    }],
                },
            },
        )),
        wire: json!({"message_type": "download_grant", "protocol_version": 1,
            "operation_id": OPERATION, "execution_id": EXECUTION,
            "payload": {"outcome": "granted", "grants": [{"object_key": PRIOR_BUNDLE,
                "url": url, "method": "GET", "headers": {"x-amz-meta-proof": "secret"},
                "expires_at": "2026-10-10T12:15:00Z"}]}}),
    };
    let refused = Case {
        message: Message::Controller(ControllerToNodeMessage::DownloadGrant(
            DownloadGrantMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: OperationId::new(OPERATION),
                execution_id: ExecutionId::new(EXECUTION),
                payload: DownloadGrant::Refused {},
            },
        )),
        wire: json!({"message_type": "download_grant", "protocol_version": 1,
            "operation_id": OPERATION, "execution_id": EXECUTION,
            "payload": {"outcome": "refused"}}),
    };
    for case in [&granted, &refused] {
        case.assert_wire().await?;
        case.assert_round_trip().await?;
        case.assert_envelope_rejections().await?;
    }
    let Message::Controller(message) = &granted.message else {
        unreachable!("the grant fixture is a Controller message");
    };
    assert!(!format!("{message:?}").contains("X-Amz-Signature"));

    let mut wire = granted.wire.clone();
    replace(&mut wire, "/payload/grants", json!([]));
    reject_semantics(
        Peer::Controller,
        &wire,
        "no grants",
        MessageValidationError::EmptyDownloadGrant,
    )
    .await?;
    let mut wire = granted.wire.clone();
    replace(&mut wire, "/payload/grants/0/url", json!("revision.bundle"));
    reject_semantics(
        Peer::Controller,
        &wire,
        "relative url",
        MessageValidationError::InvalidDownloadGrant,
    )
    .await?;
    // A download grant is a read; a write method is not part of the contract.
    let mut wire = granted.wire.clone();
    replace(&mut wire, "/payload/grants/0/method", json!("PUT"));
    reject_structure(Peer::Controller, &wire, "write method").await;

    let needed = Case {
        message: Message::Node(NodeToControllerMessage::DownloadGrantNeeded(
            DownloadGrantNeededMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: OperationId::new(OPERATION),
                execution_id: ExecutionId::new(EXECUTION),
                payload: DownloadGrantNeeded {
                    node_id: NodeId::new("node-1"),
                },
            },
        )),
        wire: json!({"message_type": "download_grant_needed", "protocol_version": 1,
            "operation_id": OPERATION, "execution_id": EXECUTION,
            "payload": {"node_id": "node-1"}}),
    };
    needed.assert_wire().await?;
    needed.assert_round_trip().await?;
    needed.assert_envelope_rejections().await?;
    needed
        .assert_fields(
            &["/payload/node_id"],
            &[
                ("/operation_id", "operation_id"),
                ("/execution_id", "execution_id"),
                ("/payload/node_id", "node_id"),
            ],
        )
        .await
}
