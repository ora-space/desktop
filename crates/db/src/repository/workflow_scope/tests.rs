use crate::{
    DatabaseBootstrapper, DatabaseLocation, SqliteWorkflowRunEngineRepository,
    default_migration_catalog, test_clock::TestClock,
};
use ora_application::{
    AdvanceWorkflowRunResult, LoopRoundAdvance, LoopRoundToStart, NodeRunToStart,
    RestartWorkflowRunResult, StartWorkflowRunResult, WorkflowRunEngineRepository,
};
use ora_domain::{WorkflowNodeRunId, WorkflowRunId, WorkflowScopeId, WorkflowScopeStatus};
use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;
use serde_json::json;

/// The production restart transaction rotates roots without rewriting old node scope identities.
#[test]
fn restart_rotates_root_and_preserves_node_history() {
    with_trace_logging(|| {
        let pool = DatabaseBootstrapper::new(TestClock::new(1))
            .bootstrap_repository_pool(
                &DatabaseLocation::in_memory(),
                &default_migration_catalog().unwrap(),
            )
            .unwrap();
        pool.with_connection(|connection| {
            connection.execute_batch(r#"
                INSERT INTO projects (id, name, created_at, updated_at) VALUES ('project', 'Project', 1, 1);
                INSERT INTO workspace_locations (id, location_kind, locator_json, created_at, updated_at)
                VALUES ('location', 'local_filesystem', '{}', 1, 1);
                INSERT INTO workspaces (id, project_id, workspace_kind, location_id, created_at, updated_at)
                VALUES ('workspace', 'project', 'main', 'location', 1, 1);
                INSERT INTO workflows (id, name, created_at, updated_at) VALUES ('workflow', 'Workflow', 1, 1);
                INSERT INTO workflow_snapshots (id, workflow_id, version, graph, created_at)
                VALUES ('snapshot', 'workflow', '1', '{}', 1);
                INSERT INTO workflow_runs (id, workspace_id, workflow_id, snapshot_id, name, run_status, created_at, updated_at)
                VALUES ('run', 'workspace', 'workflow', 'snapshot', 'Run', 2, 1, 2);
                INSERT INTO workflow_node_runs (id, run_id, node_id, node_type, status, created_at, updated_at)
                VALUES ('old-start', 'run', 'start', 'start', 2, 1, 2);
            "#)?;
            Ok(())
        }).unwrap();
        let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
        let run_id = WorkflowRunId::new("run");
        assert_eq!(
            repository.restart_run(&run_id, /*now*/ 3).unwrap(),
            RestartWorkflowRunResult::Restarted
        );
        let root = pool
            .with_connection(|connection| {
                Ok(connection.query_row(
                    "SELECT scope_id FROM workflow_run_root_scopes WHERE run_id = 'run'",
                    [],
                    |row| row.get::<_, String>(0),
                )?)
            })
            .unwrap();
        assert_ne!(root, "root:run");
        let start = NodeRunToStart {
            iteration: None,
            id: WorkflowNodeRunId::new("new-start"),
            scope_id: ora_domain::WorkflowScopeId::new(root.clone()),
            node_id: "start".into(),
            node_type: "start".into(),
            input: None,
        };
        let stale = NodeRunToStart {
            iteration: None,
            id: WorkflowNodeRunId::new("stale-start"),
            scope_id: ora_domain::WorkflowScopeId::new("root:run"),
            ..start.clone()
        };
        assert!(repository.start_run(&run_id, &stale, /*now*/ 4).is_err());
        assert_eq!(repository.find_node_run_by_id(&stale.id).unwrap(), None);
        assert_eq!(
            repository.start_run(&run_id, &start, /*now*/ 4).unwrap(),
            StartWorkflowRunResult::Started
        );
        assert_eq!(
            repository
                .find_node_run_by_id(&start.id)
                .unwrap()
                .unwrap()
                .scope_id,
            start.scope_id
        );
        assert_eq!(
            repository
                .list_node_runs(&run_id)
                .unwrap()
                .iter()
                .map(|node| (&node.id, &node.scope_id))
                .collect::<Vec<_>>(),
            vec![(&start.id, &start.scope_id)]
        );
        let nodes = pool
            .with_connection(|connection| {
                Ok(connection
                    .prepare("SELECT id, scope_id, is_deleted FROM workflow_node_runs ORDER BY id")?
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?)
            })
            .unwrap();
        assert_eq!(
            nodes,
            vec![
                ("new-start".into(), root.clone(), 0),
                ("old-start".into(), "root:run".into(), 1)
            ]
        );
        assert_eq!(
            repository.restart_run(&run_id, /*now*/ 5).unwrap(),
            RestartWorkflowRunResult::NotRestartable
        );
        let unchanged = pool
            .with_connection(|connection| {
                Ok(connection.query_row(
                    "SELECT scope_id FROM workflow_run_root_scopes WHERE run_id = 'run'",
                    [],
                    |row| row.get::<_, String>(0),
                )?)
            })
            .unwrap();
        assert_eq!(unchanged, root);
    });
}

/// Round advancement keeps child output isolated and commits either continuation or export atomically.
#[test]
fn advances_loop_rounds_without_exposing_partial_child_state() {
    with_trace_logging(|| {
        let pool = DatabaseBootstrapper::new(TestClock::new(1))
            .bootstrap_repository_pool(
                &DatabaseLocation::in_memory(),
                &default_migration_catalog().unwrap(),
            )
            .unwrap();
        pool.with_connection(|connection| {
            connection.execute_batch(r#"
                INSERT INTO projects (id, name, created_at, updated_at) VALUES ('project', 'Project', 1, 1);
                INSERT INTO workspace_locations (id, location_kind, locator_json, created_at, updated_at)
                VALUES ('location', 'local_filesystem', '{}', 1, 1);
                INSERT INTO workspaces (id, project_id, workspace_kind, location_id, created_at, updated_at)
                VALUES ('workspace', 'project', 'main', 'location', 1, 1);
                INSERT INTO workflows (id, name, created_at, updated_at) VALUES ('workflow', 'Workflow', 1, 1);
                INSERT INTO workflow_snapshots (id, workflow_id, version, graph, created_at)
                VALUES ('snapshot', 'workflow', '1', '{}', 1);
                INSERT INTO workflow_runs
                    (id, workspace_id, workflow_id, snapshot_id, name, run_status, state, payload, created_at, updated_at)
                VALUES (
                    'run', 'workspace', 'workflow', 'snapshot', 'Run', 1,
                    '{"current_nodes":["loop"]}',
                    '{"locale":"en-US","skillMaterialization":{"bindings":[]},"variablePool":{"revision":0,"catalog":{"loop.result":{"valueType":"string","writer":"loop"}},"values":{}},"conditionDecisions":{}}',
                    1, 1
                );
                INSERT INTO workflow_node_runs
                    (id, run_id, node_id, node_type, status, started_at, created_at, updated_at)
                VALUES ('loop-run', 'run', 'loop', 'loop', 1, 1, 1, 1);
            "#)?;
            Ok(())
        }).unwrap();
        let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
        let run_id = WorkflowRunId::new("run");
        let parent_id = WorkflowNodeRunId::new("loop-run");
        let round_one_id = WorkflowScopeId::new("round-1");
        let round_state = r#"{"variablePool":{"revision":0,"catalog":{"child.output":{"valueType":"string","writer":"child"}},"values":{}},"conditionDecisions":{}}"#;
        let round_one = LoopRoundToStart {
            id: round_one_id.clone(),
            parent_loop_node_run_id: parent_id.clone(),
            round_index: 1,
            state: round_state.into(),
            start_node_run: NodeRunToStart {
                iteration: None,
                id: WorkflowNodeRunId::new("round-1-start"),
                scope_id: round_one_id.clone(),
                node_id: "start".into(),
                node_type: "start".into(),
                input: None,
            },
        };
        repository
            .start_loop_round(&run_id, &round_one, /*now*/ 2)
            .unwrap();
        assert_eq!(
            repository
                .advance_loop_round(
                    &round_one_id,
                    &LoopRoundAdvance::Succeed {
                        outputs: Default::default(),
                    },
                    /*now*/ 3,
                )
                .unwrap(),
            AdvanceWorkflowRunResult::NotRunning
        );
        assert_eq!(
            repository
                .complete_node(
                    &round_one.start_node_run.id,
                    None,
                    None,
                    None,
                    vec![],
                    /*now*/ 3,
                )
                .unwrap(),
            AdvanceWorkflowRunResult::Advanced
        );
        let child = NodeRunToStart {
            iteration: None,
            id: WorkflowNodeRunId::new("round-1-child"),
            scope_id: round_one_id.clone(),
            node_id: "child".into(),
            node_type: "agent".into(),
            input: None,
        };
        repository
            .start_scope_ready_nodes(&round_one_id, std::slice::from_ref(&child), /*now*/ 4)
            .unwrap();
        repository
            .complete_node(
                &child.id,
                Some("draft".into()),
                None,
                None,
                vec![],
                /*now*/ 5,
            )
            .unwrap();
        let root_values = pool.with_connection(|connection| {
            Ok(connection.query_row(
                "SELECT json_extract(payload, '$.variablePool.values') FROM workflow_runs WHERE id = 'run'",
                [],
                |row| row.get::<_, String>(0),
            )?)
        }).unwrap();
        assert_eq!(root_values, "{}");

        let round_two_id = WorkflowScopeId::new("round-2");
        let round_two = LoopRoundToStart {
            id: round_two_id.clone(),
            parent_loop_node_run_id: parent_id,
            round_index: 2,
            state: round_state.into(),
            start_node_run: NodeRunToStart {
                iteration: None,
                id: WorkflowNodeRunId::new("round-2-start"),
                scope_id: round_two_id.clone(),
                node_id: "start".into(),
                node_type: "start".into(),
                input: None,
            },
        };
        let invalid_round_two = LoopRoundToStart {
            start_node_run: NodeRunToStart {
                iteration: None,
                scope_id: WorkflowScopeId::new("wrong-scope"),
                ..round_two.start_node_run.clone()
            },
            ..round_two.clone()
        };
        assert!(
            repository
                .advance_loop_round(
                    &round_one_id,
                    &LoopRoundAdvance::Continue {
                        next: invalid_round_two,
                    },
                    /*now*/ 6,
                )
                .is_err()
        );
        assert_eq!(
            repository
                .find_active_loop_round(&round_two.parent_loop_node_run_id)
                .unwrap()
                .unwrap()
                .id,
            round_one_id
        );
        assert_eq!(
            repository
                .advance_loop_round(
                    &round_one_id,
                    &LoopRoundAdvance::Continue {
                        next: round_two.clone(),
                    },
                    /*now*/ 6,
                )
                .unwrap(),
            AdvanceWorkflowRunResult::Advanced
        );
        assert_eq!(
            repository
                .find_active_loop_round(&round_two.parent_loop_node_run_id)
                .unwrap(),
            Some(ora_domain::WorkflowExecutionScope {
                id: round_two_id.clone(),
                run_id: run_id.clone(),
                parent_loop_node_run_id: round_two.parent_loop_node_run_id.clone(),
                round_index: 2,
                status: WorkflowScopeStatus::Running,
                state: round_state.into(),
                created_at: 6,
                updated_at: 6,
            })
        );
        repository
            .complete_node(
                &round_two.start_node_run.id,
                None,
                None,
                None,
                vec![],
                /*now*/ 7,
            )
            .unwrap();
        let succeed = LoopRoundAdvance::Succeed {
            outputs: [("result".into(), json!("final"))].into(),
        };
        assert_eq!(
            repository
                .advance_loop_round(&round_two_id, &succeed, /*now*/ 8)
                .unwrap(),
            AdvanceWorkflowRunResult::Advanced
        );
        assert_eq!(
            repository
                .advance_loop_round(&round_two_id, &succeed, /*now*/ 9)
                .unwrap(),
            AdvanceWorkflowRunResult::NotRunning
        );
        let final_state = pool
            .with_connection(|connection| {
                Ok(connection.query_row(
                    "SELECT json_extract(payload, '$.variablePool.values.\"loop.result\"'), state
                 FROM workflow_runs WHERE id = 'run'",
                    [],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )?)
            })
            .unwrap();
        assert_eq!(
            final_state,
            ("final".into(), r#"{"current_nodes":[]}"#.into())
        );
    });
}

/// A failed child settles its siblings, round, parent Loop, and root run in one transaction.
#[test]
fn child_failure_settles_the_loop_scope() {
    with_trace_logging(|| {
        let pool = DatabaseBootstrapper::new(TestClock::new(1))
            .bootstrap_repository_pool(
                &DatabaseLocation::in_memory(),
                &default_migration_catalog().unwrap(),
            )
            .unwrap();
        pool.with_connection(|connection| {
            connection.execute_batch(r#"
                INSERT INTO projects (id, name, created_at, updated_at) VALUES ('project', 'Project', 1, 1);
                INSERT INTO workspace_locations (id, location_kind, locator_json, created_at, updated_at)
                VALUES ('location', 'local_filesystem', '{}', 1, 1);
                INSERT INTO workspaces (id, project_id, workspace_kind, location_id, created_at, updated_at)
                VALUES ('workspace', 'project', 'main', 'location', 1, 1);
                INSERT INTO workflows (id, name, created_at, updated_at) VALUES ('workflow', 'Workflow', 1, 1);
                INSERT INTO workflow_snapshots (id, workflow_id, version, graph, created_at)
                VALUES ('snapshot', 'workflow', '1', '{}', 1);
                INSERT INTO workflow_runs
                    (id, workspace_id, workflow_id, snapshot_id, name, run_status, state, created_at, updated_at)
                VALUES ('run', 'workspace', 'workflow', 'snapshot', 'Run', 1,
                    '{"current_nodes":["loop"]}', 1, 1);
                INSERT INTO workflow_node_runs
                    (id, run_id, node_id, node_type, status, started_at, created_at, updated_at)
                VALUES ('loop-run', 'run', 'loop', 'loop', 1, 1, 1, 1);
            "#)?;
            Ok(())
        }).unwrap();
        let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
        let scope_id = WorkflowScopeId::new("round");
        let round = LoopRoundToStart {
            id: scope_id.clone(),
            parent_loop_node_run_id: WorkflowNodeRunId::new("loop-run"),
            round_index: 1,
            state: r#"{"variablePool":{"revision":0,"catalog":{},"values":{}},"conditionDecisions":{}}"#.into(),
            start_node_run: NodeRunToStart {
            iteration: None,
                id: WorkflowNodeRunId::new("round-start"),
                scope_id: scope_id.clone(),
                node_id: "start".into(),
                node_type: "start".into(),
                input: None,
            },
        };
        repository
            .start_loop_round(&WorkflowRunId::new("run"), &round, /*now*/ 2)
            .unwrap();
        repository
            .complete_node(
                &round.start_node_run.id,
                None,
                None,
                None,
                vec![],
                /*now*/ 3,
            )
            .unwrap();
        let failed = NodeRunToStart {
            iteration: None,
            id: WorkflowNodeRunId::new("failed-child"),
            scope_id: scope_id.clone(),
            node_id: "failed".into(),
            node_type: "agent".into(),
            input: None,
        };
        let sibling = NodeRunToStart {
            iteration: None,
            id: WorkflowNodeRunId::new("sibling-child"),
            scope_id: scope_id.clone(),
            node_id: "sibling".into(),
            node_type: "agent".into(),
            input: None,
        };
        repository
            .start_scope_ready_nodes(
                &scope_id,
                &[failed.clone(), sibling.clone()],
                /*now*/ 4,
            )
            .unwrap();

        assert_eq!(
            repository
                .fail_node(
                    &failed.id,
                    ora_application::NodeFailure::new(
                        ora_application::NodeFailureKind::Session,
                        "boom"
                    ),
                    ora_application::FailurePropagation::Run,
                    /*now*/ 5
                )
                .unwrap(),
            AdvanceWorkflowRunResult::Advanced
        );
        let settled = pool
            .with_connection(|connection| {
                Ok((
                    connection.query_row(
                        "SELECT status FROM workflow_execution_scopes WHERE id = 'round'",
                        [],
                        |row| row.get::<_, i64>(0),
                    )?,
                    connection.query_row(
                        "SELECT status FROM workflow_node_runs WHERE id = 'sibling-child'",
                        [],
                        |row| row.get::<_, i64>(0),
                    )?,
                    connection.query_row(
                        "SELECT status FROM workflow_node_runs WHERE id = 'loop-run'",
                        [],
                        |row| row.get::<_, i64>(0),
                    )?,
                    connection.query_row(
                        "SELECT run_status, state FROM workflow_runs WHERE id = 'run'",
                        [],
                        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
                    )?,
                ))
            })
            .unwrap();
        assert_eq!(
            settled,
            (
                WorkflowScopeStatus::Failed.database_value(),
                ora_domain::WorkflowNodeStatus::Cancelled.database_value(),
                ora_domain::WorkflowNodeStatus::Failed.database_value(),
                (
                    ora_domain::WorkflowRunStatus::Failed.database_value(),
                    r#"{"current_nodes":["loop"]}"#.into()
                ),
            )
        );
    });
}
