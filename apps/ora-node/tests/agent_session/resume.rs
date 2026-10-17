//! Sessions that resume a prior Revision: the restore runs before any plugin, a failure ends the
//! session with its code, and a rewritten remote history is noted in the first turn.
use super::field;
use super::support::*;
use ora_node::SessionCommand;
use ora_node::{RestoreFailure, RestoreRequest, Restored};
use ora_node_protocol::{
    AgentSessionEndReason, AgentSessionEnded, CommitId, DeliveryRef, EndSessionReason, ExecutionId,
    GitRepositoryUrl, OperationId, PriorRevision, RevisionId,
};
use pretty_assertions::assert_eq;

const PRIOR_FINAL: &str = "89abcdef0123456789abcdef0123456789abcdef";
const BASE: &str = "0123456789abcdef0123456789abcdef01234567";

/// The prior Revision every resumed session of these tests names.
fn prior() -> PriorRevision {
    PriorRevision {
        revision_id: RevisionId::new("revision-1"),
        final_commit: CommitId::new(PRIOR_FINAL),
        base_commit: CommitId::new(BASE),
        git_repository_url: GitRepositoryUrl::parse("https://git.ora.test/tenant/project.git")
            .unwrap(),
        delivery_ref: DeliveryRef::parse("refs/ora/deliveries/prior-work").unwrap(),
    }
}

/// A restore that fails ends the session with the restore's code before any plugin process,
/// lease or Thread record exists; the restore saw the session's own checkout and prior Revision.
///
/// Evidence for specs/decisions/node/revision/20261010-restore-prior-revision-before-session.md D1
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_restore_ends_the_session_before_any_plugin_starts() {
    for (failure, detail) in [
        (RestoreFailure::Unavailable, "prior_revision_unavailable"),
        (
            RestoreFailure::BaseUnavailable,
            "prior_revision_base_unavailable",
        ),
    ] {
        let fixture = Fixture::new();
        *fixture.restorer.answer.lock().unwrap() = Err(failure);
        let sessions = fixture.sessions(PLUGIN_VERSION);
        fixture.start_resuming(&sessions, PLUGIN_VERSION, "hello", Some(prior()));
        let ended = fixture.ended().await;
        assert_eq!(
            (
                ended,
                fixture.restorer.requests.lock().unwrap().clone(),
                fixture.plugin_pids(),
                fixture.ledger.events(),
                fixture.leases.load(std::sync::atomic::Ordering::SeqCst),
            ),
            (
                AgentSessionEnded {
                    node: fixture.node(),
                    reason: AgentSessionEndReason::AgentFailed,
                    detail: Some(detail.to_string()),
                },
                vec![RestoreRequest {
                    operation: OperationId::new(OPERATION),
                    execution: ExecutionId::new(EXECUTION),
                    checkout: fixture.checkout(),
                    branch: "main".into(),
                    base_commit: CommitId::new(BASE),
                    prior: prior(),
                }],
                Vec::new(),
                Vec::new(),
                0,
            ),
        );
    }
}

/// After a restore onto rewritten remote history, the first turn ends with one fixed note naming
/// the restored commit, its lost base and the remote branch; a fresh session restores nothing.
///
/// Evidence for specs/decisions/node/revision/20261010-restore-prior-revision-before-session.md D4
#[tokio::test(flavor = "multi_thread")]
async fn a_rewritten_remote_history_is_noted_in_the_first_turn() {
    let fixture = Fixture::new();
    *fixture.restorer.answer.lock().unwrap() = Ok(Restored::Diverged {
        final_commit: CommitId::new(PRIOR_FINAL),
        base_commit: CommitId::new(BASE),
        branch: "main".into(),
    });
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_resuming(&sessions, PLUGIN_VERSION, "hello", Some(prior()));
    until(|| {
        fixture
            .ledger
            .events()
            .iter()
            .any(|event| field(&event.record, &["type"]) == "turnEnded")
            .then_some(())
    })
    .await;
    fixture.command(
        &sessions,
        "command-end",
        SessionCommand::EndSession(EndSessionReason::UserEnded),
    );
    fixture.ended().await;
    let prompt: Vec<String> = fixture
        .history_lines()
        .iter()
        .filter(|line| field(line, &["update", "sessionUpdate"]) == "user_message_chunk")
        .map(|line| field(line, &["update", "content", "text"]))
        .collect();
    assert_eq!(
        prompt,
        vec![
            "hello".to_string(),
            format!(
                "Note from Ora: this run resumed the previous run's work at commit {PRIOR_FINAL}. \
                 That work was built on commit {BASE}, which is not in the history of this Workspace's snapshot: \
                 the external repository's history was rewritten since. The local branch main points at the resumed work; \
                 run `ora-git fetch upstream` to see the external repository's latest content and reconcile it."
            ),
        ]
    );

    let fresh = Fixture::new();
    let sessions = fresh.sessions(PLUGIN_VERSION);
    fresh.start(&sessions, PLUGIN_VERSION, "hello");
    fresh.command(
        &sessions,
        "command-end",
        SessionCommand::EndSession(EndSessionReason::UserEnded),
    );
    fresh.ended().await;
    assert_eq!(fresh.restorer.requests.lock().unwrap().clone(), vec![]);
}

/// An `EndSession` that arrives while the restore runs cancels the restore, waits for it, and
/// ends the session with the command's reason before any plugin starts.
///
/// Evidence for specs/decisions/node/revision/20261010-restore-prior-revision-before-session.md D1
#[tokio::test(flavor = "multi_thread")]
async fn an_end_during_the_restore_cancels_it_before_any_plugin_starts() {
    let fixture = Fixture::new();
    fixture
        .restorer
        .hold
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let sessions = fixture.sessions(PLUGIN_VERSION);
    fixture.start_resuming(&sessions, PLUGIN_VERSION, "hello", Some(prior()));
    until(|| (!fixture.restorer.requests.lock().unwrap().is_empty()).then_some(())).await;
    fixture.command(
        &sessions,
        "command-end",
        SessionCommand::EndSession(EndSessionReason::UserEnded),
    );
    let ended = fixture.ended().await;
    assert_eq!(
        (
            ended,
            fixture
                .restorer
                .cancelled
                .load(std::sync::atomic::Ordering::SeqCst),
            fixture.plugin_pids(),
            fixture.ledger.settlements(),
        ),
        (
            AgentSessionEnded {
                node: fixture.node(),
                reason: AgentSessionEndReason::UserEnded,
                detail: None,
            },
            1,
            Vec::new(),
            vec![(
                "command-end".to_string(),
                vec![ora_node::CommandSettlement::Executed]
            )],
        ),
    );
}
