//! Resuming from a prior Revision: the prior Revision in session and delivery inputs.
use super::support::*;
use ora_node_protocol::*;
use serde_json::json;

const OPERATION: &str = "run-2";
const EXECUTION: &str = "execution-session";
const PRIOR_FINAL: &str = "89abcdef0123456789abcdef0123456789abcdef";
const PRIOR_BASE: &str = "0123456789abcdef0123456789abcdef01234567";
const REPOSITORY: &str = "https://git.ora.test/tenant-1/project-1.git";
const PRIOR_DELIVERY: &str = "refs/ora/deliveries/work-1";

/// The prior Revision a resumed session restores.
fn prior() -> PriorRevision {
    PriorRevision {
        revision_id: RevisionId::new("revision-1"),
        final_commit: CommitId::new(PRIOR_FINAL),
        git_repository_url: GitRepositoryUrl::parse(REPOSITORY)
            .unwrap_or_else(|error| panic!("invalid fixture: {error}")),
        delivery_ref: DeliveryRef::parse(PRIOR_DELIVERY)
            .unwrap_or_else(|error| panic!("invalid fixture: {error}")),
        base_commit: CommitId::new(PRIOR_BASE),
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
        "git_repository_url": REPOSITORY,
        "delivery_ref": PRIOR_DELIVERY,
        "base_commit": PRIOR_BASE
    });
    case
}

/// A delivery of a resumed session, which carries the prior final commit only.
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
/// input, so a Node can verify the commit it fetches against exactly these values.
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
                "/payload/spec/prior_revision/git_repository_url",
                "/payload/spec/prior_revision/delivery_ref",
                "/payload/spec/prior_revision/base_commit",
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
            "/payload/spec/prior_revision/base_commit",
            json!("01234567"),
            MessageValidationError::InvalidCommit,
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
    for (path, value) in [
        (
            "/payload/spec/prior_revision/git_repository_url",
            "http://git.ora.test/p.git",
        ),
        (
            "/payload/spec/prior_revision/delivery_ref",
            "refs/heads/main",
        ),
    ] {
        let mut wire = resumed_start().wire;
        replace(&mut wire, path, json!(value));
        reject_structure(Peer::Controller, &wire, path).await;
    }
    // A delivery never fetches the prior Revision, so its input cannot say where from; the
    // bundle of the retired shape is refused as well.
    for (field, value) in [
        ("git_repository_url", json!(REPOSITORY)),
        (
            "bundle",
            json!({"key": "runs/prior/revision.bundle", "size": 1, "sha256": "00"}),
        ),
    ] {
        let mut wire = resumed_delivery().wire;
        wire["payload"]["spec"]["prior_revision"][field] = value;
        reject_structure(Peer::Controller, &wire, field).await;
    }
    let mut wire = resumed_start().wire;
    wire["payload"]["spec"]["prior_revision"]["bundle"] = json!({"key": "k", "size": 1});
    reject_structure(Peer::Controller, &wire, "session prior with a bundle").await;
    Ok(())
}

/// The download grant exchange is gone: the Node fetches the prior Revision from the Git Server.
#[tokio::test]
async fn download_grant_messages_are_not_part_of_the_protocol() {
    reject_structure(
        Peer::Controller,
        &json!({"message_type": "download_grant", "protocol_version": 1,
            "operation_id": OPERATION, "execution_id": EXECUTION,
            "payload": {"outcome": "refused"}}),
        "download grant",
    )
    .await;
    reject_structure(
        Peer::Node,
        &json!({"message_type": "download_grant_needed", "protocol_version": 1,
            "operation_id": OPERATION, "execution_id": EXECUTION,
            "payload": {"node_id": "node-1"}}),
        "download grant request",
    )
    .await;
}
