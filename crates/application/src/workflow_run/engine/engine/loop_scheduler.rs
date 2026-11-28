use super::{EngineError, LoopScheduleOutcome, WorkflowRunEngine};
use crate::project::Clock;
use crate::workflow_run::engine::branch_projection::BranchProjection;
use crate::workflow_run::engine::engine::WorkflowValidationError;
use crate::workflow_run::engine::failure::{NodeFailure, NodeFailureKind};
use crate::workflow_run::engine::graph::WorkflowGraph;
use crate::workflow_run::engine::node_runtime::{RegisteredNodeRuntime, SwiftCompletion};
use crate::workflow_run::engine::ports::{
    ExecutionContext, FailurePropagation, LoopRoundAdvance, LoopRoundToStart, NodeRunToStart,
    WorkflowNodeRunIdGenerator, WorkflowRunEngineRepository,
};
use crate::workflow_run::engine::variable_pool::WorkflowVariablePool;
use crate::workflow_run::engine::{LoopRoundDecision, LoopRoundExecutionState};
use ora_domain::{WorkflowExecutionScope, WorkflowNodeRun, WorkflowNodeStatus, WorkflowRunId};

/// Runs one isolated scheduling pass for a Loop parent and its active round.
pub(super) fn run_loop_schedule<R, G, C>(
    engine: &WorkflowRunEngine<R, G, C>,
    run_id: &WorkflowRunId,
    context: &ExecutionContext,
    graph: &WorkflowGraph,
    loop_node_run: &WorkflowNodeRun,
    outer_pool: &WorkflowVariablePool,
    now: i64,
) -> Result<LoopScheduleOutcome, EngineError>
where
    R: WorkflowRunEngineRepository,
    G: WorkflowNodeRunIdGenerator,
    C: Clock,
{
    // The dispatch fence already protects affected attempts and their container.
    // Also check the blocked node here so a scheduling pass from a sibling callback
    // cannot evaluate old feedback or an exit while the edit is being applied.
    if engine
        .repository
        .debug_dispatch_fence(run_id)?
        .is_some_and(|fence| fence.blocked_nodes.contains(&loop_node_run.node_id))
    {
        return Ok(LoopScheduleOutcome::Waiting);
    }
    let Some((config, body)) = graph.loop_body(&loop_node_run.node_id) else {
        engine.repository.fail_node(
            &loop_node_run.id,
            NodeFailure::new(
                NodeFailureKind::InvalidRunPayload,
                format!("Loop {} has no executable body", loop_node_run.node_id),
            ),
            FailurePropagation::Run,
            now,
        )?;
        return Ok(LoopScheduleOutcome::Progressed);
    };
    let Some(scope) = engine
        .repository
        .find_active_loop_round(&loop_node_run.id)?
    else {
        let carried = match config.initialize_carried(outer_pool) {
            Ok(carried) => carried,
            Err(error) => {
                engine.repository.fail_node(
                    &loop_node_run.id,
                    NodeFailure::new(NodeFailureKind::InvalidRunPayload, error.to_string()),
                    FailurePropagation::Run,
                    now,
                )?;
                return Ok(LoopScheduleOutcome::Progressed);
            }
        };
        let round_pool = match graph.loop_round_pool(&loop_node_run.node_id, outer_pool, &carried) {
            Ok(pool) => pool,
            Err(error) => {
                engine.repository.fail_node(
                    &loop_node_run.id,
                    NodeFailure::new(NodeFailureKind::InvalidRunPayload, error.to_string()),
                    FailurePropagation::Run,
                    now,
                )?;
                return Ok(LoopScheduleOutcome::Progressed);
            }
        };
        let round = engine.prepare_loop_round(loop_node_run, body, 1, round_pool)?;
        engine.repository.start_loop_round(run_id, &round, now)?;
        return Ok(LoopScheduleOutcome::Progressed);
    };

    let state = match serde_json::from_str::<LoopRoundExecutionState>(&scope.state) {
        Ok(state) => state,
        Err(error) => {
            engine.repository.advance_loop_round(
                &scope.id,
                &LoopRoundAdvance::Fail {
                    error: format!("Loop round state is invalid: {error}"),
                },
                now,
            )?;
            return Ok(LoopScheduleOutcome::Progressed);
        }
    };
    if state.exit.is_some() {
        return engine.drive_loop_exit_cleanup(context, loop_node_run, &scope);
    }
    let node_runs = engine.repository.list_node_runs_in_scope(&scope.id)?;
    if let Some(exit_node) = node_runs.iter().find(|node| {
        node.status == WorkflowNodeStatus::Running
            && body.node(&node.node_id).is_some_and(|node| {
                matches!(engine.runtimes.runtime(node.node_type),
                        Some(RegisteredNodeRuntime::Swift(runtime)) if runtime.requests_loop_exit())
            })
    }) {
        let result = config
            .exit_outputs(&state.variable_pool)
            .map_err(|error| error.to_string());
        engine.repository.advance_loop_round(
            &scope.id,
            &LoopRoundAdvance::RequestExit {
                node_run_id: exit_node.id.clone(),
                result,
            },
            now,
        )?;
        return Ok(LoopScheduleOutcome::Progressed);
    }
    if engine.complete_loop_controls(&scope, body, context, &state, &node_runs, now)? {
        return Ok(LoopScheduleOutcome::Progressed);
    }

    let node_runs = engine.repository.list_node_runs_in_scope(&scope.id)?;
    let projection = BranchProjection::new(body, &node_runs, &state.condition_decisions);
    let mut ready = projection.ready_nodes();
    // Do not dispatch sibling work in the same wave when a break is already reachable.
    if let Some(exit_node) = ready
        .iter()
        .find(|node| {
            matches!(engine.runtimes.runtime(node.node_type),
                Some(RegisteredNodeRuntime::Swift(runtime)) if runtime.requests_loop_exit())
        })
        .copied()
    {
        ready = vec![exit_node];
    }
    if !ready.is_empty() {
        let ready_runs: Vec<NodeRunToStart> = ready
            .iter()
            .map(|node| NodeRunToStart {
                id: engine.node_run_id_generator.generate_node_run_id(),
                scope_id: scope.id.clone(),
                node_id: node.id.clone(),
                node_type: node.node_type.as_str().to_string(),
                input: None,
                iteration: None,
            })
            .collect();
        engine
            .repository
            .start_scope_ready_nodes(&scope.id, &ready_runs, now)?;
        for (node, node_run) in ready.iter().zip(&ready_runs) {
            if let Some(RegisteredNodeRuntime::Async(runtime)) =
                engine.runtimes.runtime(node.node_type)
            {
                let revised = engine.repository.revised_graph(run_id, &node_run.id)?;
                let body = revised
                    .as_ref()
                    .and_then(|g| g.loop_body(&loop_node_run.node_id))
                    .map_or(body, |(_, b)| b);
                let node = body.node(&node.id).unwrap_or(node);
                runtime.dispatch(
                    &node_run.id,
                    node,
                    context,
                    body,
                    &scope.id,
                    &state.variable_pool,
                );
            }
        }
        return Ok(LoopScheduleOutcome::Progressed);
    }
    if projection.has_in_flight() {
        return Ok(LoopScheduleOutcome::Waiting);
    }

    let advance = match config.complete_round(scope.round_index, &state.variable_pool) {
        Ok(LoopRoundDecision::Continue { carried }) => {
            let next_pool =
                match graph.loop_round_pool(&loop_node_run.node_id, outer_pool, &carried) {
                    Ok(pool) => pool,
                    Err(error) => {
                        return engine.fail_loop_round(&scope, error.to_string(), now);
                    }
                };
            LoopRoundAdvance::Continue {
                next: engine.prepare_loop_round(
                    loop_node_run,
                    body,
                    scope.round_index + 1,
                    next_pool,
                )?,
            }
        }
        Ok(LoopRoundDecision::Succeeded { outputs, reason }) => {
            LoopRoundAdvance::Succeed { outputs, reason }
        }
        Err(error) => return engine.fail_loop_round(&scope, error.to_string(), now),
    };
    engine
        .repository
        .advance_loop_round(&scope.id, &advance, now)?;
    Ok(LoopScheduleOutcome::Progressed)
}

impl<R, G, C> WorkflowRunEngine<R, G, C>
where
    R: WorkflowRunEngineRepository,
    G: WorkflowNodeRunIdGenerator,
    C: Clock,
{
    /// Runs one isolated scheduling pass for a Loop parent and its active round.
    pub(super) fn run_loop_schedule(
        &self,
        run_id: &WorkflowRunId,
        context: &ExecutionContext,
        graph: &WorkflowGraph,
        loop_node_run: &WorkflowNodeRun,
        outer_pool: &WorkflowVariablePool,
        now: i64,
    ) -> Result<LoopScheduleOutcome, EngineError> {
        run_loop_schedule(self, run_id, context, graph, loop_node_run, outer_pool, now)
    }

    /// Constructs the durable round state and child Start node for one new iteration.
    fn prepare_loop_round(
        &self,
        loop_node_run: &WorkflowNodeRun,
        body: &WorkflowGraph,
        round_index: u32,
        variable_pool: WorkflowVariablePool,
    ) -> Result<LoopRoundToStart, EngineError> {
        let start = body
            .start_node()
            .ok_or(WorkflowValidationError::MissingStartNode)?;
        let scope_id = self.node_run_id_generator.generate_scope_id();
        let state = serde_json::to_string(&LoopRoundExecutionState {
            variable_pool,
            condition_decisions: Default::default(),
            exit: None,
        })
        .map_err(|error| EngineError::LoopState {
            message: error.to_string(),
        })?;
        Ok(LoopRoundToStart {
            id: scope_id.clone(),
            parent_loop_node_run_id: loop_node_run.id.clone(),
            round_index,
            state,
            start_node_run: NodeRunToStart {
                id: self.node_run_id_generator.generate_node_run_id(),
                scope_id,
                node_id: start.id.clone(),
                node_type: start.node_type.as_str().to_string(),
                input: None,
                iteration: None,
            },
        })
    }

    /// Completes child control nodes against the current round pool before branch projection.
    fn complete_loop_controls(
        &self,
        _scope: &WorkflowExecutionScope,
        body: &WorkflowGraph,
        context: &ExecutionContext,
        state: &LoopRoundExecutionState,
        node_runs: &[WorkflowNodeRun],
        now: i64,
    ) -> Result<bool, EngineError> {
        let mut progressed = false;
        for node_run in node_runs
            .iter()
            .filter(|node_run| node_run.status == WorkflowNodeStatus::Running)
        {
            let Some(node) = body.node(&node_run.node_id) else {
                continue;
            };
            let Some(RegisteredNodeRuntime::Swift(runtime)) = self.runtimes.runtime(node.node_type)
            else {
                continue;
            };
            let revised = self
                .repository
                .revised_graph(&context.run.id, &node_run.id)?;
            let node = revised
                .as_ref()
                .and_then(|g| g.execution_node(&node.id))
                .unwrap_or(node);
            let completion = SwiftCompletion {
                node_run,
                run_input: None,
                pool: &state.variable_pool,
                node_runs,
            };
            match runtime.complete_running(node, &completion) {
                Ok(output) => {
                    let evidence = runtime.completion_evidence(&state.variable_pool, &output);
                    self.repository.complete_node(
                        &node_run.id,
                        Some(output),
                        None,
                        None,
                        Vec::new(),
                        evidence.as_ref(),
                        now,
                    )?;
                    progressed = true;
                }
                Err(error) => {
                    self.repository.fail_node(
                        &node_run.id,
                        NodeFailure::from_runtime(node.node_type, error),
                        FailurePropagation::Run,
                        now,
                    )?;
                    return Ok(true);
                }
            }
        }
        Ok(progressed)
    }

    /// Persists a drained round decision failure without leaving its parent Loop active.
    fn fail_loop_round(
        &self,
        scope: &WorkflowExecutionScope,
        error: String,
        now: i64,
    ) -> Result<LoopScheduleOutcome, EngineError> {
        self.repository
            .advance_loop_round(&scope.id, &LoopRoundAdvance::Fail { error }, now)?;
        Ok(LoopScheduleOutcome::Progressed)
    }
}
