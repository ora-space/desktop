//! Real persistence and dependency preparation verify spare nodes never acquire runtime state.

use super::prerequisites::SkillRoleWorkspaceInitializer;
use super::test_fixture::{ClockAt, RecordingExecutor, bootstrap, run_test, seeded_pending_run};
use ora_application::{
    SkillMaterializationReceipt, UuidWorkflowNodeRunIdGenerator, WorkflowGraph, WorkflowRunEngine,
    WorkflowRunEngineRepository, WorkflowRunWorkspaceInitializer,
};
use ora_db::SqliteWorkflowRunEngineRepository;
use ora_domain::WorkflowRunStatus;
use pretty_assertions::assert_eq;
use serde_json::json;

/// Missing spare dependencies cannot block preparation, create sessions, or delay completion.
#[test]
fn unused_agents_are_excluded_from_preparation_execution_and_restart() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let source = json!({"nodes":[
            {"id":"start","data":{"kind":"start"}},
            {"id":"out","data":{"kind":"output"}},
            {"id":"spare","data":{"kind":"agent","agentConfig":{
                "roleId":"missing-role","skills":[{"skillId":"missing-skill","enabled":true}],
                "executor":{"agentCli":"missing-plugin","modelId":"missing"},"interactive":true
            }}}
        ],"edges":[{"source":"start","target":"out"},{"source":"spare","target":"out"}]})
        .to_string();
        let graph = WorkflowGraph::parse(&source).unwrap();
        let definitions =
            crate::workflow::WorkflowApi::new(pool.clone(), crate::clock::SystemClock);
        assert_eq!(
            definitions
                .analyze(ora_contracts::AnalyzeWorkflowRequest {
                    graph: source.clone()
                })
                .unwrap(),
            ora_contracts::AnalyzeWorkflowResponse {
                unused_node_ids: vec!["spare".into()],
                unrecognized_nodes: Vec::new(),
            }
        );
        let created = definitions
            .create(ora_contracts::CreateWorkflowRequest {
                name: "Spare graph".into(),
                graph: Some(source.clone()),
            })
            .unwrap();
        let published = definitions
            .publish(ora_contracts::PublishWorkflowRequest {
                workflow_id: created.workflow.id.clone(),
                version: Some("v1".into()),
            })
            .unwrap();
        assert_eq!(published.snapshot.graph, source);
        let initializer =
            SkillRoleWorkspaceInitializer::new(temp.path().join("skills"), pool.clone()).unwrap();
        assert_eq!(
            initializer
                .initialize_workspace(&graph, temp.path())
                .unwrap(),
            SkillMaterializationReceipt::default()
        );
        let run_id = seeded_pending_run(&temp, &pool, &source);
        let repository = SqliteWorkflowRunEngineRepository::new(pool);
        let executor = RecordingExecutor::default();
        let engine = WorkflowRunEngine::new(
            repository.clone(),
            executor.clone(),
            UuidWorkflowNodeRunIdGenerator,
            ClockAt(40),
        );
        engine.start(&run_id).unwrap();
        engine.restart(&run_id).unwrap();
        let context = repository.find_execution_context(&run_id).unwrap().unwrap();
        assert_eq!(context.run.status, WorkflowRunStatus::Succeeded);
        assert_eq!(context.graph_json, source);
        assert_eq!(executor.records(), vec![]);
        let mut nodes: Vec<_> = repository
            .list_node_runs(&run_id)
            .unwrap()
            .into_iter()
            .map(|node| (node.node_id, node.session_id))
            .collect();
        nodes.sort();
        assert_eq!(nodes, vec![("out".into(), None), ("start".into(), None)]);
    });
}

/// Recovery scheduling, cancellation, and late callbacks cannot activate spare nodes.
#[test]
fn unused_nodes_stay_excluded_across_cancel_resume_and_late_completion() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let source = json!({"nodes":[
            {"id":"start","data":{"kind":"start"}},
            {"id":"active","data":{"kind":"agent"}},
            {"id":"spare","data":{"kind":"agent"}}
        ],"edges":[{"source":"start","target":"active"}]})
        .to_string();
        let run_id = seeded_pending_run(&temp, &pool, &source);
        let repository = SqliteWorkflowRunEngineRepository::new(pool);
        let executor = RecordingExecutor::default();
        let engine = WorkflowRunEngine::new(
            repository.clone(),
            executor.clone(),
            UuidWorkflowNodeRunIdGenerator,
            ClockAt(40),
        );
        engine.start(&run_id).unwrap();
        engine.resume(&run_id).unwrap();
        let old_node_id =
            ora_domain::WorkflowNodeRunId::new(executor.records()[0].node_run_id.clone());
        engine.cancel(&run_id).unwrap();
        engine.restart(&run_id).unwrap();
        engine
            .complete_node(
                &run_id,
                &old_node_id,
                Some("late".into()),
                /*structured_output*/ None,
                /*stop_reason*/ None,
                vec![],
            )
            .unwrap();
        engine.resume(&run_id).unwrap();
        assert_eq!(
            executor
                .records()
                .into_iter()
                .map(|record| record.node_id)
                .collect::<Vec<_>>(),
            vec!["active", "active"]
        );
        let context = repository.find_execution_context(&run_id).unwrap().unwrap();
        assert_eq!(
            (context.run.status, context.graph_json),
            (WorkflowRunStatus::Running, source)
        );
        assert!(
            repository
                .list_node_runs(&run_id)
                .unwrap()
                .iter()
                .all(|node| node.node_id != "spare")
        );
    });
}
