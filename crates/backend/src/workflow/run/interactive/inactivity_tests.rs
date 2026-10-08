//! Frozen silence policy must govern every admitted human turn without rewriting stored graphs.

use super::session::{HumanTurnAdmission, begin_human_turn, end_human_turn};
use crate::workflow::run::test_fixture::{
    AGENT_GRAPH, bind_and_park, bootstrap, locks, recording_transitions, run_test, started_run,
};
use ora_application::{WorkflowRepository, WorkflowRunEngineRepository};
use ora_db::{SqliteWorkflowRepository, SqliteWorkflowRunEngineRepository};
use ora_domain::{PromptInactivityPolicy, WorkflowId, WorkflowNodeStatus};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::error::Error;

/// A changed draft must not alter an existing run's first or later human turns.
///
/// Core case: specs/test-cases/desktop/core/workflow/prompt-inactivity.md#every-workflow-turn-uses-the-frozen-inactivity-policy
#[test]
fn every_human_turn_uses_the_frozen_policy_after_a_draft_edit() {
    run_test(async {
        let mut graph: Value = serde_json::from_str(AGENT_GRAPH).unwrap();
        graph["nodes"][1]["data"]["agentConfig"]["interactive"] = json!(true);
        graph["nodes"][1]["data"]["agentConfig"]["promptInactivity"] = json!("wait");
        let frozen = graph.to_string();
        let (temp, pool) = bootstrap();
        let (run_id, node_runs) = started_run(&temp, &pool, &frozen);
        let agent = node_runs
            .iter()
            .find(|node| node.node_id == "agent")
            .unwrap();
        let (session_id, node_run_id) = bind_and_park(&pool, agent);
        graph["nodes"][1]["data"]["agentConfig"]["promptInactivity"] = json!("timeout");
        SqliteWorkflowRepository::new(pool.clone())
            .update_draft(&WorkflowId::new("workflow-1"), graph.to_string(), 60)
            .unwrap();
        let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
        let (run_locks, completing) = locks();
        let (transitions, recording) = recording_transitions(&pool);

        for _ in 0..2 {
            assert_eq!(
                begin_human_turn(
                    &pool,
                    &run_locks,
                    &completing,
                    &transitions,
                    session_id.as_ref()
                )
                .await
                .unwrap(),
                HumanTurnAdmission::Workflow {
                    node_run_id: node_run_id.clone(),
                    prompt_inactivity: PromptInactivityPolicy::Wait,
                },
            );
            end_human_turn(&transitions, &node_run_id).await.unwrap();
        }
        assert_eq!(
            repository
                .find_node_run_by_id(&node_run_id)
                .unwrap()
                .unwrap()
                .status,
            WorkflowNodeStatus::Pending
        );
        assert_eq!(
            repository
                .find_execution_context(&run_id)
                .unwrap()
                .unwrap()
                .graph_json,
            frozen
        );
        assert_eq!(
            *recording.published.lock().unwrap(),
            vec![run_id.to_string(); 4]
        );
    });
}

/// Policy decoding must fail before Pending -> Running, leaving the awaiting session usable.
#[test]
fn invalid_frozen_policy_rejects_a_human_turn_without_changing_node_state() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (_run_id, node_runs) = started_run(&temp, &pool, AGENT_GRAPH);
        let agent = node_runs
            .iter()
            .find(|node| node.node_id == "agent")
            .unwrap();
        let (session_id, node_run_id) = bind_and_park(&pool, agent);
        let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
        let before = repository
            .find_node_run_by_id(&node_run_id)
            .unwrap()
            .unwrap();
        let mut graph: Value = serde_json::from_str(AGENT_GRAPH).unwrap();
        graph["nodes"][1]["data"]["agentConfig"]["promptInactivity"] = json!(false);
        // Simulate an external/corrupt snapshot; the public update API correctly forbids edits to
        // published snapshots, so only this fixture injects the invalid stored execution contract.
        rusqlite::Connection::open(temp.path().join("repository.sqlite3"))
            .unwrap()
            .execute(
                "UPDATE workflow_snapshots SET graph = ?1 WHERE id = 'snapshot-1'",
                rusqlite::params![graph.to_string()],
            )
            .unwrap();
        let (run_locks, completing) = locks();
        let (transitions, recording) = recording_transitions(&pool);
        let error = begin_human_turn(
            &pool,
            &run_locks,
            &completing,
            &transitions,
            session_id.as_ref(),
        )
        .await
        .unwrap_err();
        assert!(
            error
                .source()
                .unwrap()
                .source()
                .unwrap()
                .to_string()
                .contains("promptInactivity")
        );
        assert_eq!(
            repository.find_node_run_by_id(&node_run_id).unwrap(),
            Some(before)
        );
        assert_eq!(*recording.published.lock().unwrap(), Vec::<String>::new());
    });
}
