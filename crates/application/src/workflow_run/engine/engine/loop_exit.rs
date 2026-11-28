//! Durable break completion, separated from normal feedback-round advancement.
use super::{EngineError, LoopScheduleOutcome, WorkflowRunEngine};
use crate::{
    Clock, ExecutionContext, LoopExitCleanup, LoopRoundAdvance, LoopRoundExecutionState,
    WorkflowNodeRunIdGenerator, WorkflowRunEngineRepository,
};
use ora_domain::{
    WorkflowExecutionScope, WorkflowNodeRun, WorkflowNodeRunId, WorkflowRunId, WorkflowScopeId,
};

impl<R, G, C> WorkflowRunEngine<R, G, C>
where
    R: WorkflowRunEngineRepository,
    G: WorkflowNodeRunIdGenerator,
    C: Clock,
{
    /// Hands cleanup to the executor without blocking the run gate; restart repeats this safely.
    pub(super) fn drive_loop_exit_cleanup(
        &self,
        context: &ExecutionContext,
        parent: &WorkflowNodeRun,
        scope: &WorkflowExecutionScope,
    ) -> Result<LoopScheduleOutcome, EngineError> {
        match self
            .node_executor
            .cleanup_loop_exit(context, &parent.id, &scope.id)
        {
            LoopExitCleanup::Complete => {
                self.settle_loop_exit(&context.run.id, &parent.id, &scope.id, Ok(()))?;
                Ok(LoopScheduleOutcome::Progressed)
            }
            LoopExitCleanup::Pending => Ok(LoopScheduleOutcome::Waiting),
        }
    }

    /// Accepts a cleanup acknowledgement only for the same still-active parent and scope.
    pub fn finish_loop_exit(
        &self,
        run_id: &WorkflowRunId,
        parent_id: &WorkflowNodeRunId,
        scope_id: &WorkflowScopeId,
        result: Result<(), String>,
    ) -> Result<(), EngineError> {
        self.settle_loop_exit(run_id, parent_id, scope_id, result)?;
        self.run_events.publish_run_invalidated(run_id);
        self.run_schedule(run_id)
    }

    /// Uses frozen exit outputs instead of rereading a pool that late callbacks could change.
    fn settle_loop_exit(
        &self,
        run_id: &WorkflowRunId,
        parent_id: &WorkflowNodeRunId,
        scope_id: &WorkflowScopeId,
        result: Result<(), String>,
    ) -> Result<(), EngineError> {
        let Some(scope) = self.repository.find_active_loop_round(parent_id)? else {
            return Ok(());
        };
        if scope.id != *scope_id || scope.run_id != *run_id {
            return Ok(());
        }
        let state: LoopRoundExecutionState =
            serde_json::from_str(&scope.state).map_err(|error| EngineError::LoopState {
                message: error.to_string(),
            })?;
        let Some(exit) = state.exit else {
            return Ok(());
        };
        self.repository.advance_loop_round(
            scope_id,
            &LoopRoundAdvance::FinishExit {
                node_run_id: exit.node_run_id,
                cleanup: result,
            },
            self.clock.now_timestamp_millis(),
        )?;
        Ok(())
    }
}
