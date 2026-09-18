use super::support::{Case, Message, Peer, TestError, reject_semantics, reject_structure};
use ora_node_protocol::*;
use pretty_assertions::assert_eq;
use serde_json::json;

/// Defines an independent wire fixture without a Main Workspace or destination input.
fn clone_request() -> Case {
    Case {
        message: Message::Controller(ControllerToNodeMessage::CloneRepository(
            CloneRepositoryMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                request_id: Some(RequestId::new("request-clone")),
                operation_id: OperationId::new("operation-clone"),
                execution_id: ExecutionId::new("execution-clone"),
                payload: CloneRepository {
                    spec: CloneExecutionSpec {
                        node_id: NodeId::new("node-1"),
                        repository: CloneRepositoryUrl::parse("https://example.com/team/repo.git")
                            .unwrap_or_else(|error| panic!("invalid fixture source: {error}")),
                        branch: BranchName::new("feature/clone"),
                    },
                },
            },
        )),
        wire: json!({
            "message_type": "clone_repository",
            "protocol_version": 1,
            "request_id": "request-clone",
            "operation_id": "operation-clone",
            "execution_id": "execution-clone",
            "payload": {"spec": {
                "node_id": "node-1",
                "repository": "https://example.com/team/repo.git",
                "branch": "feature/clone"
            }}
        }),
    }
}

/// Covers the input seam of the approved clone contract, not execution or authentication.
/// specs/test-cases/node/repository/minimal-clone-execution.md#clone-input-and-authentication-stay-noninteractive
#[tokio::test]
async fn clone_request_has_independent_wire_and_fragmented_round_trip() -> Result<(), TestError> {
    let case = clone_request();
    case.assert_wire().await?;
    case.assert_round_trip().await?;
    case.assert_envelope_rejections().await?;
    case.assert_fields(
        &[
            "/operation_id",
            "/execution_id",
            "/payload/spec/node_id",
            "/payload/spec/repository",
            "/payload/spec/branch",
        ],
        &[
            ("/operation_id", "operation_id"),
            ("/execution_id", "execution_id"),
            ("/request_id", "request_id"),
            ("/payload/spec/node_id", "node_id"),
        ],
    )
    .await?;
    Ok(())
}

/// Allows deployments to choose a source while preserving distinct original input spellings.
#[tokio::test]
async fn clone_sources_preserve_valid_https_and_explicit_ssh_inputs() -> Result<(), TestError> {
    for source in [
        "https://example.com/team/repo.git",
        "https://EXAMPLE.com:443/team/repo.git",
        "https://example.com/team/a%20b.git",
        "ssh://git@example.com/team/repo.git",
        "ssh://example.com:2222/team/repo.git",
        "ssh://git@[::1]:2222/team/repo.git",
    ] {
        let parsed = CloneRepositoryUrl::parse(source)
            .unwrap_or_else(|error| panic!("invalid fixture source: {error}"));
        assert_eq!(parsed.as_str(), source);
        assert_eq!(serde_json::to_value(&parsed)?, json!(source));
        assert_eq!(format!("{parsed:?}"), "CloneRepositoryUrl([redacted])");
        let mut case = clone_request();
        case.wire["payload"]["spec"]["repository"] = json!(source);
        case.message = Message::Controller(serde_json::from_value(case.wire.clone())?);
        case.assert_wire().await?;
        case.assert_round_trip().await?;
    }
    assert_ne!(
        CloneRepositoryUrl::parse("https://example.com/repo"),
        CloneRepositoryUrl::parse("https://EXAMPLE.com:443/repo"),
    );
    Ok(())
}

/// Invalid addresses cannot be constructed or decoded and errors never echo their contents.
#[tokio::test]
async fn clone_rejects_other_transports_credentials_and_ambiguous_urls() {
    for source in [
        "",
        "/tmp/repo",
        "C:\\repo",
        "file:///tmp/repo",
        "git@example.com:repo.git",
        "http://example.com/repo",
        "git://example.com/repo",
        "ext::command",
        "https:example.com/repo",
        "https:///example.com/repo",
        "https://example.com",
        "ssh://example.com",
        "https://example.com/repo\n",
        " https://example.com/repo",
        "https://example.com\\repo",
        "https://user:secret@example.com/repo",
        "https://token@example.com/repo",
        "https://@example.com/repo",
        "ssh://user:secret@example.com/repo",
        "ssh://user:@example.com/repo",
        "ssh://@example.com/repo",
        "https://example.com/repo?token=secret",
        "ssh://git@example.com/repo#secret",
        "https://example.com/repo?",
    ] {
        assert_eq!(
            CloneRepositoryUrl::parse(source),
            Err(InvalidCloneRepositoryUrl)
        );
        let mut wire = clone_request().wire;
        wire["payload"]["spec"]["repository"] = json!(source);
        reject_structure(Peer::Controller, &wire, "invalid clone source").await;
        let Err(error) = serde_json::from_value::<ControllerToNodeMessage>(wire) else {
            panic!("invalid source accepted");
        };
        assert!(!error.to_string().contains("secret"));
        assert!(!format!("{error:?}").contains("secret"));
    }
}

/// Literal branch validation runs on both codec directions, before outbound bytes are written.
#[tokio::test]
async fn clone_rejects_revision_expressions_and_invalid_branches() -> Result<(), TestError> {
    for branch in [
        "",
        " ",
        "HEAD",
        "HEAD~1",
        "refs/heads/main",
        "refs/tags/v1",
        "@{-1}",
        "--upload-pack=command",
        "a..b",
        "a.lock",
        "a:b",
        "feature\nbranch",
    ] {
        let mut wire = clone_request().wire;
        wire["payload"]["spec"]["branch"] = json!(branch);
        reject_semantics(
            Peer::Controller,
            &wire,
            "invalid branch",
            MessageValidationError::InvalidCloneBranch,
        )
        .await?;
    }
    Ok(())
}

/// Caller-selected execution controls must fail rather than silently being ignored.
#[tokio::test]
async fn clone_rejects_execution_controls_in_payload_and_spec() {
    for field in [
        "target_path",
        "credentials",
        "git_args",
        "environment",
        "main_workspace",
    ] {
        for parent in ["/payload", "/payload/spec"] {
            let mut wire = clone_request().wire;
            wire.pointer_mut(parent)
                .unwrap_or_else(|| panic!("missing fixture object"))[field] = json!("not allowed");
            reject_structure(Peer::Controller, &wire, "unexpected clone input").await;
        }
    }
}

/// Client request correlation remains optional without changing execution identity.
#[tokio::test]
async fn clone_request_without_client_request_id_round_trips() -> Result<(), TestError> {
    let mut case = clone_request();
    case.wire
        .as_object_mut()
        .unwrap_or_else(|| panic!("missing fixture object"))
        .remove("request_id");
    case.message = Message::Controller(serde_json::from_value(case.wire.clone())?);
    case.assert_wire().await?;
    case.assert_round_trip().await?;
    Ok(())
}
