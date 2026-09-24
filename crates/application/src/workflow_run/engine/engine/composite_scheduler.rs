//! Executes pure composite plans through atomic persistence operations.
use super::*;
use crate::workflow_run::engine::failure::{NodeFailure, NodeFailureKind};
use crate::workflow_run::engine::node_runtime::{CompositeAdvancePlan, CompositeContinuation};
use crate::workflow_run::engine::ports::IterationRoundContinuation;
use crate::workflow_run::engine::region::region_rows_round;

impl<R, G, C> WorkflowRunEngine<R, G, C>
where
    R: WorkflowRunEngineRepository,
    G: WorkflowNodeRunIdGenerator,
    C: Clock,
{
    /// Plans and executes one advance step for every Running composite node-run of the run.
    ///
    /// Returns whether any transition committed (so the caller reloads and re-plans). The plan
    /// is pure; execution goes through the repository's atomic composite operations, and any
    /// rows the plan started are dispatched to their async runtimes before returning.
    pub(super) fn advance_composites(
        &self,
        run_id: &WorkflowRunId,
        graph: &WorkflowGraph,
        node_runs: &[WorkflowNodeRun],
        context: &ExecutionContext,
        now: i64,
    ) -> Result<bool, EngineError> {
        let payload = execution_payload_from(context.run.payload.as_deref());
        for node_run in node_runs
            .iter()
            .filter(|node_run| node_run.status == WorkflowNodeStatus::Running)
        {
            let Some(node) = graph.node(&node_run.node_id) else {
                continue;
            };
            if matches!(
                self.runtimes.runtime(node.node_type),
                Some(RegisteredNodeRuntime::ScopedLoop)
            ) {
                if matches!(
                    self.run_loop_schedule(
                        run_id,
                        context,
                        graph,
                        node_run,
                        &payload.variable_pool,
                        now
                    )?,
                    LoopScheduleOutcome::Progressed
                ) {
                    self.run_events.publish_run_invalidated(run_id);
                    return Ok(true);
                }
                continue;
            }
            let Some(RegisteredNodeRuntime::Composite(runtime)) =
                self.runtimes.runtime(node.node_type)
            else {
                continue;
            };
            let plan = match runtime.plan_advance(node, graph, node_runs, &payload) {
                Ok(plan) => plan,
                // A composite-own failure always propagates to the run (ADR D8).
                Err(error) => {
                    self.repository.fail_node(
                        &node_run.id,
                        NodeFailure::from_runtime(node.node_type, error),
                        FailurePropagation::Run,
                        now,
                    )?;
                    self.run_events.publish_run_invalidated(run_id);
                    return Ok(true);
                }
            };
            let started = self
                .execute_composite_plan(run_id, graph, context, node_run, node_runs, plan, now)?;
            if started.is_some() {
                self.run_events.publish_run_invalidated(run_id);
                // Dispatch the rows this plan started to their background drivers.
                if let Some((_, started_runs)) = started {
                    for planned in &started_runs {
                        if let Some(started_node) = graph.node(&planned.node_id)
                            && let Some(RegisteredNodeRuntime::Async(runtime)) =
                                self.runtimes.runtime(started_node.node_type)
                        {
                            let refreshed = self.execution_context(run_id)?;
                            let (pool, _) = execution_state_from(refreshed.run.payload.as_deref());
                            runtime.dispatch(
                                &planned.id,
                                started_node,
                                &refreshed,
                                graph,
                                &refreshed.root_scope_id,
                                &pool,
                            );
                        }
                    }
                }
                // One transition per pass: reload rows and re-plan with fresh facts.
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Executes one composite advance plan through the repository's atomic operations.
    ///
    /// Returns the round and materialized rows the plan started, so the caller can dispatch
    /// their async runtimes; `None` means the plan committed no transition (a no-op).
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    fn execute_composite_plan(
        &self,
        run_id: &WorkflowRunId,
        graph: &WorkflowGraph,
        context: &ExecutionContext,
        node_run: &WorkflowNodeRun,
        node_runs: &[WorkflowNodeRun],
        plan: CompositeAdvancePlan,
        now: i64,
    ) -> Result<Option<(Option<u32>, Vec<NodeRunToStart>)>, EngineError> {
        let materialize = |round: Option<u32>, node_ids: &[String]| -> Vec<NodeRunToStart> {
            node_ids
                .iter()
                .filter_map(|node_id| {
                    let node = graph.node(node_id)?;
                    Some(NodeRunToStart {
                        id: self.node_run_id_generator.generate_node_run_id(),
                        scope_id: context.root_scope_id.clone(),
                        node_id: node.id.clone(),
                        node_type: node.node_type.as_str().to_string(),
                        input: self.runtimes.start_input(node, context),
                        iteration: round,
                    })
                })
                .collect()
        };
        match plan {
            CompositeAdvancePlan::Noop => Ok(None),
            CompositeAdvancePlan::StartRound {
                round,
                item,
                node_ids,
            } => {
                let rows = materialize(Some(round), &node_ids);
                let result = self.repository.start_iteration_round(
                    run_id,
                    &node_run.node_id,
                    round,
                    &item,
                    &rows,
                    now,
                )?;
                Ok(matches!(result, AdvanceWorkflowRunResult::Advanced)
                    .then_some((Some(round), rows)))
            }
            CompositeAdvancePlan::StartRegionNodes { node_ids } => {
                // Mid-round wave: the rows carry the round they belong to, derived from the
                // region's persisted rows so no in-memory round state exists (ADR D2).
                let round = region_rows_round(graph, node_run, node_runs);
                let rows = materialize(round, &node_ids);
                self.repository.start_ready_nodes(run_id, &rows, now)?;
                Ok(Some((round, rows)))
            }
            CompositeAdvancePlan::SettleRound {
                round,
                entry,
                continuation,
            } => {
                let continuation = match continuation {
                    CompositeContinuation::StartNextRound {
                        round,
                        item,
                        node_ids,
                    } => IterationRoundContinuation::StartNextRound {
                        round,
                        item,
                        node_runs: materialize(Some(round), &node_ids),
                    },
                    CompositeContinuation::Complete { exposed, output } => {
                        IterationRoundContinuation::Complete { exposed, output }
                    }
                    CompositeContinuation::Fail { error } => {
                        IterationRoundContinuation::Fail { error }
                    }
                };
                let started = match &continuation {
                    IterationRoundContinuation::StartNextRound {
                        round, node_runs, ..
                    } => (Some(*round), node_runs.clone()),
                    IterationRoundContinuation::Complete { .. }
                    | IterationRoundContinuation::Fail { .. } => (None, Vec::new()),
                };
                let result = self.repository.settle_iteration_round(
                    run_id,
                    &node_run.node_id,
                    round,
                    entry,
                    continuation,
                    now,
                )?;
                Ok(matches!(result, AdvanceWorkflowRunResult::Advanced).then_some(started))
            }
            CompositeAdvancePlan::CompleteNode { exposed, output } => {
                let result = self.repository.complete_iteration_node(
                    &node_run.id,
                    &node_run.node_id,
                    &exposed,
                    output,
                    now,
                )?;
                Ok(matches!(result, AdvanceWorkflowRunResult::Advanced)
                    .then_some((None, Vec::new())))
            }
            CompositeAdvancePlan::FailNode { error } => {
                let result = self.repository.fail_node(
                    &node_run.id,
                    NodeFailure::new(NodeFailureKind::InvalidRunPayload, error),
                    FailurePropagation::Run,
                    now,
                )?;
                Ok(matches!(result, AdvanceWorkflowRunResult::Advanced)
                    .then_some((None, Vec::new())))
            }
        }
    }

    /// Resolves how one node-run's failure propagates, by graph structure only (ADR
    /// "iteration composite runtime" D6): a failure inside a `continue`-strategy composite
    /// region is absorbed by the runtime; everything else keeps the `Run` behavior.
    pub(super) fn failure_propagation(
        &self,
        run_id: &WorkflowRunId,
        node_run: &WorkflowNodeRun,
    ) -> Result<FailurePropagation, EngineError> {
        if node_run.iteration.is_none() {
            return Ok(FailurePropagation::Run);
        }
        let context = self.execution_context(run_id)?;
        let graph = WorkflowGraph::parse(&context.graph_json)?;
        Ok(region_failure_propagation(&graph, &node_run.node_id))
    }
}
