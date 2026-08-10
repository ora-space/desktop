use crate::agent_runtime::AgentRuntimeManager;
use crate::clock::SystemClock;
use crate::workflow_run_executor::WorkflowRunNodeExecutor;
use ora_application::{
    FileChange, UuidWorkflowNodeRunIdGenerator, WorkflowRunCallback, WorkflowRunControlHandler,
    WorkflowRunEngine,
};
use ora_db::{
    RepositoryPool, SqliteAgentDefinitionRepository, SqliteWorkflowRunEngineRepository,
    SqliteWorkflowRunRepository,
};
use ora_domain::{WorkflowNodeRunId, WorkflowRunId};
use ora_logging::ora_error;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

/// The concrete run engine as composed by the backend.
pub(crate) type ConcreteWorkflowRunEngine = WorkflowRunEngine<
    SqliteWorkflowRunEngineRepository,
    WorkflowRunNodeExecutor,
    UuidWorkflowNodeRunIdGenerator,
    SystemClock,
>;

/// The concrete control handler exposed to the Web and Tauri adapters.
pub(crate) type ConcreteWorkflowRunControl = WorkflowRunControlHandler<
    SqliteWorkflowRunEngineRepository,
    WorkflowRunNodeExecutor,
    UuidWorkflowNodeRunIdGenerator,
    SystemClock,
    SqliteWorkflowRunRepository,
>;

/// Routes session-driver completions back to the run engine.
///
/// The callback is created before the engine (the engine embeds the executor, which embeds this
/// callback), so the engine reference is attached once the composition root finishes building.
pub(crate) struct WorkflowRunEngineCallback {
    engine: RwLock<Option<Arc<ConcreteWorkflowRunEngine>>>,
}

impl WorkflowRunEngineCallback {
    /// Creates a callback with no engine attached yet.
    fn new() -> Self {
        Self {
            engine: RwLock::new(None),
        }
    }

    /// Attaches the engine once the composition root has built it.
    fn set_engine(&self, engine: Arc<ConcreteWorkflowRunEngine>) {
        if let Ok(mut guard) = self.engine.write() {
            *guard = Some(engine);
        }
    }
}

impl WorkflowRunCallback for WorkflowRunEngineCallback {
    fn complete_node(
        &self,
        run_id: &WorkflowRunId,
        node_run_id: &WorkflowNodeRunId,
        output: Option<String>,
        stop_reason: Option<String>,
        file_changes: Vec<FileChange>,
    ) {
        if let Ok(guard) = self.engine.read()
            && let Some(engine) = guard.as_ref()
            && let Err(error) =
                engine.complete_node(run_id, node_run_id, output, stop_reason, file_changes)
        {
            ora_error!(run_id = %run_id, node_run_id = %node_run_id, error = %error, "node completion callback failed");
        }
    }

    fn fail_node(
        &self,
        run_id: &WorkflowRunId,
        node_run_id: &WorkflowNodeRunId,
        error: String,
        output: Option<String>,
    ) {
        if let Ok(guard) = self.engine.read()
            && let Some(engine) = guard.as_ref()
            && let Err(callback_error) = engine.fail_node(node_run_id, error, output)
        {
            ora_error!(run_id = %run_id, node_run_id = %node_run_id, error = %callback_error, "node fail callback failed");
        }
    }
}

/// The run engine control handler, as built by the composition root.
pub(crate) struct WorkflowRunEngineAssembly {
    pub control: Arc<ConcreteWorkflowRunControl>,
}

/// Builds the run engine, its session executor, and control handler.
pub(crate) fn build_workflow_run_engine(
    agent_runtime: Arc<AgentRuntimeManager>,
    pool: RepositoryPool,
    skills_root: PathBuf,
    clock: SystemClock,
) -> WorkflowRunEngineAssembly {
    let callback = Arc::new(WorkflowRunEngineCallback::new());
    let executor = WorkflowRunNodeExecutor::new(
        agent_runtime,
        pool.clone(),
        skills_root,
        SqliteAgentDefinitionRepository::new(pool.clone()),
        callback.clone(),
        clock,
    );
    let engine = Arc::new(WorkflowRunEngine::new(
        SqliteWorkflowRunEngineRepository::new(pool.clone()),
        executor,
        UuidWorkflowNodeRunIdGenerator::new(),
        clock,
    ));
    callback.set_engine(engine.clone());
    let control = Arc::new(WorkflowRunControlHandler::new(
        (*engine).clone(),
        Arc::new(SqliteWorkflowRunRepository::new(pool)),
    ));
    WorkflowRunEngineAssembly { control }
}
