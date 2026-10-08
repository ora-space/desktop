//! Real provider failures remain retryable even when the frozen node waits through silence.

use super::{NodeExecutionError, drive_agent_node};
use crate::agent_runtime::{AgentRuntimeSetup, open_agent_runtime};
use crate::app_event::AppEventHub;
use crate::clock::SystemClock;
use crate::plugin::PluginApi;
use crate::settings::Settings;
use crate::workflow::run::test_fixture::{
    ClockAt, SeqGen, bootstrap, recording_transitions, run_test, seeded_pending_run,
};
use ora_application::{
    ExecutionContext, NodeExecutor, NodeFailureKind, WorkflowGraph, WorkflowGraphNode,
    WorkflowRepository, WorkflowRunEngine, WorkflowRunEngineRepository, WorkflowVariablePool,
};
use ora_contracts::{EmptyErrorParams, PublicError};
use ora_db::{
    SqliteAgentDefinitionRepository, SqliteWorkflowRepository, SqliteWorkflowRunEngineRepository,
};
use ora_domain::{
    PromptInactivityPolicy, WorkflowId, WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRunStatus,
    WorkflowScopeId,
};
use ora_scheduler::Scheduler;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Records the policy the production scheduler supplies to each executor admission.
#[derive(Clone, Default)]
struct PolicyDispatches(Arc<Mutex<Vec<PromptInactivityPolicy>>>);

impl NodeExecutor for PolicyDispatches {
    fn dispatch(
        &self,
        _node_run_id: &WorkflowNodeRunId,
        node: &WorkflowGraphNode,
        _graph: &WorkflowGraph,
        _context: &ExecutionContext,
        _scope_id: &WorkflowScopeId,
        _pool: &WorkflowVariablePool,
    ) {
        self.0
            .lock()
            .unwrap()
            .push(node.agent_config.as_ref().unwrap().prompt_inactivity);
    }
}

/// The actual driver still reports an unavailable provider, and retries keep the frozen policy
/// after the editable draft changes; exhausted errors still fail the run.
///
/// Core case: specs/test-cases/desktop/core/workflow/prompt-inactivity.md#every-workflow-turn-uses-the-frozen-inactivity-policy
#[test]
fn wait_policy_preserves_real_session_failures_and_frozen_policy_across_retries() {
    run_test(async {
        let frozen = json!({"nodes":[
            {"id":"start","data":{"kind":"start"}},
            {"id":"agent","data":{"kind":"agent","agentConfig":{
                "executor":{"agentCli":"official/unavailable","modelId":"m"},
                "prompt":"do", "promptInactivity":"wait",
                "retry":{"enabled":true,"maxRetries":2,"initialDelaySeconds":0}
            }}}
        ],"edges":[{"source":"start","target":"agent"}]})
        .to_string();
        let (temp, pool) = bootstrap();
        let run_id = seeded_pending_run(&temp, &pool, &frozen);
        let mut draft: serde_json::Value = serde_json::from_str(&frozen).unwrap();
        draft["nodes"][1]["data"]["agentConfig"]["promptInactivity"] = json!("timeout");
        SqliteWorkflowRepository::new(pool.clone())
            .update_draft(&WorkflowId::new("workflow-1"), draft.to_string(), 35)
            .unwrap();
        let dispatches = PolicyDispatches::default();
        let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
        let engine = WorkflowRunEngine::new(
            repository.clone(),
            dispatches.clone(),
            SeqGen::default(),
            ClockAt(40),
        );
        engine.start(&run_id).unwrap();
        let events = AppEventHub::new();
        let plugin = Arc::new(
            PluginApi::open(
                pool.clone(),
                temp.path().to_path_buf(),
                PathBuf::from("deno"),
                SystemClock,
                events.publisher(),
                Arc::new(Settings::new(pool.clone())),
            )
            .unwrap(),
        );
        let scheduler = Scheduler::new(chrono_tz::Asia::Shanghai);
        let runtime = open_agent_runtime(AgentRuntimeSetup {
            plugin_host: plugin,
            pool: pool.clone(),
            home_directory: temp.path().to_path_buf(),
            relative_path_base: temp.path().to_path_buf(),
            sessions_root: temp.path().join("sessions"),
            scheduler: scheduler.clone(),
            app_events: events.publisher(),
        })
        .unwrap();
        let (transitions, _recording) = recording_transitions(&pool);
        let agent_repository = SqliteAgentDefinitionRepository::new(pool.clone());
        for attempt in 1..=3 {
            let node_run = repository
                .list_node_runs(&run_id)
                .unwrap()
                .into_iter()
                .find(|node| node.node_id == "agent" && node.status == WorkflowNodeStatus::Running)
                .unwrap();
            let context = repository.find_execution_context(&run_id).unwrap().unwrap();
            let graph = WorkflowGraph::parse(&context.graph_json).unwrap();
            let node = graph.node("agent").unwrap();
            let payload: ora_application::WorkflowRunPayload =
                serde_json::from_str(context.run.payload.as_deref().unwrap()).unwrap();
            let outcome = drive_agent_node(
                &runtime,
                &pool,
                &agent_repository,
                &SystemClock,
                &temp.path().join("baselines"),
                &transitions,
                &node_run.id,
                node,
                &graph,
                &context,
                &node_run.scope_id,
                &payload.variable_pool,
            )
            .await;
            let error = match outcome {
                Err(NodeExecutionError::Session(error)) => error,
                Err(error) => panic!("expected actual session failure, got {error}"),
                Ok(_) => panic!("an unavailable provider must not succeed"),
            };
            assert_eq!(
                error.public_error(),
                &PublicError::AgentRuntimeUnavailable(EmptyErrorParams {})
            );
            let failure = NodeExecutionError::Session(error).into_failure_report();
            assert_eq!(failure.kind, NodeFailureKind::Session);
            engine.fail_node(&run_id, &node_run.id, failure).unwrap();
            let context = repository.find_execution_context(&run_id).unwrap().unwrap();
            assert_eq!(
                context.run.status,
                if attempt < 3 {
                    WorkflowRunStatus::Running
                } else {
                    WorkflowRunStatus::Failed
                }
            );
            if attempt < 3 {
                let waiting = repository
                    .list_node_runs(&run_id)
                    .unwrap()
                    .into_iter()
                    .find(|node| {
                        node.node_id == "agent" && node.status == WorkflowNodeStatus::Running
                    })
                    .unwrap();
                engine.wake_retry(&run_id, &waiting.id).unwrap();
            }
        }
        assert_eq!(
            *dispatches.0.lock().unwrap(),
            vec![PromptInactivityPolicy::Wait; 3]
        );
        assert_eq!(
            repository
                .find_execution_context(&run_id)
                .unwrap()
                .unwrap()
                .graph_json,
            frozen
        );
        scheduler.shutdown().await;
    });
}
