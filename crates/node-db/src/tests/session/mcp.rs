//! Generation rejection never persists work; a timed-out turn and its event commit together.
use super::*;
use pretty_assertions::assert_eq;

#[test]
fn floor_rejection_is_retryable_and_duplicates_compare_before_current_generation() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = NodeDatabase::open(
        &dir.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
    )
    .unwrap();
    let mut start = command();
    start.payload.spec.mcp_lifecycle = true;
    db.accept_session(&start).unwrap();
    let mut input = turn("turn");
    assert_eq!(
        db.accept_session_command_at_generation(&input, Some(0))
            .unwrap(),
        CommandAdmission::McpUpdateRequired
    );
    if let SessionCommandInput::SubmitUserTurn(turn) = &mut input {
        turn.payload.mcp_generation = Some(2);
    }
    assert_eq!(
        db.accept_session_command_at_generation(&input, Some(1))
            .unwrap(),
        CommandAdmission::McpUpdateRequired
    );
    assert!(
        db.session_journal()
            .unwrap()
            .queued_commands(&start.execution_id)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        db.accept_session_command_at_generation(&input, Some(2))
            .unwrap(),
        CommandAdmission::Accepted
    );
    assert_eq!(
        db.accept_session_command_at_generation(&input, Some(0))
            .unwrap(),
        CommandAdmission::Accepted
    );
    if let SessionCommandInput::SubmitUserTurn(turn) = &mut input {
        turn.payload.mcp_generation = Some(3);
    }
    assert!(
        db.accept_session_command_at_generation(&input, Some(3))
            .is_err()
    );
}

#[test]
fn failure_settlement_is_durable_idempotent_and_does_not_end_session() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut db = NodeDatabase::open(&path, NodeIdentity::Require(NodeId::new("node"))).unwrap();
    let start = command();
    db.accept_session(&start).unwrap();
    let input = turn("turn");
    db.accept_session_command(&input).unwrap();
    let journal = db.session_journal().unwrap();
    let failure = SessionTurnFailed {
        turn_id: start.payload.spec.initial_turn.turn_id.clone(),
        command_id: Some(input.command_id().clone()),
        code: SessionTurnFailureCode::McpRefreshWaitTimeout,
    };
    let sequence = journal
        .fail_session_turn(&start.execution_id, failure.clone())
        .unwrap();
    assert_eq!(
        journal
            .fail_session_turn(&start.execution_id, failure.clone())
            .unwrap(),
        sequence
    );
    assert_eq!(
        journal
            .command_state(&start.execution_id, input.command_id())
            .unwrap(),
        Some(SessionCommandState::Failed)
    );
    assert!(
        journal
            .queued_commands(&start.execution_id)
            .unwrap()
            .is_empty()
    );
    assert!(!matches!(
        db.find_session(&start.operation_id, &start.execution_id)
            .unwrap()
            .unwrap()
            .state,
        ExecutionState::Completed(_)
    ));
    drop(journal);
    drop(db);
    let db = NodeDatabase::open(&path, NodeIdentity::Discover).unwrap();
    assert_eq!(
        db.session_journal()
            .unwrap()
            .fail_session_turn(&start.execution_id, failure)
            .unwrap(),
        sequence
    );
}
