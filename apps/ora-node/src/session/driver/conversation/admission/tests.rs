use super::*;
use pretty_assertions::assert_eq;

#[tokio::test(start_paused = true)]
async fn timeout_and_actor_commit_have_one_winner_at_the_deadline() {
    let deadline = Instant::now() + std::time::Duration::from_secs(300);
    let mut committed = TurnGate {
        deadline: Some(deadline),
        ..Default::default()
    };
    committed.commit(|| Ok(())).unwrap();
    tokio::time::advance(std::time::Duration::from_secs(300)).await;
    assert!(!committed.expire());
    let mut late = TurnGate {
        deadline: Some(deadline),
        ..Default::default()
    };
    assert!(
        late.commit(|| panic!("late Actor must not write Executed"))
            .is_err()
    );
    assert!(late.expire());
    let mut cancelled = TurnGate::default();
    assert!(cancelled.expire());
    assert!(
        cancelled
            .commit(|| panic!("cancelled attempt must not commit"))
            .is_err()
    );
}

#[tokio::test(start_paused = true)]
async fn six_minute_business_turn_does_not_consume_queued_mcp_budget() {
    let turn = ora_node_protocol::TurnId::new("queued");
    let clocks = std::sync::Mutex::new(std::collections::HashMap::from([(
        turn.clone(),
        Instant::now(),
    )]));
    tokio::time::advance(std::time::Duration::from_secs(10)).await;
    let business = BusinessTime::new(&clocks);
    tokio::time::advance(std::time::Duration::from_secs(360)).await;
    drop(business);
    let deadline = clocks.lock().unwrap()[&turn] + std::time::Duration::from_secs(300);
    assert_eq!(
        deadline - Instant::now(),
        std::time::Duration::from_secs(290)
    );
    tokio::time::advance(std::time::Duration::from_secs(289)).await;
    assert!(Instant::now() < deadline);
    tokio::time::advance(std::time::Duration::from_secs(1)).await;
    assert_eq!(Instant::now(), deadline);
}

#[test]
fn agent_failures_are_terminal_while_typed_mcp_and_busy_results_wait() {
    assert!(!mcp_wait(&RuntimeError::internal(
        "readiness",
        ReadinessUnavailable
    )));
    assert!(!mcp_wait(&RuntimeError::internal(
        "transport",
        std::io::Error::other("broken")
    )));
    assert!(mcp_wait(&RuntimeError::internal(
        "pending",
        crate::session::mcp::control::Admission::Pending
    )));
    assert!(mcp_wait(&RuntimeError::internal(
        "blocked",
        crate::session::mcp::control::Admission::Blocked
    )));
    assert!(mcp_wait(&RuntimeError::new(
        ora_agent_runtime::ErrorClassification::Conflict,
        PublicError::SessionBusy(ora_contracts::EmptyErrorParams {}),
        "busy"
    )));
}
