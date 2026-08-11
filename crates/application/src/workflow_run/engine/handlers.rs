use crate::workflow_run::mapper::map_run;
use crate::workflow_run::{
    CancelWorkflowRunResult, NodeExecutor, RestartWorkflowRunResult, UpdateWorkflowRunInputResult,
    WorkflowNodeRunIdGenerator, WorkflowRunEngine, WorkflowRunEngineRepository,
    WorkflowRunRepository,
};
use crate::{ApplicationError, Clock};
use ora_contracts::{
    CancelWorkflowRunRequest, CancelWorkflowRunResponse, RestartWorkflowRunRequest,
    RestartWorkflowRunResponse, StartWorkflowRunRequest, StartWorkflowRunResponse,
    UpdateWorkflowRunInputRequest, UpdateWorkflowRunInputResponse,
};
use ora_domain::{WorkflowRun, WorkflowRunId};
use std::sync::Arc;

/// Exposes the engine's start/cancel/restart operations as application handlers.
///
/// Each command runs the engine, then returns the run's current state for the transport layer.
pub struct WorkflowRunControlHandler<R, E, G, C, RunRepo> {
    engine: WorkflowRunEngine<R, E, G, C>,
    run_repository: Arc<RunRepo>,
}

impl<R, E, G, C, RunRepo> WorkflowRunControlHandler<R, E, G, C, RunRepo>
where
    R: WorkflowRunEngineRepository,
    E: NodeExecutor,
    G: WorkflowNodeRunIdGenerator,
    C: Clock,
    RunRepo: WorkflowRunRepository,
{
    /// Builds a control handler from the run engine and the run read repository.
    pub fn new(engine: WorkflowRunEngine<R, E, G, C>, run_repository: Arc<RunRepo>) -> Self {
        Self {
            engine,
            run_repository,
        }
    }

    /// Starts a run, returning the run in its current state (idempotent when already started).
    pub fn start(
        &self,
        request: StartWorkflowRunRequest,
    ) -> Result<StartWorkflowRunResponse, ApplicationError> {
        let run_id = WorkflowRunId::new(&request.run_id);
        self.engine
            .start(&run_id)
            .map_err(ApplicationError::from_workflow_engine_error)?;
        let run = self.find_run(&run_id)?;
        Ok(StartWorkflowRunResponse { run: map_run(run) })
    }

    /// Cancels a running run, returning the cancelled (or already terminal) run.
    pub fn cancel(
        &self,
        request: CancelWorkflowRunRequest,
    ) -> Result<CancelWorkflowRunResponse, ApplicationError> {
        let run_id = WorkflowRunId::new(&request.run_id);
        match self
            .engine
            .cancel(&run_id)
            .map_err(ApplicationError::from_workflow_engine_error)?
        {
            CancelWorkflowRunResult::Cancelled | CancelWorkflowRunResult::NotActive => {}
            CancelWorkflowRunResult::NotFound => {
                return Err(ApplicationError::WorkflowRunNotFound {
                    run_id: request.run_id,
                });
            }
        }
        let run = self.find_run(&run_id)?;
        Ok(CancelWorkflowRunResponse { run: map_run(run) })
    }

    /// Restarts a non-running run, returning the reset and re-running run.
    pub fn restart(
        &self,
        request: RestartWorkflowRunRequest,
    ) -> Result<RestartWorkflowRunResponse, ApplicationError> {
        let run_id = WorkflowRunId::new(&request.run_id);
        match self
            .engine
            .restart(&run_id)
            .map_err(ApplicationError::from_workflow_engine_error)?
        {
            RestartWorkflowRunResult::Restarted => {}
            RestartWorkflowRunResult::NotRestartable => {
                return Err(ApplicationError::WorkflowRunNotRestartable);
            }
            RestartWorkflowRunResult::NotFound => {
                return Err(ApplicationError::WorkflowRunNotFound {
                    run_id: request.run_id,
                });
            }
        }
        let run = self.find_run(&run_id)?;
        Ok(RestartWorkflowRunResponse { run: map_run(run) })
    }

    /// Sets the kickoff input of a pending run.
    pub fn update_input(
        &self,
        request: UpdateWorkflowRunInputRequest,
    ) -> Result<UpdateWorkflowRunInputResponse, ApplicationError> {
        let run_id = WorkflowRunId::new(&request.run_id);
        match self
            .engine
            .update_run_input(&run_id, request.input)
            .map_err(ApplicationError::from_workflow_engine_error)?
        {
            UpdateWorkflowRunInputResult::Updated => {}
            UpdateWorkflowRunInputResult::NotEditable => {
                return Err(ApplicationError::WorkflowRunNotEditable);
            }
            UpdateWorkflowRunInputResult::NotFound => {
                return Err(ApplicationError::WorkflowRunNotFound {
                    run_id: request.run_id,
                });
            }
        }
        let run = self.find_run(&run_id)?;
        Ok(UpdateWorkflowRunInputResponse { run: map_run(run) })
    }

    /// Loads one visible run or reports it missing.
    fn find_run(&self, run_id: &WorkflowRunId) -> Result<WorkflowRun, ApplicationError> {
        self.run_repository
            .find_run(run_id)
            .map_err(ApplicationError::from_workflow_run_repository_error)?
            .ok_or_else(|| ApplicationError::WorkflowRunNotFound {
                run_id: run_id.to_string(),
            })
    }
}
