use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::task::{
    CreateTaskWorktreeRequest, DeleteTaskWorktreeRequest, TaskIdGenerator,
    TaskWorktreeDeletionMode, TaskWorktreeProvisioner,
};
use crate::workflow::WorkflowRepository;
use crate::workflow_run::mapper::{map_node_run, map_run, map_run_summary};
use crate::workflow_run::{DeleteWorkflowRunResult, WorkflowRunIdGenerator, WorkflowRunRepository};
use crate::worktree::WorktreeIdGenerator;
use crate::{ApplicationError, Clock};
use ora_contracts::{
    CreateWorkflowRunRequest, CreateWorkflowRunResponse, DeleteWorkflowRunRequest,
    DeleteWorkflowRunResponse, GetWorkflowRunRequest, GetWorkflowRunResponse,
    ListWorkflowNodeRunsRequest, ListWorkflowNodeRunsResponse, ListWorkflowRunsByWorkflowRequest,
    ListWorkflowRunsByWorkflowResponse, ListWorkflowRunsRequest, ListWorkflowRunsResponse,
};
use ora_domain::{
    AuditFields, ProjectId, Task, TaskId, TaskStatus, Workflow, WorkflowId, WorkflowRun,
    WorkflowRunId, WorkflowRunStatus, WorkflowSnapshot, WorkflowSnapshotId, Worktree,
    WorktreeActivity, WorktreeBaseline,
};

const DRAFT_VERSION: &str = "draft";
const DEFAULT_RUN_BASE_REFERENCE: &str = "main";
const TASK_BRANCH_PREFIX_LEN: usize = 8;

/// Handles creation of a workflow run against a published snapshot with a dedicated worktree.
pub struct CreateWorkflowRunHandler<
    WorkflowRepositoryPort,
    RunRepositoryPort,
    RunIdGenerator,
    TaskIdGeneratorPort,
    WorktreeIdGeneratorPort,
    WorktreeProvisioner,
    ClockSource,
> {
    workflow_repository: Arc<WorkflowRepositoryPort>,
    run_repository: Arc<RunRepositoryPort>,
    run_id_generator: RunIdGenerator,
    task_id_generator: TaskIdGeneratorPort,
    worktree_id_generator: WorktreeIdGeneratorPort,
    worktree_provisioner: WorktreeProvisioner,
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
    ClockSource,
>
    CreateWorkflowRunHandler<
        WorkflowRepositoryPort,
        RunRepositoryPort,
        RunIdGenerator,
        TaskIdGeneratorPort,
        WorktreeIdGeneratorPort,
        WorktreeProvisioner,
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
    ClockSource,
>
    CreateWorkflowRunHandler<
        WorkflowRepositoryPort,
        RunRepositoryPort,
        RunIdGenerator,
        TaskIdGeneratorPort,
        WorktreeIdGeneratorPort,
        WorktreeProvisioner,
        ClockSource,
    >
where
    WorkflowRepositoryPort: WorkflowRepository + Send + Sync + 'static,
    RunRepositoryPort: WorkflowRunRepository + Send + Sync + 'static,
    RunIdGenerator: WorkflowRunIdGenerator,
    TaskIdGeneratorPort: TaskIdGenerator,
    WorktreeIdGeneratorPort: WorktreeIdGenerator,
    WorktreeProvisioner: TaskWorktreeProvisioner,
    ClockSource: Clock,
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
        let provisioned = self
            .worktree_provisioner
            .create_task_worktree(CreateTaskWorktreeRequest {
                branch_name: branch_name.clone(),
                base_reference_name: base_reference_name.to_string(),
                worktree_path,
            })
            .map_err(ApplicationError::from_task_worktree_provisioner_error)?;

        let worktree = Worktree::new(
            worktree_id.clone(),
            task_id.clone(),
            Some(branch_name.clone()),
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
            project_id,
            title,
            TaskStatus::Todo,
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
            .create_run(run, task, worktree)
            .map_err(|error| {
                self.compensate_provisioned_worktree(
                    &branch_name,
                    ApplicationError::from_workflow_run_repository_error(error),
                )
            })?;

        Ok(CreateWorkflowRunResponse {
            run: map_run(created),
            task_id: task_id.to_string(),
        })
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

    /// Deletes the provisioned physical worktree when persistence fails so no orphan remains.
    fn compensate_provisioned_worktree(
        &self,
        branch_name: &str,
        original_error: ApplicationError,
    ) -> ApplicationError {
        let cleanup = self
            .worktree_provisioner
            .delete_task_worktree(DeleteTaskWorktreeRequest {
                branch_name: branch_name.to_string(),
                mode: TaskWorktreeDeletionMode::Force,
            });
        match cleanup {
            Ok(()) => original_error,
            Err(cleanup_error) => {
                ApplicationError::from_task_worktree_provisioner_error(cleanup_error)
            }
        }
    }
}

/// Derives the stable task branch name from the first eight characters of the generated task id.
fn branch_name_for_task(task_id: &TaskId) -> String {
    format!("ora/{}", task_branch_prefix(task_id))
}

/// Derives the short branch prefix used to keep task branch names readable.
fn task_branch_prefix(task_id: &TaskId) -> String {
    task_id
        .to_string()
        .chars()
        .take(TASK_BRANCH_PREFIX_LEN)
        .collect()
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

/// Handles soft-deletion of a workflow run followed by physical worktree cleanup.
pub struct DeleteWorkflowRunHandler<Repository, WorktreeProvisioner, ClockSource> {
    repository: Arc<Repository>,
    worktree_provisioner: WorktreeProvisioner,
    clock: ClockSource,
}

impl<Repository, WorktreeProvisioner, ClockSource>
    DeleteWorkflowRunHandler<Repository, WorktreeProvisioner, ClockSource>
{
    /// Builds a delete-run handler with the provisioner used for physical worktree cleanup.
    pub fn new(
        repository: Arc<Repository>,
        worktree_provisioner: WorktreeProvisioner,
        clock: ClockSource,
    ) -> Self {
        Self {
            repository,
            worktree_provisioner,
            clock,
        }
    }
}

impl<Repository, WorktreeProvisioner, ClockSource>
    DeleteWorkflowRunHandler<Repository, WorktreeProvisioner, ClockSource>
where
    Repository: WorkflowRunRepository + Send + Sync + 'static,
    WorktreeProvisioner: TaskWorktreeProvisioner,
    ClockSource: Clock,
{
    /// Soft-deletes one run and removes its physical worktree after refusing active runs.
    pub fn handle(
        &self,
        request: DeleteWorkflowRunRequest,
    ) -> Result<DeleteWorkflowRunResponse, ApplicationError> {
        let run_id = WorkflowRunId::new(request.run_id);
        // Capture the branch before the cascade hides the task row it belongs to.
        let task_id = self
            .repository
            .find_run_task_id(&run_id)
            .map_err(ApplicationError::from_workflow_run_repository_error)?;

        let deleted = self
            .repository
            .soft_delete_run(&run_id, self.clock.now_timestamp_millis())
            .map_err(ApplicationError::from_workflow_run_repository_error)?;
        match deleted {
            DeleteWorkflowRunResult::Deleted => {}
            DeleteWorkflowRunResult::NotFound => {
                return Err(ApplicationError::WorkflowRunNotFound {
                    run_id: run_id.to_string(),
                });
            }
            DeleteWorkflowRunResult::ActiveRun => {
                return Err(ApplicationError::WorkflowRunActive);
            }
        }

        if let Some(task_id) = task_id {
            self.worktree_provisioner
                .delete_task_worktree(DeleteTaskWorktreeRequest {
                    branch_name: branch_name_for_task(&task_id),
                    mode: TaskWorktreeDeletionMode::Force,
                })
                .map_err(ApplicationError::from_task_worktree_provisioner_error)?;
        }

        Ok(DeleteWorkflowRunResponse {
            run_id: run_id.to_string(),
        })
    }
}
