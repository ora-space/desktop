use std::path::Path;
use std::sync::Arc;

use crate::workflow::WorkflowRepository;
use crate::workflow_run::mapper::{map_node_run, map_run, map_run_awaiting, map_run_summary};
use crate::workflow_run::{
    DeleteWorkflowRunResult, WorkflowRunCreateOutcome, WorkflowRunIdGenerator, WorkflowRunPayload,
    WorkflowRunRepository, WorkflowRunWorkspaceInitializer, WorkspaceRepository,
};
use crate::{ApplicationError, Clock, WorkflowGraph};
use ora_contracts::{
    CreateWorkflowRunRequest, CreateWorkflowRunResponse, DeleteWorkflowRunRequest,
    DeleteWorkflowRunResponse, GetWorkflowRunRequest, GetWorkflowRunResponse,
    ListWorkflowNodeRunsRequest, ListWorkflowNodeRunsResponse, ListWorkflowRunsByWorkflowRequest,
    ListWorkflowRunsByWorkflowResponse, ListWorkflowRunsRequest, ListWorkflowRunsResponse,
    RenameWorkflowRunRequest, RenameWorkflowRunResponse,
};
use ora_domain::{
    AuditFields, Workflow, WorkflowId, WorkflowNodeStatus, WorkflowRun, WorkflowRunId,
    WorkflowRunStatus, WorkflowSnapshot, WorkflowSnapshotId, WorkspaceId, WorkspaceLocation,
};

const DRAFT_VERSION: &str = "draft";

/// Handles creation of a workflow run directly inside an admitted workspace.
pub struct CreateWorkflowRunHandler<
    WorkflowRepositoryPort,
    WorkspaceRepositoryPort,
    RunRepositoryPort,
    RunIdGenerator,
    WorkspaceInitializer,
    ClockSource,
> {
    workflow_repository: Arc<WorkflowRepositoryPort>,
    workspace_repository: Arc<WorkspaceRepositoryPort>,
    run_repository: Arc<RunRepositoryPort>,
    run_id_generator: RunIdGenerator,
    workspace_initializer: WorkspaceInitializer,
    clock: ClockSource,
}

impl<
    WorkflowRepositoryPort,
    WorkspaceRepositoryPort,
    RunRepositoryPort,
    RunIdGenerator,
    WorkspaceInitializer,
    ClockSource,
>
    CreateWorkflowRunHandler<
        WorkflowRepositoryPort,
        WorkspaceRepositoryPort,
        RunRepositoryPort,
        RunIdGenerator,
        WorkspaceInitializer,
        ClockSource,
    >
{
    /// Builds a handler from workflow, workspace, run, and workspace-initialization ports.
    pub fn new(
        workflow_repository: Arc<WorkflowRepositoryPort>,
        workspace_repository: Arc<WorkspaceRepositoryPort>,
        run_repository: Arc<RunRepositoryPort>,
        run_id_generator: RunIdGenerator,
        workspace_initializer: WorkspaceInitializer,
        clock: ClockSource,
    ) -> Self {
        Self {
            workflow_repository,
            workspace_repository,
            run_repository,
            run_id_generator,
            workspace_initializer,
            clock,
        }
    }
}

impl<
    WorkflowRepositoryPort,
    WorkspaceRepositoryPort,
    RunRepositoryPort,
    RunIdGenerator,
    WorkspaceInitializer,
    ClockSource,
>
    CreateWorkflowRunHandler<
        WorkflowRepositoryPort,
        WorkspaceRepositoryPort,
        RunRepositoryPort,
        RunIdGenerator,
        WorkspaceInitializer,
        ClockSource,
    >
where
    WorkflowRepositoryPort: WorkflowRepository + Send + Sync + 'static,
    WorkspaceRepositoryPort: WorkspaceRepository + Send + Sync + 'static,
    RunRepositoryPort: WorkflowRunRepository + Send + Sync + 'static,
    RunIdGenerator: WorkflowRunIdGenerator,
    WorkspaceInitializer: WorkflowRunWorkspaceInitializer,
    ClockSource: Clock + Clone + Send + 'static,
{
    /// Resolves the frozen snapshot, prepares the selected workspace, and persists the run.
    pub fn handle(
        &self,
        request: CreateWorkflowRunRequest,
    ) -> Result<CreateWorkflowRunResponse, ApplicationError> {
        let now = self.clock.now_timestamp_millis();
        let workspace_id = WorkspaceId::new(request.workspace_id);
        let workspace = self
            .workspace_repository
            .find_workspace(&workspace_id)
            .map_err(ApplicationError::from_workflow_run_repository_error)?
            .ok_or_else(|| workspace_admission_error(&workspace_id, "workspace not found"))?;
        if !workspace.is_admissible() {
            return Err(workspace_admission_error(
                &workspace_id,
                "workspace is not active",
            ));
        }
        if !self
            .workspace_repository
            .is_provisioning_ready(&workspace_id)
            .map_err(ApplicationError::from_workflow_run_repository_error)?
        {
            return Err(workspace_admission_error(
                &workspace_id,
                "workspace provisioning is not ready",
            ));
        }

        let workflow_id = WorkflowId::new(request.workflow_id);
        let workflow = self
            .workflow_repository
            .find_workflow(&workflow_id)
            .map_err(ApplicationError::from_workflow_repository_error)?
            .ok_or_else(|| ApplicationError::WorkflowNotFound {
                workflow_id: workflow_id.to_string(),
            })?;
        let snapshot = self.resolve_snapshot(&workflow_id, request.snapshot_id, &workflow)?;
        let graph = WorkflowGraph::parse(&snapshot.graph)
            .map_err(ApplicationError::WorkflowRunGraphParse)?;
        let kickoff_input = request
            .kickoff_input
            .or_else(|| graph.start_node().and_then(|node| node.instruction.clone()));
        let workspace_root = match workspace.location {
            WorkspaceLocation::LocalFilesystem { path } => path,
            WorkspaceLocation::Ssh { .. } | WorkspaceLocation::RemoteTarget { .. } => {
                return Err(workspace_admission_error(
                    &workspace_id,
                    "workflow execution does not support this workspace location yet",
                ));
            }
        };
        let skill_materialization = self
            .workspace_initializer
            .initialize_workspace(&graph, Path::new(&workspace_root))
            .map_err(ApplicationError::from_start_prerequisites_error)?;
        let run_payload = serde_json::to_string(&WorkflowRunPayload::new(
            request.locale,
            skill_materialization,
        ))
        .map_err(|error| ApplicationError::WorkflowRunStartFailed {
            message: format!("failed to serialize workflow run payload: {error}"),
        })?;
        let name = request
            .name
            .unwrap_or_else(|| default_run_title(&workflow.name, now));
        let run = WorkflowRun::new(
            self.run_id_generator.generate_run_id(),
            workspace_id.clone(),
            workflow_id,
            snapshot.id,
            name,
            WorkflowRunStatus::Pending,
            Some("{\"current_nodes\":[]}".to_string()),
            kickoff_input,
            None,
            None,
            Some(run_payload),
            None,
            None,
            AuditFields::new(now, now, /*is_deleted*/ false),
        );
        let created = self
            .run_repository
            .create_run(run)
            .map_err(ApplicationError::from_workflow_run_repository_error)?;
        match created {
            WorkflowRunCreateOutcome::Created(run) => {
                Ok(CreateWorkflowRunResponse { run: map_run(*run) })
            }
            WorkflowRunCreateOutcome::WorkspaceNotVisible => Err(workspace_admission_error(
                &workspace_id,
                "workspace is no longer visible",
            )),
        }
    }

    /// Resolves the snapshot a run freezes: an explicit id, or the workflow's published snapshot.
    fn resolve_snapshot(
        &self,
        workflow_id: &WorkflowId,
        explicit_snapshot_id: Option<String>,
        workflow: &Workflow,
    ) -> Result<WorkflowSnapshot, ApplicationError> {
        let snapshot_id = match explicit_snapshot_id {
            Some(id) => WorkflowSnapshotId::new(id),
            None => workflow
                .published_snapshot_id
                .clone()
                .ok_or(ApplicationError::WorkflowNoPublishedSnapshot)?,
        };
        let snapshot = self
            .workflow_repository
            .find_snapshot_by_id(workflow_id, &snapshot_id)
            .map_err(ApplicationError::from_workflow_repository_error)?
            .ok_or_else(|| ApplicationError::WorkflowSnapshotNotFoundById {
                snapshot_id: snapshot_id.to_string(),
            })?;
        if snapshot.version == DRAFT_VERSION {
            return Err(ApplicationError::WorkflowRunCannotUseDraftSnapshot);
        }
        Ok(snapshot)
    }
}

/// Builds the default display name when a caller does not provide one.
fn default_run_title(workflow_name: &str, now_millis: i64) -> String {
    format!("{workflow_name} {now_millis}")
}

/// Builds the stable application error used when a workspace cannot admit a run.
fn workspace_admission_error(workspace_id: &WorkspaceId, reason: &str) -> ApplicationError {
    ApplicationError::WorkflowRunStartFailed {
        message: format!("{reason}: {workspace_id}"),
    }
}

/// Handles lookup of one workflow run with its workspace context and node runs.
pub struct GetWorkflowRunHandler<Repository> {
    repository: Arc<Repository>,
}

impl<Repository> GetWorkflowRunHandler<Repository> {
    /// Builds a get-run handler over the shared run repository.
    pub fn new(repository: Arc<Repository>) -> Self {
        Self { repository }
    }
}

impl<Repository> GetWorkflowRunHandler<Repository>
where
    Repository: WorkflowRunRepository + Send + Sync + 'static,
{
    /// Loads one run detail or reports a not-found error.
    pub fn handle(
        &self,
        request: GetWorkflowRunRequest,
    ) -> Result<GetWorkflowRunResponse, ApplicationError> {
        let run_id = WorkflowRunId::new(request.run_id);
        let detail = self
            .repository
            .get_run_detail(&run_id)
            .map_err(ApplicationError::from_workflow_run_repository_error)?
            .ok_or_else(|| ApplicationError::WorkflowRunNotFound {
                run_id: run_id.to_string(),
            })?;
        let has_awaiting_node = detail
            .nodes
            .iter()
            .any(|node_run| node_run.status == WorkflowNodeStatus::Pending);
        Ok(GetWorkflowRunResponse {
            run: map_run_awaiting(detail.run, has_awaiting_node),
            name: detail.name,
            workspace_id: detail.workspace_id.to_string(),
            project_id: detail.project_id.to_string(),
            nodes: detail.nodes.into_iter().map(map_node_run).collect(),
        })
    }
}

/// Handles listing of visible workflow runs for one project.
pub struct ListWorkflowRunsHandler<Repository> {
    repository: Arc<Repository>,
}

impl<Repository> ListWorkflowRunsHandler<Repository> {
    /// Builds a list-runs handler over the shared run repository.
    pub fn new(repository: Arc<Repository>) -> Self {
        Self { repository }
    }
}

impl<Repository> ListWorkflowRunsHandler<Repository>
where
    Repository: WorkflowRunRepository + Send + Sync + 'static,
{
    /// Lists run summaries for the requested project in stable order.
    pub fn handle(
        &self,
        request: ListWorkflowRunsRequest,
    ) -> Result<ListWorkflowRunsResponse, ApplicationError> {
        let project_id = ora_domain::ProjectId::new(request.project_id);
        let runs = self
            .repository
            .list_runs_by_project(&project_id)
            .map_err(ApplicationError::from_workflow_run_repository_error)?;
        Ok(ListWorkflowRunsResponse {
            runs: runs.into_iter().map(map_run_summary).collect(),
        })
    }
}

/// Handles listing of visible workflow runs for one workflow.
pub struct ListWorkflowRunsByWorkflowHandler<Repository> {
    repository: Arc<Repository>,
}

impl<Repository> ListWorkflowRunsByWorkflowHandler<Repository> {
    /// Builds a list-runs-by-workflow handler over the shared run repository.
    pub fn new(repository: Arc<Repository>) -> Self {
        Self { repository }
    }
}

impl<Repository> ListWorkflowRunsByWorkflowHandler<Repository>
where
    Repository: WorkflowRunRepository + Send + Sync + 'static,
{
    /// Lists run summaries for the requested workflow in stable order.
    pub fn handle(
        &self,
        request: ListWorkflowRunsByWorkflowRequest,
    ) -> Result<ListWorkflowRunsByWorkflowResponse, ApplicationError> {
        let workflow_id = WorkflowId::new(request.workflow_id);
        let runs = self
            .repository
            .list_runs_by_workflow(&workflow_id)
            .map_err(ApplicationError::from_workflow_run_repository_error)?;
        Ok(ListWorkflowRunsByWorkflowResponse {
            runs: runs.into_iter().map(map_run_summary).collect(),
        })
    }
}

/// Handles listing of one run's node-run history.
pub struct ListWorkflowNodeRunsHandler<Repository> {
    repository: Arc<Repository>,
}

/// Handles replacement of a workflow run's Workspace-owned display name.
pub struct RenameWorkflowRunHandler<Repository, ClockSource> {
    repository: Arc<Repository>,
    clock: ClockSource,
}

impl<Repository, ClockSource> RenameWorkflowRunHandler<Repository, ClockSource> {
    /// Builds a rename handler over the shared run repository and clock.
    pub fn new(repository: Arc<Repository>, clock: ClockSource) -> Self {
        Self { repository, clock }
    }
}

impl<Repository, ClockSource> RenameWorkflowRunHandler<Repository, ClockSource>
where
    Repository: WorkflowRunRepository + Send + Sync + 'static,
    ClockSource: Clock,
{
    /// Renames one visible run while preserving its workspace and execution facts.
    pub fn handle(
        &self,
        request: RenameWorkflowRunRequest,
    ) -> Result<RenameWorkflowRunResponse, ApplicationError> {
        let run_id = WorkflowRunId::new(request.run_id);
        let name = request.name.trim().to_string();
        let run = self
            .repository
            .rename_run(&run_id, name, self.clock.now_timestamp_millis())
            .map_err(ApplicationError::from_workflow_run_repository_error)?
            .ok_or_else(|| ApplicationError::WorkflowRunNotFound {
                run_id: run_id.to_string(),
            })?;
        Ok(RenameWorkflowRunResponse { run: map_run(run) })
    }
}

impl<Repository> ListWorkflowNodeRunsHandler<Repository> {
    /// Builds a list-node-runs handler over the shared run repository.
    pub fn new(repository: Arc<Repository>) -> Self {
        Self { repository }
    }
}

impl<Repository> ListWorkflowNodeRunsHandler<Repository>
where
    Repository: WorkflowRunRepository + Send + Sync + 'static,
{
    /// Lists the node-run records of one run in stable ascending order.
    pub fn handle(
        &self,
        request: ListWorkflowNodeRunsRequest,
    ) -> Result<ListWorkflowNodeRunsResponse, ApplicationError> {
        let run_id = WorkflowRunId::new(request.run_id);
        let nodes = self
            .repository
            .list_node_runs(&run_id)
            .map_err(ApplicationError::from_workflow_run_repository_error)?;
        Ok(ListWorkflowNodeRunsResponse {
            nodes: nodes.into_iter().map(map_node_run).collect(),
        })
    }
}

/// Handles soft-deletion of a workflow run without deleting its shared workspace.
pub struct DeleteWorkflowRunHandler<Repository, ClockSource> {
    repository: Arc<Repository>,
    clock: ClockSource,
}

impl<Repository, ClockSource> DeleteWorkflowRunHandler<Repository, ClockSource> {
    /// Builds a delete handler over the shared run repository and clock.
    pub fn new(repository: Arc<Repository>, clock: ClockSource) -> Self {
        Self { repository, clock }
    }
}

impl<Repository, ClockSource> DeleteWorkflowRunHandler<Repository, ClockSource>
where
    Repository: WorkflowRunRepository + Send + Sync + 'static,
    ClockSource: Clock,
{
    /// Soft-deletes one run and its node-owned sessions after refusing active execution.
    pub fn handle(
        &self,
        request: DeleteWorkflowRunRequest,
    ) -> Result<DeleteWorkflowRunResponse, ApplicationError> {
        let run_id = WorkflowRunId::new(request.run_id);
        match self
            .repository
            .soft_delete_run(&run_id, self.clock.now_timestamp_millis())
            .map_err(ApplicationError::from_workflow_run_repository_error)?
        {
            DeleteWorkflowRunResult::Deleted => Ok(DeleteWorkflowRunResponse {
                run_id: run_id.to_string(),
            }),
            DeleteWorkflowRunResult::NotFound => Err(ApplicationError::WorkflowRunNotFound {
                run_id: run_id.to_string(),
            }),
            DeleteWorkflowRunResult::ActiveRun => Err(ApplicationError::WorkflowRunActive),
        }
    }
}
