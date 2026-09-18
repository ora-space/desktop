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
    let node_runs = engine.repository.list_node_runs_in_scope(&scope.id)?;
    if engine.complete_loop_controls(&scope, body, context, &state, &node_runs, now)? {
        return Ok(LoopScheduleOutcome::Progressed);
    }

    let node_runs = engine.repository.list_node_runs_in_scope(&scope.id)?;
    let projection = BranchProjection::new(body, &node_runs, &state.condition_decisions);
    let ready = projection.ready_nodes();
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
        Ok(LoopRoundDecision::Succeeded { outputs }) => LoopRoundAdvance::Succeed { outputs },
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
        _context: &ExecutionContext,
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
            let outcome = match self.runtimes.runtime(node.node_type) {
                Some(RegisteredNodeRuntime::Swift(runtime)) => Some(runtime.complete_running(
                    node,
                    &SwiftCompletion {
                        node_run,
                        run_input: None,
                        pool: &state.variable_pool,
                        node_runs,
                    },
                )),
                _ => None,
            };
            match outcome {
                Some(Ok(output)) => {
                    self.repository.complete_node(
                        &node_run.id,
                        Some(output),
                        None,
                        None,
                        Vec::new(),
                        now,
                    )?;
                    progressed = true;
                }
                Some(Err(error)) => {
                    self.repository.fail_node(
                        &node_run.id,
                        NodeFailure::from_runtime(node.node_type, error),
                        FailurePropagation::Run,
                        now,
                    )?;
                    return Ok(true);
                }
                None => {}
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
