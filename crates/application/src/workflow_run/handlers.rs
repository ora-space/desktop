use crate::task::branch::branch_name_for_task;
use crate::task::{
    PROVISIONING_LEASE_DURATION_MS, ProvisioningLeaseRenewal, WorktreeProvisioningLeaseStore,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

use crate::task::{CreateTaskWorktreeRequest, TaskIdGenerator, TaskWorktreeProvisioner};
use crate::workflow::WorkflowRepository;
use crate::workflow_run::mapper::{map_node_run, map_run, map_run_summary};
use crate::workflow_run::{
    DeleteWorkflowRunResult, WorkflowRunCreateOutcome, WorkflowRunIdGenerator,
    WorkflowRunRepository, WorkflowRunWorktreeInitializer,
};
use crate::worktree::WorktreeIdGenerator;
use crate::{ApplicationError, Clock, WorkflowGraph};
use ora_contracts::{
    CreateWorkflowRunRequest, CreateWorkflowRunResponse, DeleteWorkflowRunRequest,
    DeleteWorkflowRunResponse, GetWorkflowRunRequest, GetWorkflowRunResponse,
    ListWorkflowNodeRunsRequest, ListWorkflowNodeRunsResponse, ListWorkflowRunsByWorkflowRequest,
    ListWorkflowRunsByWorkflowResponse, ListWorkflowRunsRequest, ListWorkflowRunsResponse,
};
use ora_domain::{
    AuditFields, ProjectId, Task, TaskId, Workflow, WorkflowId, WorkflowRun, WorkflowRunId,
    WorkflowRunStatus, WorkflowSnapshot, WorkflowSnapshotId, Worktree, WorktreeActivity,
    WorktreeBaseline, WorktreeProvisioningLease, WorktreeProvisioningLeaseId,
};

const DRAFT_VERSION: &str = "draft";
const DEFAULT_RUN_BASE_REFERENCE: &str = "main";

/// Handles creation of a workflow run against a published snapshot with a dedicated worktree.
pub struct CreateWorkflowRunHandler<
    WorkflowRepositoryPort,
    RunRepositoryPort,
    RunIdGenerator,
    TaskIdGeneratorPort,
    WorktreeIdGeneratorPort,
    WorktreeProvisioner,
    WorktreeInitializer,
    LeaseStorePort,
    ClockSource,
> {
    workflow_repository: Arc<WorkflowRepositoryPort>,
    run_repository: Arc<RunRepositoryPort>,
    run_id_generator: RunIdGenerator,
    task_id_generator: TaskIdGeneratorPort,
    worktree_id_generator: WorktreeIdGeneratorPort,
    worktree_provisioner: WorktreeProvisioner,
    worktree_initializer: WorktreeInitializer,
    lease_store: LeaseStorePort,
    /// Root of the project's Git repository, persisted into leases.
    repository_root: PathBuf,
    work_dir: PathBuf,
    clock: ClockSource,
}

impl<
    WorkflowRepositoryPort,
    RunRepositoryPort,
    RunIdGenerator,
    TaskIdGeneratorPort,
    WorktreeIdGeneratorPort,
    WorktreeProvisioner,
    WorktreeInitializer,
    LeaseStorePort,
    ClockSource,
>
    CreateWorkflowRunHandler<
        WorkflowRepositoryPort,
        RunRepositoryPort,
        RunIdGenerator,
        TaskIdGeneratorPort,
        WorktreeIdGeneratorPort,
        WorktreeProvisioner,
        WorktreeInitializer,
        LeaseStorePort,
        ClockSource,
    >
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        workflow_repository: Arc<WorkflowRepositoryPort>,
        run_repository: Arc<RunRepositoryPort>,
        run_id_generator: RunIdGenerator,
        task_id_generator: TaskIdGeneratorPort,
        worktree_id_generator: WorktreeIdGeneratorPort,
        worktree_provisioner: WorktreeProvisioner,
        worktree_initializer: WorktreeInitializer,
        lease_store: LeaseStorePort,
        repository_root: PathBuf,
        work_dir: PathBuf,
        clock: ClockSource,
    ) -> Self {
        Self {
            workflow_repository,
            run_repository,
            run_id_generator,
            task_id_generator,
            worktree_id_generator,
            worktree_provisioner,
            worktree_initializer,
            lease_store,
            repository_root,
            work_dir,
            clock,
        }
    }
}

impl<
    WorkflowRepositoryPort,
    RunRepositoryPort,
    RunIdGenerator,
    TaskIdGeneratorPort,
    WorktreeIdGeneratorPort,
    WorktreeProvisioner,
    WorktreeInitializer,
    LeaseStorePort,
    ClockSource,
>
    CreateWorkflowRunHandler<
        WorkflowRepositoryPort,
        RunRepositoryPort,
        RunIdGenerator,
        TaskIdGeneratorPort,
        WorktreeIdGeneratorPort,
        WorktreeProvisioner,
        WorktreeInitializer,
        LeaseStorePort,
        ClockSource,
    >
where
    WorkflowRepositoryPort: WorkflowRepository + Send + Sync + 'static,
    RunRepositoryPort: WorkflowRunRepository + Send + Sync + 'static,
    RunIdGenerator: WorkflowRunIdGenerator,
    TaskIdGeneratorPort: TaskIdGenerator,
    WorktreeIdGeneratorPort: WorktreeIdGenerator,
    WorktreeProvisioner: TaskWorktreeProvisioner,
    WorktreeInitializer: WorkflowRunWorktreeInitializer,
    LeaseStorePort: WorktreeProvisioningLeaseStore,
    ClockSource: Clock + Clone + Send + 'static,
{
    /// Resolves the frozen snapshot and provisions a worktree before persisting the run atomically.
    pub fn handle(
        &self,
        request: CreateWorkflowRunRequest,
    ) -> Result<CreateWorkflowRunResponse, ApplicationError> {
        let now = self.clock.now_timestamp_millis();
        let workflow_id = WorkflowId::new(request.workflow_id);
        let project_id = ProjectId::new(request.project_id);

        let workflow = self
            .workflow_repository
            .find_workflow(&workflow_id)
            .map_err(ApplicationError::from_workflow_repository_error)?
            .ok_or_else(|| ApplicationError::WorkflowNotFound {
                workflow_id: workflow_id.to_string(),
            })?;
        let snapshot = self.resolve_snapshot(&workflow_id, request.snapshot_id, &workflow)?;

        let run_id = self.run_id_generator.generate_run_id();
        let task_id = self.task_id_generator.generate_task_id();
        let worktree_id = self.worktree_id_generator.generate_worktree_id();
        let branch_name = branch_name_for_task(&task_id);
        let worktree_path = worktree_path_for_task(&self.work_dir, &task_id);

        // The run-task worktree is created from the requested branch (like a normal task);
        // absent an explicit branch, keep the conventional main fallback for existing clients.
        let base_reference_name = request
            .base_branch
            .as_deref()
            .map(str::trim)
            .filter(|branch| !branch.is_empty())
            .unwrap_or(DEFAULT_RUN_BASE_REFERENCE);

        // Write-ahead lease: identical to ordinary task creation, the run-task's
        // provisioned Git resources always have a durable owner from here on.
        let lease = WorktreeProvisioningLease::new(
            WorktreeProvisioningLeaseId::new(Uuid::new_v4().to_string()),
            project_id.clone(),
            task_id.clone(),
            self.repository_root.to_string_lossy().into_owned(),
            worktree_path.to_string_lossy().into_owned(),
            branch_name.clone(),
            now + PROVISIONING_LEASE_DURATION_MS,
            now,
        );
        self.lease_store
            .create_lease(&lease)
            .map_err(ApplicationError::from_workflow_run_repository_error)?;
        let renewal =
            ProvisioningLeaseRenewal::spawn(self.lease_store.clone(), lease.id.clone(), {
                let clock = self.clock.clone();
                move || clock.now_timestamp_millis()
            });

        let provisioned =
            match self
                .worktree_provisioner
                .create_task_worktree(CreateTaskWorktreeRequest {
                    branch_name: branch_name.clone(),
                    base_reference_name: base_reference_name.to_string(),
                    worktree_path: worktree_path.clone(),
                }) {
                Ok(provisioned) => provisioned,
                Err(error) => {
                    drop(renewal);
                    self.release_lease_to_cleanup(&lease.id);
                    return Err(ApplicationError::from_task_worktree_provisioner_error(
                        error,
                    ));
                }
            };

        // Set up the worktree's initial state (validate declared roles and materialize enabled
        // skills) while the worktree is being created, so the run is born complete and `start`
        // needs no re-validation. A failure aborts creation; the durable cleanup
        // path reclaims the physical worktree and branch.
        let graph = match WorkflowGraph::parse(&snapshot.graph)
            .map_err(ApplicationError::WorkflowRunGraphParse)
        {
            Ok(graph) => graph,
            Err(error) => {
                drop(renewal);
                self.release_lease_to_cleanup(&lease.id);
                return Err(error);
            }
        };
        if let Err(error) = self
            .worktree_initializer
            .initialize_worktree(&graph, &worktree_path)
        {
            drop(renewal);
            self.release_lease_to_cleanup(&lease.id);
            return Err(ApplicationError::from_start_prerequisites_error(error));
        }

        let worktree = Worktree::new(
            worktree_id.clone(),
            task_id.clone(),
            Some(branch_name),
            Some(worktree_path.to_string_lossy().into_owned()),
            WorktreeBaseline::recorded(provisioned.base_commit_id).map_err(|error| {
                ApplicationError::TaskWorktreeProvisioner {
                    source: crate::TaskWorktreeProvisionerError::operation_failed(
                        "failed to record workflow run worktree baseline",
                        error,
                    ),
                }
            })?,
            WorktreeActivity::Active,
            AuditFields::new(now, now, /*is_deleted*/ false),
        );
        let title = request
            .name
            .unwrap_or_else(|| default_run_title(&workflow.name, now));
        let task = Task::workflow_run(
            task_id.clone(),
            project_id.clone(),
            title,
            run_id.clone(),
            worktree_id,
            AuditFields::new(now, now, /*is_deleted*/ false),
        );
        let run = WorkflowRun::new(
            run_id,
            workflow_id,
            snapshot.id,
            WorkflowRunStatus::Pending,
            Some("{\"current_nodes\":[]}".to_string()),
            request.kickoff_input,
            None,
            None,
            None,
            None,
            None,
            AuditFields::new(now, now, /*is_deleted*/ false),
        );

        let created = self
            .run_repository
            .create_run(run, task, worktree, &lease.id);
        drop(renewal);
        match created {
            Ok(WorkflowRunCreateOutcome::Created(created)) => Ok(CreateWorkflowRunResponse {
                run: map_run(*created),
                task_id: task_id.to_string(),
            }),
            // The owning project was deleted while Git work ran; durable
            // cleanup reclaims the provisioned worktree and branch.
            Ok(WorkflowRunCreateOutcome::ProjectNotVisible) => {
                self.release_lease_to_cleanup(&lease.id);
                Err(ApplicationError::ProjectNotFound {
                    project_id: project_id.to_string(),
                })
            }
            Err(error) => {
                self.release_lease_to_cleanup(&lease.id);
                Err(ApplicationError::from_workflow_run_repository_error(error))
            }
        }
    }

    /// Hands the lease's Git resources to the durable cleanup path.
    ///
    /// Failure is tolerated: the lease then simply expires and the cleanup
    /// worker reclaims it on schedule.
    fn release_lease_to_cleanup(&self, lease_id: &WorktreeProvisioningLeaseId) {
        let _ = self
            .lease_store
            .release_to_cleanup(lease_id, self.clock.now_timestamp_millis());
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

/// Derives the owned linked-worktree path from the configured worktree root and full task id.
fn worktree_path_for_task(work_dir: &Path, task_id: &TaskId) -> PathBuf {
    work_dir.join(task_id.to_string())
}

/// Builds the default run-task title as `"{workflow.name} {创建时间}"`.
///
/// The time component uses the injected clock's epoch-millis creation timestamp; a human-readable
/// local-time rendering is a display refinement and intentionally not pinned here.
fn default_run_title(workflow_name: &str, now_millis: i64) -> String {
    format!("{workflow_name} {now_millis}")
}

/// Handles lookup of one workflow run with its display name and node runs.
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

        Ok(GetWorkflowRunResponse {
            run: map_run(detail.run),
            name: detail.name,
            project_id: detail.project_id.to_string(),
            task_id: detail.task_id.to_string(),
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
        let project_id = ProjectId::new(request.project_id);
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

/// Handles soft-deletion of a workflow run; physical Git cleanup is durable.
pub struct DeleteWorkflowRunHandler<Repository, ClockSource> {
    repository: Arc<Repository>,
    clock: ClockSource,
}

impl<Repository, ClockSource> DeleteWorkflowRunHandler<Repository, ClockSource> {
    pub fn new(repository: Arc<Repository>, clock: ClockSource) -> Self {
        Self { repository, clock }
    }
}

impl<Repository, ClockSource> DeleteWorkflowRunHandler<Repository, ClockSource>
where
    Repository: WorkflowRunRepository + Send + Sync + 'static,
    ClockSource: Clock,
{
    /// Soft-deletes one run after refusing active runs.
    ///
    /// Physical Git cleanup is not invoked here: the repository cascade
    /// registers a durable cleanup job for the run-task's worktree and branch
    /// in the same transaction, and the backend cleanup worker executes it.
    /// This keeps workflow-run deletion semantics identical to task/project
    /// deletion — the commit is the success, cleanup is asynchronous and
    /// replayable.
    pub fn handle(
        &self,
        request: DeleteWorkflowRunRequest,
    ) -> Result<DeleteWorkflowRunResponse, ApplicationError> {
        let run_id = WorkflowRunId::new(request.run_id);
        let deleted = self
            .repository
            .soft_delete_run(&run_id, self.clock.now_timestamp_millis())
            .map_err(ApplicationError::from_workflow_run_repository_error)?;
        match deleted {
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
