use crate::clock::SystemClock;
use crate::effect_worker::EffectWorkerHandle;
use crate::git_cleanup::GitCleanupHandle;
use crate::repository_work::spawn_repository_work;
use std::path::PathBuf;
use std::sync::Arc;

use ora_application::{
    BranchLister, BranchListingError, BranchReference, Clock, CreateProjectHandler,
    GetProjectHandler, ListProjectBranchesHandler, ListProjectsHandler, UpdateProjectHandler,
    UuidProjectIdGenerator,
};
use ora_contracts::{
    CreateProjectRequest, CreateProjectResponse, DeleteProjectRequest, DeleteProjectResponse,
    GetProjectRequest, GetProjectResponse, ListProjectBranchesRequest, ListProjectBranchesResponse,
    ListProjectsRequest, ListProjectsResponse, UpdateProjectRequest, UpdateProjectResponse,
};
use ora_db::{
    CascadeDeleteOutcome, RepositoryPool, SqliteCascadeRepository, SqliteProjectRepository,
    SqliteTaskRepository, SqliteWorktreeRepository,
};
use ora_domain::ProjectId;
use std::path::Path;

use crate::{BackendError, ErrorClassification};
use gitlancer::git::base_branch::{ListWorktreeBasesRequest, ListWorktreeBasesResponse};
use gitlancer::{CliGitRunner, Git, RepoRoot};
use ora_contracts::{EmptyErrorParams, PublicError};

type ProjectBranchListHandler = ListProjectBranchesHandler<
    SqliteProjectRepository,
    ora_db::SqliteWorkspaceRepository,
    SqliteTaskRepository,
    SqliteWorktreeRepository,
    GitBranchLister,
>;

/// Groups the concrete project handlers shared by runtime adapters.
pub struct ProjectApi {
    pool: RepositoryPool,
    /// Where cascaded sessions' recorded conversations are removed from.
    sessions_root: PathBuf,
    /// Anchors relative main Workspace roots exactly as later cwd resolution will.
    relative_path_base: PathBuf,
    create: CreateProjectHandler<SqliteProjectRepository, UuidProjectIdGenerator, SystemClock>,
    get: GetProjectHandler<SqliteProjectRepository>,
    list: ListProjectsHandler<SqliteProjectRepository>,
    list_branches: ProjectBranchListHandler,
    update: UpdateProjectHandler<SqliteProjectRepository, SystemClock>,
    clock: SystemClock,
    /// Wakes Effect convergence for the Workspace a new project brings with it.
    effect_reconcile: EffectWorkerHandle,
    git_cleanup: GitCleanupHandle,
}

impl ProjectApi {
    /// Builds project handlers from the shared repository pool.
    pub(crate) fn new(
        pool: RepositoryPool,
        sessions_root: PathBuf,
        relative_path_base: PathBuf,
        clock: SystemClock,
        effect_reconcile: EffectWorkerHandle,
        git_cleanup: GitCleanupHandle,
    ) -> Self {
        let repository = SqliteProjectRepository::new(pool.clone());

        Self {
            pool: pool.clone(),
            sessions_root,
            relative_path_base,
            effect_reconcile,
            git_cleanup,
            create: CreateProjectHandler::new(
                repository.clone(),
                UuidProjectIdGenerator::new(),
                clock,
            ),
            get: GetProjectHandler::new(repository.clone()),
            list: ListProjectsHandler::new(repository.clone()),
            list_branches: ListProjectBranchesHandler::new(
                repository.clone(),
                ora_db::SqliteWorkspaceRepository::new(pool.clone()),
                SqliteTaskRepository::new(pool.clone()),
                SqliteWorktreeRepository::new(pool),
                GitBranchLister,
            ),
            update: UpdateProjectHandler::new(repository, clock),
            clock,
        }
    }

    /// Executes project creation through the application handler.
    ///
    /// The main Workspace root must already be a directory: nothing provisions a local checkout
    /// later, so accepting a missing one would only defer the failure to every model listing,
    /// session start, and Effect pass in that project. The check reuses the cwd resolver so a
    /// relative root is judged against the same base it will be resolved against afterwards.
    ///
    /// The new main Workspace needs the Effect Targets every running Consumer already declared,
    /// and no declaration will fire again on its own. Waking here is a latency optimization only:
    /// the worker converges the same Workspace within one scan interval regardless, so a wake lost
    /// to a crash costs a scan interval rather than the materialization.
    pub fn create(
        &self,
        request: CreateProjectRequest,
    ) -> Result<CreateProjectResponse, BackendError> {
        // A blank root joins to the base directory itself and would pass the directory check.
        if request.main_workspace_path.trim().is_empty()
            || crate::task::absolute_project_root(
                PathBuf::from(&request.main_workspace_path),
                &self.relative_path_base,
            )
            .is_err()
        {
            return Err(BackendError::new(
                ErrorClassification::InvalidRequest,
                PublicError::ProjectWorkspaceNotDirectory(EmptyErrorParams {}),
                "project main workspace must be an existing directory",
            ));
        }
        let response = self.create.handle(request)?;
        self.effect_reconcile.notify();
        Ok(response)
    }

    /// Executes one project lookup through the application handler.
    pub fn get(&self, request: GetProjectRequest) -> Result<GetProjectResponse, BackendError> {
        self.get.handle(request).map_err(BackendError::from)
    }

    /// Executes project listing through the application handler.
    pub fn list(&self, request: ListProjectsRequest) -> Result<ListProjectsResponse, BackendError> {
        self.list.handle(request).map_err(BackendError::from)
    }

    /// Executes project branch listing through the application handler.
    pub fn list_branches(
        &self,
        request: ListProjectBranchesRequest,
    ) -> Result<ListProjectBranchesResponse, BackendError> {
        self.list_branches
            .handle(request)
            .map_err(BackendError::from)
    }

    /// Executes project replacement through the application handler.
    pub fn update(
        &self,
        request: UpdateProjectRequest,
    ) -> Result<UpdateProjectResponse, BackendError> {
        self.update.handle(request).map_err(BackendError::from)
    }

    /// Runs the whole cascade off the async worker and wakes cleanup only after it commits.
    pub async fn delete(
        self: &Arc<Self>,
        request: DeleteProjectRequest,
    ) -> Result<DeleteProjectResponse, BackendError> {
        let project = self.clone();
        let response = spawn_repository_work(move || project.delete_persisted(request)).await?;
        self.git_cleanup.notify();
        Ok(response)
    }

    /// Keeps database cascades, active-session refusal, and history cleanup in one owned use case.
    fn delete_persisted(
        &self,
        request: DeleteProjectRequest,
    ) -> Result<DeleteProjectResponse, BackendError> {
        let project_id = ProjectId::new(request.project_id);
        // Collected before the cascade: once the rows are soft-deleted nothing
        // links their history files back to the tasks that owned them.
        let session_ids = crate::session_history::session_ids_for_project(&self.pool, &project_id);
        let outcome = SqliteCascadeRepository::new(self.pool.clone())
            .delete_project(&project_id, self.clock.now_timestamp_millis())
            .map_err(|source| {
                BackendError::internal("project repository operation failed", source)
            })?;

        match outcome {
            CascadeDeleteOutcome::Deleted => {
                crate::session_history::remove_session_histories(&self.sessions_root, session_ids);
                Ok(DeleteProjectResponse {
                    project_id: project_id.to_string(),
                })
            }
            CascadeDeleteOutcome::NotFound => Err(BackendError::new(
                ErrorClassification::NotFound,
                PublicError::ProjectNotFound(EmptyErrorParams {}),
                format!("project not found: {project_id}"),
            )),
            CascadeDeleteOutcome::ActiveSession => Err(BackendError::new(
                ErrorClassification::Conflict,
                PublicError::ResourceInUse(EmptyErrorParams {}),
                "project has a running session and cannot be deleted",
            )),
        }
    }
}

/// Adapts Gitlancer's local worktree bases to the application branch-listing port.
#[derive(Debug, Clone, Copy)]
struct GitBranchLister;

impl BranchLister for GitBranchLister {
    /// Discovers the repository and returns logical names paired with resolvable refs.
    fn list_branches(
        &self,
        repository_root: &Path,
    ) -> Result<Vec<BranchReference>, BranchListingError> {
        let git = Git::new(CliGitRunner);
        let repository = git
            .discover_repository(RepoRoot::new(repository_root))
            .map_err(|_| BranchListingError::NotARepository)?;
        let ListWorktreeBasesResponse { bases } = git
            .list_worktree_bases(ListWorktreeBasesRequest {
                repository: &repository,
            })
            .map_err(BranchListingError::operation_failed)?;

        Ok(bases
            .into_iter()
            .map(|base| BranchReference {
                name: base.branch_name().as_str().to_string(),
                ref_name: base.reference_name(),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use crate::test_backend::backend_paths;
    use crate::{Backend, ErrorClassification};
    use ora_contracts::{
        CreateProjectRequest, EmptyErrorParams, ListProjectsRequest, ListWorkspacesRequest,
        PublicError,
    };
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Rejects every root that is not an existing directory and persists nothing for it.
    #[test]
    fn rejects_a_main_workspace_that_is_not_an_existing_directory() {
        ora_logging::with_trace_logging(|| {
            let temporary = TempDir::new().expect("create temporary backend directory");
            let backend = Backend::open(backend_paths(temporary.path(), temporary.path()))
                .expect("open shared backend");
            let file = temporary.path().join("README.md");
            std::fs::write(&file, "not a directory").expect("write file fixture");
            let rejected = [
                temporary
                    .path()
                    .join("missing")
                    .to_string_lossy()
                    .into_owned(),
                file.to_string_lossy().into_owned(),
                String::new(),
                "   ".to_string(),
            ];

            for main_workspace_path in rejected {
                let error = backend
                    .projects()
                    .create(CreateProjectRequest {
                        name: "Missing".to_string(),
                        main_workspace_path: main_workspace_path.clone(),
                    })
                    .expect_err("a root that is not a directory is rejected");
                assert_eq!(
                    (error.classification(), error.public_error().clone()),
                    (
                        ErrorClassification::InvalidRequest,
                        PublicError::ProjectWorkspaceNotDirectory(EmptyErrorParams {}),
                    ),
                    "{main_workspace_path:?}"
                );
            }
            assert_eq!(
                backend
                    .projects()
                    .list(ListProjectsRequest {})
                    .expect("list projects")
                    .projects,
                Vec::new()
            );
        });
    }

    /// Judges a relative root against the base later cwd resolution anchors it to.
    #[test]
    fn accepts_a_relative_main_workspace_under_the_relative_path_base() {
        ora_logging::with_trace_logging(|| {
            let temporary = TempDir::new().expect("create temporary backend directory");
            let relative_root = PathBuf::from("nested").join("repository");
            std::fs::create_dir_all(temporary.path().join(&relative_root))
                .expect("create relative root");
            let backend = Backend::open(backend_paths(temporary.path(), temporary.path()))
                .expect("open shared backend");

            backend
                .projects()
                .create(CreateProjectRequest {
                    name: "Relative".to_string(),
                    main_workspace_path: relative_root.to_string_lossy().into_owned(),
                })
                .expect("create project from a relative root");
            let workspace_id = backend
                .workspaces()
                .list(ListWorkspacesRequest {})
                .expect("list workspaces")
                .workspaces
                .remove(0)
                .id;

            assert_eq!(
                backend
                    .workspaces()
                    .resolve_workspace_cwd(&workspace_id)
                    .expect("resolve relative root"),
                temporary.path().join(relative_root)
            );
        });
    }

    /// Tracks the directory on every use, so a checkout that disappears and returns recovers.
    ///
    /// The lifecycle used to freeze a single probe taken at creation, which stranded a project
    /// forever if its directory was briefly unavailable.
    #[test]
    fn main_workspace_availability_follows_its_directory() {
        ora_logging::with_trace_logging(|| {
            let temporary = TempDir::new().expect("create temporary backend directory");
            let repository = temporary.path().join("repository");
            let moved = temporary.path().join("moved");
            std::fs::create_dir(&repository).expect("create repository root");
            let backend = Backend::open(backend_paths(temporary.path(), temporary.path()))
                .expect("open shared backend");
            backend
                .projects()
                .create(CreateProjectRequest {
                    name: "Ora".to_string(),
                    main_workspace_path: repository.to_string_lossy().into_owned(),
                })
                .expect("create project");
            let workspace_id = backend
                .workspaces()
                .list(ListWorkspacesRequest {})
                .expect("list workspaces")
                .workspaces
                .remove(0)
                .id;

            std::fs::rename(&repository, &moved).expect("move repository away");
            let error = backend
                .workspaces()
                .resolve_workspace_cwd(&workspace_id)
                .expect_err("a missing checkout is unavailable");
            assert_eq!(
                error.public_error().clone(),
                PublicError::WorkspaceUnavailable(EmptyErrorParams {})
            );

            std::fs::rename(&moved, &repository).expect("restore repository");
            assert_eq!(
                backend
                    .workspaces()
                    .resolve_workspace_cwd(&workspace_id)
                    .expect("a restored checkout is available again"),
                repository
            );
        });
    }
}
