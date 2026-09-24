//! Automatic retry of failed agent attempts: scheduling the waiting attempt and starting it when
//! its backoff elapses.
//!
//! The decision is made here, in the engine, so it runs under the same per-run serial gate and
//! injected clock as every other transition; the backend only supplies the timer that calls
//! [`WorkflowRunEngine::wake_retry`].
use super::*;
use crate::workflow_run::engine::LoopRoundExecutionState;
use crate::workflow_run::engine::failure::NodeFailureKind;
use crate::workflow_run::engine::retry::{
    BeginNodeRetryResult, NodeAutoRetry, NodeRetryToSchedule, ScheduleNodeRetryResult,
};

impl<R, G, C> WorkflowRunEngine<R, G, C> {
    /// Attaches the timer that wakes waiting attempts. Engines built without one schedule
    /// retries but never start them, which suits tests that drive `wake_retry` by hand.
    pub fn with_retry_timer(self, retry_timer: Arc<dyn WorkflowRetryTimer>) -> Self {
        Self {
            retry_timer,
            ..self
        }
    }
}

impl<R, G, C> WorkflowRunEngine<R, G, C>
where
    R: WorkflowRunEngineRepository,
    G: WorkflowNodeRunIdGenerator,
    C: Clock,
{
    /// Replaces the failed attempt `node_run` by a waiting attempt when its node's retry policy
    /// covers `failure`, and arms the timer that starts it.
    ///
    /// Returns `true` when the failure is consumed (a retry was scheduled, or the callback was
    /// stale) and `false` when the normal failure path must run: the node has no policy, the
    /// kind does not retry, the budget is spent, or the run already left `Running`.
    ///
    /// An error while deciding also returns `false`. Nothing was persisted yet, and the node's
    /// session has already ended, so failing the node is the only way to keep the run from
    /// waiting on a `Running` row that nothing drives.
    pub(super) fn schedule_retry(
        &self,
        run_id: &WorkflowRunId,
        node_run: &WorkflowNodeRun,
        failure: &NodeFailure,
        now: i64,
    ) -> bool {
        self.try_schedule_retry(run_id, node_run, failure, now)
            .unwrap_or_else(|error| {
                ora_logging::ora_warn!(
                    error = %error,
                    node_run_id = %node_run.id,
                    "could not decide an automatic retry; failing the attempt instead"
                );
                false
            })
    }

    fn try_schedule_retry(
        &self,
        run_id: &WorkflowRunId,
        node_run: &WorkflowNodeRun,
        failure: &NodeFailure,
        now: i64,
    ) -> Result<bool, EngineError> {
        if node_run.status != WorkflowNodeStatus::Running {
            return Ok(false);
        }
        let context = self.execution_context(run_id)?;
        if context.run.status != WorkflowRunStatus::Running {
            return Ok(false);
        }
        let graph = WorkflowGraph::parse(&context.graph_json)?;
        let Some(node) = graph.execution_node(&node_run.node_id) else {
            return Ok(false);
        };
        let Some(RegisteredNodeRuntime::Async(runtime)) = self.runtimes.runtime(node.node_type)
        else {
            return Ok(false);
        };
        let Some(policy) = runtime.retry_policy(node) else {
            return Ok(false);
        };
        // Only rows an automatic retry started carry the marker, so a first attempt, a manual
        // resume, and a restart all start a fresh budget.
        let retries_used = NodeAutoRetry::from_payload(node_run.payload.as_deref())
            .map_or(0, |marker| marker.retry);
        let Some(retry) = policy.next_retry(failure.kind, retries_used) else {
            return Ok(false);
        };
        let delay_ms =
            i64::try_from(policy.wait_seconds(retry).saturating_mul(1000)).unwrap_or(i64::MAX);
        let waiting = NodeRetryToSchedule {
            node_run_id: self.node_run_id_generator.generate_node_run_id(),
            retry,
            max_retries: policy.max_retries,
            delay_ms,
            due_at: now.saturating_add(delay_ms),
        };
        match self
            .repository
            .schedule_node_retry(&node_run.id, failure, &waiting, now)?
        {
            ScheduleNodeRetryResult::Scheduled => {
                self.run_events.publish_run_invalidated(run_id);
                self.retry_timer
                    .arm(run_id, &waiting.node_run_id, waiting.due_at);
                Ok(true)
            }
            ScheduleNodeRetryResult::NotRunning | ScheduleNodeRetryResult::NotFound => Ok(true),
            ScheduleNodeRetryResult::RunNotActive => Ok(false),
        }
    }

    /// Starts one waiting attempt whose backoff elapsed and dispatches it to its async runtime.
    ///
    /// Called by the retry timer under the run's serial gate. A wake for an attempt that is no
    /// longer waiting — cancelled, failed with its run, interrupted by a restart, or already
    /// started by an earlier wake — is a no-op, so timers never need to be disarmed. An early
    /// wake re-arms the timer for the persisted deadline.
    pub fn wake_retry(
        &self,
        run_id: &WorkflowRunId,
        node_run_id: &WorkflowNodeRunId,
    ) -> Result<(), EngineError> {
        let now = self.clock.now_timestamp_millis();
        let node_run = match self.repository.begin_node_retry(node_run_id, now)? {
            BeginNodeRetryResult::Started(node_run) => *node_run,
            BeginNodeRetryResult::NotDue { due_at } => {
                self.retry_timer.arm(run_id, node_run_id, due_at);
                return Ok(());
            }
            BeginNodeRetryResult::Abandoned => {
                self.run_events.publish_run_invalidated(run_id);
                return Ok(());
            }
            BeginNodeRetryResult::NotWaiting | BeginNodeRetryResult::NotFound => return Ok(()),
        };
        self.run_events.publish_run_invalidated(run_id);

        // The attempt is started now, so from here on an error must fail it: returning the error
        // would leave a `Running` row that no executor and no timer will ever finish.
        let failure = match self.dispatch_started_retry(run_id, &node_run) {
            Ok(true) => return Ok(()),
            Ok(false) => NodeFailure::new(
                NodeFailureKind::InvalidRunPayload,
                format!(
                    "retry of node {} has no executable target in its scope",
                    node_run.node_id
                ),
            ),
            Err(error) => {
                let kind = match &error {
                    EngineError::Repository(_) => NodeFailureKind::Repository,
                    EngineError::WorkflowRunNotFound { .. }
                    | EngineError::GraphParse(_)
                    | EngineError::Validation(_)
                    | EngineError::LoopState { .. } => NodeFailureKind::InvalidRunPayload,
                };
                NodeFailure::new(
                    kind,
                    format!(
                        "retry of node {} could not be dispatched: {error}",
                        node_run.node_id
                    ),
                )
            }
        };
        self.fail_node(run_id, &node_run.id, failure)
    }

    /// Dispatches a started retry exactly as the original attempt was dispatched: root-scope
    /// rows (outer nodes and iteration region members) run against the outer graph and the run
    /// pool; a Loop round's rows run against the Loop body and that round's isolated pool.
    ///
    /// Returns `false` when the attempt's scope has no executable target.
    fn dispatch_started_retry(
        &self,
        run_id: &WorkflowRunId,
        node_run: &WorkflowNodeRun,
    ) -> Result<bool, EngineError> {
        let context = self.execution_context(run_id)?;
        let graph = WorkflowGraph::parse(&context.graph_json)?;
        let mut target = None;
        if node_run.scope_id == context.root_scope_id {
            let (pool, _) = execution_state_from(context.run.payload.as_deref());
            target = Some((&graph, pool));
        } else {
            for loop_run in self
                .repository
                .list_node_runs_in_scope(&context.root_scope_id)?
                .iter()
                .filter(|row| row.status == WorkflowNodeStatus::Running)
            {
                let Some((_, body)) = graph.loop_body(&loop_run.node_id) else {
                    continue;
                };
                let Some(scope) = self.repository.find_active_loop_round(&loop_run.id)? else {
                    continue;
                };
                if scope.id == node_run.scope_id {
                    let state = serde_json::from_str::<LoopRoundExecutionState>(&scope.state)
                        .map_err(|error| EngineError::LoopState {
                            message: error.to_string(),
                        })?;
                    target = Some((body, state.variable_pool));
                    break;
                }
            }
        }
        if let Some((scope_graph, pool)) = target
            && let Some(node) = scope_graph.node(&node_run.node_id)
            && let Some(RegisteredNodeRuntime::Async(runtime)) =
                self.runtimes.runtime(node.node_type)
        {
            runtime.dispatch(
                &node_run.id,
                node,
                &context,
                scope_graph,
                &node_run.scope_id,
                &pool,
            );
            return Ok(true);
        }
        Ok(false)
    }
}
