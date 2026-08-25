use std::collections::HashMap;
use std::path::Path;

use ora_contracts::{ListProjectBranchesRequest, ListProjectBranchesResponse, ProjectBranch};
use ora_domain::{ProjectId, WorkspaceLocation};

use crate::project::ports::{BranchLister, ProjectRepository};
use crate::{ApplicationError, TaskRepository, WorkspaceRepository, WorktreeRepository};

/// Lists project branches while applying Ora-owned display-name rules.
pub struct ListProjectBranchesHandler<
    ProjectRepositoryPort,
    WorkspaceRepositoryPort,
    TaskRepositoryPort,
    WorktreeRepositoryPort,
    BranchListerPort,
> {
    project_repository: ProjectRepositoryPort,
    workspace_repository: WorkspaceRepositoryPort,
    task_repository: TaskRepositoryPort,
    worktree_repository: WorktreeRepositoryPort,
    branch_lister: BranchListerPort,
}

impl<
    ProjectRepositoryPort,
    WorkspaceRepositoryPort,
    TaskRepositoryPort,
    WorktreeRepositoryPort,
    BranchListerPort,
>
    ListProjectBranchesHandler<
        ProjectRepositoryPort,
        WorkspaceRepositoryPort,
        TaskRepositoryPort,
        WorktreeRepositoryPort,
        BranchListerPort,
    >
{
    /// Builds the branch-list use case from persistence and Git-facing ports.
    pub fn new(
        project_repository: ProjectRepositoryPort,
        workspace_repository: WorkspaceRepositoryPort,
        task_repository: TaskRepositoryPort,
        worktree_repository: WorktreeRepositoryPort,
        branch_lister: BranchListerPort,
    ) -> Self {
        Self {
            project_repository,
            workspace_repository,
            task_repository,
            worktree_repository,
            branch_lister,
        }
    }
}

impl<
    ProjectRepositoryPort,
    WorkspaceRepositoryPort,
    TaskRepositoryPort,
    WorktreeRepositoryPort,
    BranchListerPort,
>
    ListProjectBranchesHandler<
        ProjectRepositoryPort,
        WorkspaceRepositoryPort,
        TaskRepositoryPort,
        WorktreeRepositoryPort,
        BranchListerPort,
    >
where
    ProjectRepositoryPort: ProjectRepository,
    WorkspaceRepositoryPort: WorkspaceRepository,
    TaskRepositoryPort: TaskRepository,
    WorktreeRepositoryPort: WorktreeRepository,
    BranchListerPort: BranchLister,
{
    /// Lists local branch refs and replaces Ora-managed branch labels with task titles.
    pub fn handle(
        &self,
        request: ListProjectBranchesRequest,
    ) -> Result<ListProjectBranchesResponse, ApplicationError> {
        let project_id = ProjectId::new(request.project_id);
        let _project = self
            .project_repository
            .find_project(&project_id)
            .map_err(ApplicationError::from_project_repository_error)?
            .ok_or_else(|| ApplicationError::ProjectNotFound {
                project_id: project_id.to_string(),
            })?;
        let workspace = self
            .workspace_repository
            .find_main_workspace(&project_id)
            .map_err(ApplicationError::from_workflow_run_repository_error)?
            .ok_or_else(|| ApplicationError::ProjectNotFound {
                project_id: project_id.to_string(),
            })?;
        let WorkspaceLocation::LocalFilesystem { path } = workspace.location else {
            return Err(ApplicationError::TaskWorktreeRequiresGitRepository);
        };
        let branches = self
            .branch_lister
            .list_branches(Path::new(&path))
            .map_err(ApplicationError::from_branch_listing_error)?;
        let task_titles = self
            .task_repository
            .list_tasks()
            .map_err(ApplicationError::from_task_repository_error)?
            .into_iter()
            .filter(|task| task.project_id == project_id)
            .map(|task| (task.workspace_id, task.title))
            .collect::<HashMap<_, _>>();
        let managed_branch_titles = self
            .worktree_repository
            .list_worktrees()
            .map_err(ApplicationError::from_worktree_repository_error)?
            .into_iter()
            .filter_map(|worktree| {
                Some((
                    worktree.branch_name?,
                    task_titles.get(&worktree.workspace_id)?.clone(),
                ))
            })
            .collect::<HashMap<_, _>>();
        let branches = branches
            .into_iter()
            .map(|branch| {
                let display_name = managed_branch_titles
                    .get(&branch.name)
                    .cloned()
                    .unwrap_or_else(|| branch.name.clone());
                ProjectBranch {
                    name: branch.name,
                    ref_name: branch.ref_name,
                    display_name,
                }
            })
            .collect::<Vec<_>>();

        Ok(ListProjectBranchesResponse { branches })
    }
}
