use super::support::{Case, Message, Peer, TestError, reject_semantics, reject_structure};
use ora_node_protocol::*;
use pretty_assertions::assert_eq;
use serde_json::json;

/// The snapshot commit every clone fixture names.
pub(super) const BASE_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
/// A Git Server repository address in the form Cloud issues.
pub(super) const REPOSITORY_URL: &str = "https://git.ora.test/2f1b7a52-3c4d-4e5f-8a9b-0c1d2e3f4a5b/7a8b9c0d-1e2f-4a3b-8c4d-5e6f7a8b9c0d.git";
/// The Binding snapshot ref of `BASE_COMMIT`.
pub(super) const SNAPSHOT_REF: &str = "refs/ora/upstreams/9d8c7b6a-5f4e-4d3c-8b2a-1f0e9d8c7b6a/snapshots/0123456789abcdef0123456789abcdef01234567";
/// The Workspace branch the checkout's local branch is named after.
pub(super) const WORKSPACE_REF: &str =
    "refs/heads/workspaces/4c3b2a19-0f8e-4d7c-9b6a-5f4e3d2c1b0a/main";

/// The clone input shared by command and result fixtures.
pub(super) fn clone_spec() -> CloneExecutionSpec {
    CloneExecutionSpec {
        node_id: NodeId::new("node-1"),
        repository_url: GitRepositoryUrl::parse(REPOSITORY_URL)
            .unwrap_or_else(|error| panic!("invalid fixture source: {error}")),
        snapshot_ref: SnapshotRef::parse(SNAPSHOT_REF)
            .unwrap_or_else(|error| panic!("invalid fixture snapshot: {error}")),
        base_commit: CommitId::new(BASE_COMMIT),
        workspace_ref: WorkspaceRef::parse(WORKSPACE_REF)
            .unwrap_or_else(|error| panic!("invalid fixture branch: {error}")),
    }
}

/// The independent wire spelling of `clone_spec`.
pub(super) fn clone_spec_wire() -> serde_json::Value {
    json!({
        "node_id": "node-1",
        "repository_url": REPOSITORY_URL,
        "snapshot_ref": SNAPSHOT_REF,
        "base_commit": BASE_COMMIT,
        "workspace_ref": WORKSPACE_REF
    })
}

/// Defines an independent wire fixture without a Main Workspace or destination input.
fn clone_request() -> Case {
    Case {
        message: Message::Controller(ControllerToNodeMessage::CloneRepository(
            CloneRepositoryMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                request_id: Some(RequestId::new("request-clone")),
                operation_id: OperationId::new("operation-clone"),
                execution_id: ExecutionId::new("execution-clone"),
                payload: CloneRepository { spec: clone_spec() },
            },
        )),
        wire: json!({
            "message_type": "clone_repository",
            "protocol_version": 1,
            "request_id": "request-clone",
            "operation_id": "operation-clone",
            "execution_id": "execution-clone",
            "payload": {"spec": clone_spec_wire()}
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
            "/payload/spec/repository_url",
            "/payload/spec/snapshot_ref",
            "/payload/spec/base_commit",
            "/payload/spec/workspace_ref",
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

/// Git Server addresses keep their original spelling; nothing but HTTPS without user
/// information, query or fragment is accepted, and errors never echo the address.
#[tokio::test]
async fn clone_accepts_only_git_server_https_addresses() -> Result<(), TestError> {
    for source in [
        REPOSITORY_URL,
        "https://GIT.ora.test:8443/tenant/project.git",
        "https://127.0.0.1:9443/tenant/project.git",
    ] {
        let parsed = GitRepositoryUrl::parse(source)
            .unwrap_or_else(|error| panic!("invalid fixture source: {error}"));
        assert_eq!(parsed.as_str(), source);
        let mut case = clone_request();
        case.wire["payload"]["spec"]["repository_url"] = json!(source);
        case.message = Message::Controller(serde_json::from_value(case.wire.clone())?);
        case.assert_wire().await?;
        case.assert_round_trip().await?;
    }
    for source in [
        "",
        "/tmp/repo",
        "file:///tmp/repo",
        "git@example.com:repo.git",
        "http://example.com/repo",
        "ssh://git@example.com/team/repo.git",
        "ssh://example.com:2222/team/repo.git",
        "git://example.com/repo",
        "ext::command",
        "https:example.com/repo",
        "https:///example.com/repo",
        "https://example.com",
        "https://example.com/",
        "https://example.com/repo\n",
        " https://example.com/repo",
        "https://example.com\\repo",
        "https://user:secret@example.com/repo",
        "https://secret@example.com/repo",
        "https://@example.com/repo",
        "https://example.com/repo?token=secret",
        "https://example.com/repo#secret",
        "https://example.com/repo?",
    ] {
        assert_eq!(
            GitRepositoryUrl::parse(source),
            Err(InvalidGitRepositoryUrl)
        );
        let mut wire = clone_request().wire;
        wire["payload"]["spec"]["repository_url"] = json!(source);
        reject_structure(Peer::Controller, &wire, "invalid clone source").await;
        let Err(error) = serde_json::from_value::<ControllerToNodeMessage>(wire) else {
            panic!("invalid source accepted");
        };
        assert!(!error.to_string().contains("secret"));
    }
    Ok(())
}

/// Each snapshot field is checked on its own: refs outside their namespace, non-canonical IDs
/// and abbreviated or uppercase commits cannot be decoded.
#[tokio::test]
async fn clone_rejects_malformed_snapshot_inputs() {
    let binding = "9d8c7b6a-5f4e-4d3c-8b2a-1f0e9d8c7b6a";
    let workspace = "4c3b2a19-0f8e-4d7c-9b6a-5f4e3d2c1b0a";
    let rejected: Vec<(&str, String)> = vec![
        ("snapshot_ref", String::new()),
        ("snapshot_ref", format!("refs/heads/{BASE_COMMIT}")),
        (
            "snapshot_ref",
            format!("refs/ora/upstreams/{binding}/heads/main"),
        ),
        (
            "snapshot_ref",
            format!("refs/ora/upstreams/{binding}/snapshots/0123456"),
        ),
        (
            "snapshot_ref",
            format!(
                "refs/ora/upstreams/{}/snapshots/{BASE_COMMIT}",
                binding.to_uppercase()
            ),
        ),
        (
            "snapshot_ref",
            format!("refs/ora/upstreams/binding-1/snapshots/{BASE_COMMIT}"),
        ),
        (
            "snapshot_ref",
            format!(
                "refs/ora/upstreams/{binding}/snapshots/{}",
                BASE_COMMIT.to_uppercase()
            ),
        ),
        (
            "snapshot_ref",
            format!("refs/ora/upstreams/{binding}/snapshots/{BASE_COMMIT}/x"),
        ),
        ("workspace_ref", String::new()),
        ("workspace_ref", "refs/heads/main".to_owned()),
        (
            "workspace_ref",
            format!("refs/heads/workspaces/{workspace}"),
        ),
        (
            "workspace_ref",
            format!("refs/heads/workspaces/{workspace}/HEAD"),
        ),
        (
            "workspace_ref",
            format!("refs/heads/workspaces/{workspace}/-x"),
        ),
        (
            "workspace_ref",
            format!("refs/heads/workspaces/{workspace}/a..b"),
        ),
        (
            "workspace_ref",
            format!("refs/heads/workspaces/{workspace}/a:b"),
        ),
        (
            "workspace_ref",
            format!("refs/heads/workspaces/{workspace}/a*"),
        ),
        (
            "workspace_ref",
            format!("refs/heads/workspaces/{workspace}/{}", "b".repeat(201)),
        ),
        (
            "workspace_ref",
            "refs/heads/workspaces/workspace-1/main".to_owned(),
        ),
        (
            "workspace_ref",
            format!("refs/ora/workspaces/{workspace}/main"),
        ),
    ];
    for (field, value) in rejected {
        let mut wire = clone_request().wire;
        wire["payload"]["spec"][field] = json!(value);
        reject_structure(Peer::Controller, &wire, field).await;
    }
}

/// The base commit must be a full lowercase object ID, and the snapshot ref's last segment must
/// name exactly that commit.
#[tokio::test]
async fn clone_rejects_a_base_commit_the_snapshot_ref_does_not_name() -> Result<(), TestError> {
    for (commit, expected) in [
        ("", MessageValidationError::InvalidCloneCommit),
        ("HEAD", MessageValidationError::InvalidCloneCommit),
        ("01234567", MessageValidationError::InvalidCloneCommit),
        (
            "0123456789ABCDEF0123456789ABCDEF01234567",
            MessageValidationError::InvalidCloneCommit,
        ),
        (
            "1123456789abcdef0123456789abcdef01234567",
            MessageValidationError::SnapshotCommitMismatch,
        ),
    ] {
        let mut wire = clone_request().wire;
        wire["payload"]["spec"]["base_commit"] = json!(commit);
        reject_semantics(Peer::Controller, &wire, "base commit", expected).await?;
    }
    Ok(())
}

/// The external-clone shape from before the Git Server is no longer understood.
#[tokio::test]
async fn clone_rejects_the_retired_repository_and_branch_shape() {
    let mut wire = clone_request().wire;
    wire["payload"]["spec"] = json!({
        "node_id": "node-1",
        "repository": "https://example.com/team/repo.git",
        "branch": "main"
    });
    reject_structure(Peer::Controller, &wire, "retired clone shape").await;
    for field in ["repository", "branch"] {
        let mut wire = clone_request().wire;
        wire["payload"]["spec"][field] = json!("main");
        reject_structure(Peer::Controller, &wire, field).await;
    }
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
