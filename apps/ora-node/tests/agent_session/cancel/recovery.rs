//! Case 18: a prompt rejection can invalidate only the provider session, leaving its process alive.
use super::*;
use pretty_assertions::assert_eq;

/// Reads the real ACP methods received by the fixture, excluding initialize and close traffic.
fn conversation_methods(fixture: &Fixture) -> Vec<String> {
    std::fs::read_to_string(fixture.package_root().join("echo-acp-methods"))
        .unwrap_or_default()
        .lines()
        .filter(|method| matches!(*method, "session/new" | "session/load" | "session/prompt"))
        .map(str::to_owned)
        .collect()
}

/// Both normal and MCP lifecycle sessions restore the original provider id before another prompt.
#[test]
fn an_invalidated_provider_session_is_loaded_before_the_next_prompt() {
    super::super::with_scoped_async_test(async {
        for lifecycle in [false, true] {
            let mut fixture = Fixture::new();
            fixture.verify_turn_boundary();
            super::super::mcp::configure(&fixture, /*load*/ true, /*http*/ true);
            let sessions = fixture.sessions(PLUGIN_VERSION);
            let mut input = super::super::mcp::input(Vec::new());
            input.payload.spec.mcp_lifecycle = lifecycle;
            input.payload.spec.keep_session_on_failure = true;
            input.payload.spec.initial_turn = turn("turn-1", "[session-closed]");
            sessions.start(input.operation_id, input.execution_id, input.payload.spec);
            until(|| (turn_ends(&fixture) == 1).then_some(())).await;
            fixture.command(
                &sessions,
                "command-2",
                SessionCommand::SubmitUserTurn(turn("turn-2", "hello")),
            );
            until(|| (turn_ends(&fixture) == 2).then_some(())).await;
            assert_eq!(
                conversation_methods(&fixture),
                [
                    "session/new",
                    "session/prompt",
                    "session/load",
                    "session/prompt"
                ]
            );
            assert_eq!(fixture.ledger.ended(), None);
            assert_eq!(
                end(&fixture, &sessions).await,
                AgentSessionEndReason::UserEnded
            );
        }
    });
}

/// A lifecycle load refusal leaves admission blocked instead of silently making a new provider.
#[test]
fn a_lifecycle_restore_failure_blocks_without_creating_a_provider() {
    super::super::with_scoped_async_test(async {
        let mut fixture = Fixture::new();
        fixture.verify_turn_boundary();
        super::super::mcp::configure(&fixture, /*load*/ true, /*http*/ true);
        let sessions = fixture.sessions(PLUGIN_VERSION);
        let mut input = super::super::mcp::input(Vec::new());
        input.payload.spec.mcp_lifecycle = true;
        input.payload.spec.keep_session_on_failure = true;
        input.payload.spec.keep_session_on_cancel = true;
        input.payload.spec.initial_turn = turn("turn-1", "[session-closed]");
        sessions.start(input.operation_id, input.execution_id, input.payload.spec);
        until(|| (turn_ends(&fixture) == 1).then_some(())).await;
        std::fs::write(fixture.package_root().join("fail-load"), b"").unwrap();
        fixture.command(
            &sessions,
            "command-2",
            SessionCommand::SubmitUserTurn(turn("turn-2", "hello")),
        );
        until(|| {
            conversation_methods(&fixture)
                .contains(&"session/load".to_owned())
                .then_some(())
        })
        .await;
        fixture.command(
            &sessions,
            "cancel-2",
            SessionCommand::CancelTurn(TurnId::new("turn-2")),
        );
        assert_eq!(
            settled(&fixture, "cancel-2").await,
            TurnCancelOutcome::NotStarted
        );
        assert_eq!(
            conversation_methods(&fixture),
            ["session/new", "session/prompt", "session/load"]
        );
        assert_eq!((turn_ends(&fixture), fixture.ledger.ended()), (1, None));
        assert_eq!(
            end(&fixture, &sessions).await,
            AgentSessionEndReason::UserEnded
        );
    });
}

/// Case 18 and the ADR review follow-up: repeated restores count toward the process budget.
/// Crossing it after a failed turn retires only the old process, never the Ora execution or id.
#[test]
fn failure_restores_cross_the_process_budget_without_ending_the_session() {
    super::super::with_scoped_async_test(async {
        for replacement_fails in [false, true] {
            let mut fixture = Fixture::new();
            fixture.verify_turn_boundary();
            super::super::mcp::configure(&fixture, /*load*/ true, /*http*/ true);
            let sessions = fixture.sessions(PLUGIN_VERSION);
            let mut input = super::super::mcp::input(Vec::new());
            input.payload.spec.mcp_lifecycle = true;
            input.payload.spec.keep_session_on_failure = true;
            input.payload.spec.keep_session_on_cancel = true;
            input.payload.spec.initial_turn = turn("turn-1", "[session-closed]");
            sessions.start(input.operation_id, input.execution_id, input.payload.spec);
            for ended in 1..=9 {
                until(|| (turn_ends(&fixture) == ended).then_some(())).await;
                if ended == 9 && replacement_fails {
                    std::fs::write(fixture.package_root().join("fail-initialize"), b"").unwrap();
                }
                fixture.command(
                    &sessions,
                    &format!("command-{}", ended + 1),
                    SessionCommand::SubmitUserTurn(turn(
                        &format!("turn-{}", ended + 1),
                        if ended == 9 {
                            "hello"
                        } else {
                            "[session-closed]"
                        },
                    )),
                );
            }
            if replacement_fails {
                until(|| {
                    let pids =
                        std::fs::read_to_string(fixture.package_root().join("echo-agent.pids"))
                            .unwrap_or_default();
                    (pids.lines().count() == 2).then_some(())
                })
                .await;
                fixture.command(
                    &sessions,
                    "cancel-10",
                    SessionCommand::CancelTurn(TurnId::new("turn-10")),
                );
                assert_eq!(
                    settled(&fixture, "cancel-10").await,
                    TurnCancelOutcome::NotStarted
                );
                assert_eq!(turn_ends(&fixture), 9);
            } else {
                until(|| (turn_ends(&fixture) == 10).then_some(())).await;
            }
            assert_eq!(fixture.ledger.ended(), None);
            assert_eq!(
                conversation_methods(&fixture)
                    .iter()
                    .filter(|method| method.as_str() == "session/new")
                    .count(),
                1
            );
            assert_eq!(
                std::fs::read_to_string(fixture.package_root().join("echo-agent.pids"))
                    .unwrap()
                    .lines()
                    .count(),
                2
            );
            assert_eq!(
                end(&fixture, &sessions).await,
                AgentSessionEndReason::UserEnded
            );
        }
    });
}

/// The first load after a cancelled prompt can hit D6's replacement threshold, too. A provider
/// that closed its session while answering the cancel is restored without losing the Ora session.
#[test]
fn a_cancelled_turn_can_resume_across_agent_replacement() {
    super::super::with_scoped_async_test(async {
        let mut fixture = Fixture::new();
        fixture.verify_turn_boundary();
        super::super::mcp::configure(&fixture, /*load*/ true, /*http*/ true);
        let sessions = fixture.sessions(PLUGIN_VERSION);
        let mut input = super::super::mcp::input(Vec::new());
        input.payload.spec.mcp_lifecycle = true;
        input.payload.spec.keep_session_on_failure = true;
        input.payload.spec.keep_session_on_cancel = true;
        input.payload.spec.initial_turn = turn("turn-1", "[session-closed]");
        sessions.start(input.operation_id, input.execution_id, input.payload.spec);
        for ended in 1..=8 {
            until(|| (turn_ends(&fixture) == ended).then_some(())).await;
            fixture.command(
                &sessions,
                &format!("command-{}", ended + 1),
                SessionCommand::SubmitUserTurn(turn(
                    &format!("turn-{}", ended + 1),
                    if ended == 8 {
                        "[hold-close]"
                    } else {
                        "[session-closed]"
                    },
                )),
            );
        }
        until(|| {
            (conversation_methods(&fixture)
                .iter()
                .filter(|method| method.as_str() == "session/prompt")
                .count()
                == 9)
                .then_some(())
        })
        .await;
        fixture.command(
            &sessions,
            "cancel-9",
            SessionCommand::CancelTurn(TurnId::new("turn-9")),
        );
        assert_eq!(
            settled(&fixture, "cancel-9").await,
            TurnCancelOutcome::Stopped
        );
        fixture.command(
            &sessions,
            "command-10",
            SessionCommand::SubmitUserTurn(turn("turn-10", "hello")),
        );
        until(|| (turn_ends(&fixture) == 10).then_some(())).await;
        assert_eq!(fixture.ledger.ended(), None);
        assert_eq!(
            conversation_methods(&fixture)
                .iter()
                .filter(|method| method.as_str() == "session/new")
                .count(),
            1
        );
        assert_eq!(
            std::fs::read_to_string(fixture.package_root().join("echo-agent.pids"))
                .unwrap()
                .lines()
                .count(),
            2
        );
        assert_eq!(
            end(&fixture, &sessions).await,
            AgentSessionEndReason::UserEnded
        );
    });
}
