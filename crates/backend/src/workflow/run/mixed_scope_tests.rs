//! Loop scopes and foreach rounds must coexist through the production scheduler and repository.

use super::test_fixture::{ClockAt, bootstrap, run_test, seeded_pending_run};
use ora_application::{
    ExecutionContext, NodeExecutor, UuidWorkflowNodeRunIdGenerator, WorkflowGraph,
    WorkflowGraphNode, WorkflowRunEngine, WorkflowRunEngineRepository, WorkflowVariablePool,
};
use ora_db::SqliteWorkflowRunEngineRepository;
use ora_domain::{
    PromptInactivityPolicy, WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRunStatus,
    WorkflowScopeId,
};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

/// Records the committed pool seen by actual async dispatches, not merely persisted rows.
#[derive(Clone, Default)]
struct RecordingExecutor(Arc<Mutex<Vec<(String, Value, PromptInactivityPolicy)>>>);

impl NodeExecutor for RecordingExecutor {
    fn dispatch(
        &self,
        _node_run_id: &WorkflowNodeRunId,
        node: &WorkflowGraphNode,
        _graph: &WorkflowGraph,
        _context: &ExecutionContext,
        _scope_id: &WorkflowScopeId,
        pool: &WorkflowVariablePool,
    ) {
        let selector = if node.id == "writer" {
            "loop.draft"
        } else {
            "iter.item"
        };
        self.0.lock().unwrap().push((
            node.id.clone(),
            pool.values[selector].clone(),
            node.agent_config.as_ref().unwrap().prompt_inactivity,
        ));
    }
}

/// Separate composites retain their own variable pools and inactivity policies on every round.
///
/// Core case: specs/test-cases/desktop/core/workflow/prompt-inactivity.md#every-workflow-turn-uses-the-frozen-inactivity-policy
#[test]
fn loop_and_iteration_dispatch_each_round_with_the_committed_pool() {
    run_test(async {
        let graph = json!({
            "schemaVersion": 2,
            "nodes": [
                {"id":"start","data":{"kind":"start","inputVariables":[
                    {"name":"items","valueType":"array[string]","value":["one","two"]}
                ]}},
                {"id":"loop","data":{"kind":"loop","loopConfig":{
                    "maxIterations":3,
                    "variables":[{"name":"draft","valueType":"string","initial":{"kind":"constant","value":"seed"},"feedback":["writer","output"]}],
                    "until":{"logic":"and","conditions":[{"variableSelector":["writer","output"],"operator":"equals","value":"done"}]},
                    "outputs":[{"name":"result","variableSelector":["writer","output"]}]
                }}},
                {"id":"entry","parentId":"loop","data":{"kind":"start","containerId":"loop"}},
                {"id":"writer","parentId":"loop","data":{"kind":"agent","containerId":"loop","agentConfig":{
                    "executor":{"agentCli":"c","modelId":"m"},"prompt":"revise", "promptInactivity":"wait"
                }}},
                {"id":"iter","data":{"kind":"iteration","iterationConfig":{
                    "iteratorSelector":["start","items"],"collectSelector":["fix","output"],"errorStrategy":"fail","maxIterations":3
                }}},
                {"id":"fix","parentId":"iter","data":{"kind":"agent","agentConfig":{
                    "executor":{"agentCli":"c","modelId":"m"},"prompt":"process"
                }}},
                {"id":"out","data":{"kind":"output","outputs":[
                    {"name":"draft","variableSelector":["loop","result"]},
                    {"name":"items","variableSelector":["iter","output"]}
                ]}}
            ],
            "edges":[{"source":"start","target":"loop"},{"source":"entry","target":"writer"},
                {"source":"loop","target":"iter"},{"source":"iter","target":"fix"},{"source":"iter","target":"out"}]
        }).to_string();
        let (temp, pool) = bootstrap();
        let run_id = seeded_pending_run(&temp, &pool, &graph);
        let repository = SqliteWorkflowRunEngineRepository::new(pool);
        let executor = RecordingExecutor::default();
        let engine = WorkflowRunEngine::new(
            repository.clone(),
            executor.clone(),
            UuidWorkflowNodeRunIdGenerator,
            ClockAt(40),
        );
        engine.start(&run_id).unwrap();
        for output in ["again", "done", "first", "second"] {
            let node = repository
                .list_node_runs(&run_id)
                .unwrap()
                .into_iter()
                .find(|node| {
                    node.node_type == "agent" && node.status == WorkflowNodeStatus::Running
                })
                .unwrap();
            engine
                .complete_node(&run_id, &node.id, Some(output.into()), None, None, vec![])
                .unwrap();
        }
        assert_eq!(
            *executor.0.lock().unwrap(),
            vec![
                ("writer".into(), json!("seed"), PromptInactivityPolicy::Wait),
                (
                    "writer".into(),
                    json!("again"),
                    PromptInactivityPolicy::Wait
                ),
                ("fix".into(), json!("one"), PromptInactivityPolicy::Timeout),
                ("fix".into(), json!("two"), PromptInactivityPolicy::Timeout),
            ]
        );
        let run = repository
            .find_execution_context(&run_id)
            .unwrap()
            .unwrap()
            .run;
        assert_eq!(
            (run.status, run.output),
            (
                WorkflowRunStatus::Succeeded,
                Some(r#"{"draft":"done","items":["first","second"]}"#.into())
            )
        );
        let rows = repository.list_node_runs(&run_id).unwrap();
        let writers: Vec<_> = rows
            .iter()
            .filter(|node| node.node_id == "writer")
            .collect();
        assert_ne!(writers[0].scope_id, writers[1].scope_id);
        let rounds: Vec<_> = rows
            .iter()
            .filter(|node| node.node_id == "fix")
            .map(|node| node.iteration)
            .collect();
        assert_eq!(
            rounds
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>(),
            [Some(0), Some(1)].into_iter().collect()
        );
    });
}
