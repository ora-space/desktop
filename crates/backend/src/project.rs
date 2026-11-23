use crate::clock::SystemClock;
use crate::effect_worker::EffectWorkerHandle;
use crate::git_cleanup::GitCleanupHandle;
use crate::repository_work::spawn_repository_work;
use std::path::{Component, Path, PathBuf};
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
    /// session start, and Effect pass in that project. The check is the same probe behind
    /// `listWorkspaces`' `directory` and cwd resolution, so a relative root is judged against the
    /// base it will be resolved against afterwards and an accepted root starts out `present`.
    ///
    /// The new main Workspace needs the Effect Targets every running Consumer already declared,
    /// and no declaration will fire again on its own. Waking here is a latency optimization only:
    /// the worker converges the same Workspace within one scan interval regardless, so a wake lost
    /// to a crash costs a scan interval rather than the materialization.
    pub fn create(
        &self,
        request: CreateProjectRequest,
    ) -> Result<CreateProjectResponse, BackendError> {
        let root = PathBuf::from(&request.main_workspace_path);
        // A root naming no folder ("", ".", "./", "..", "/") or a relative one that anchors back
        // to the base would pass the directory check while silently rooting the project at the
        // base (Ora's start directory in production) or above it. An absolute root equal to the
        // base is an explicit choice, such as starting Ora inside the repository and picking it.
        let names_a_folder = !request.main_workspace_path.trim().is_empty()
            && root
                .components()
                .any(|component| matches!(component, Component::Normal(_)));
        let accepted = names_a_folder
            && match crate::workspace::probe_local_directory(root.clone(), &self.relative_path_base)
            {
                crate::workspace::LocalDirectory::Present(resolved) => {
                    root.is_absolute()
                        || std::fs::canonicalize(resolved).ok()
                            != std::fs::canonicalize(&self.relative_path_base).ok()
                }
                crate::workspace::LocalDirectory::Missing(_) => false,
            };
        if !accepted {
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
    use ora_application::ProjectRepository;
    use ora_contracts::{
        CreateProjectRequest, EmptyErrorParams, ListProjectsRequest, ListWorkspacesRequest,
        PublicError, Workspace, WorkspaceDirectory, WorkspaceKind, WorkspaceLifecycle,
    };
    use ora_db::{
        DatabaseBootstrapper, DatabaseLocation, MigrationCatalog, SqliteProjectRepository,
        default_migration_catalog,
    };
    use ora_domain::{AuditFields, Project, ProjectId, WorkspaceLocation};
    use pretty_assertions::assert_eq;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    /// Writes the rows an older build stored for a project created while its folder was absent.
    ///
    /// Those builds probed the folder once at creation and kept the local main Workspace in
    /// `provisioning` / `pending` when it was missing. The database is left at the last schema
    /// before migration `0024`, so opening a backend on it runs the real upgrade.
    fn seed_project_created_before_its_folder(database_path: &Path, folder: &Path) {
        let latest = default_migration_catalog().expect("create migration catalog");
        let previous = MigrationCatalog::new(
            latest
                .target_versions()
                .iter()
                .filter(|version| **version < "0024")
                .map(|version| {
                    latest
                        .migration(version)
                        .expect("catalog migration")
                        .clone()
                })
                .collect(),
        )
        .expect("create pre-0024 catalog");
        let pool = DatabaseBootstrapper::new(crate::test_clock::TestClock)
            .bootstrap_repository_pool(&DatabaseLocation::path(database_path), &previous)
            .expect("bootstrap pre-0024 database");
        SqliteProjectRepository::with_clock(pool, crate::test_clock::TestClock)
            .create_project(
                Project::new(
                    ProjectId::new("stranded"),
                    "Stranded",
                    AuditFields::new(1, 1, false),
                ),
                WorkspaceLocation::local_filesystem(folder.to_string_lossy()),
            )
            .expect("persist project");
        rusqlite::Connection::open(database_path)
            .expect("open seeded database")
            .execute_batch(
                "UPDATE workspaces SET lifecycle = 'provisioning' WHERE project_id = 'stranded';
                 UPDATE workspace_provisioning SET state = 'pending'
                 WHERE workspace_id IN (SELECT id FROM workspaces WHERE project_id = 'stranded');",
            )
            .expect("store the pre-fix creation state");
    }

    /// Lists the only workspace a test backend holds.
    fn only_workspace(backend: &Backend) -> Workspace {
        let mut workspaces = backend
            .workspaces()
            .list(ListWorkspacesRequest {})
            .expect("list workspaces")
            .workspaces;
        assert_eq!(workspaces.len(), 1);
        workspaces.remove(0)
    }

    /// Rejects every root that is not an existing directory and persists nothing for it.
    #[test]
    fn rejects_a_main_workspace_that_is_not_an_existing_directory() {
        ora_logging::with_trace_logging(|| {
            let temporary = TempDir::new().expect("create temporary backend directory");
            let backend = Backend::open(backend_paths(temporary.path(), temporary.path()))
                .expect("open shared backend");
            let file = temporary.path().join("README.md");
            std::fs::write(&file, "not a directory").expect("write file fixture");
            std::fs::create_dir(temporary.path().join("nested")).expect("create nested folder");
            let rejected = [
                temporary
                    .path()
                    .join("missing")
                    .to_string_lossy()
                    .into_owned(),
                file.to_string_lossy().into_owned(),
                String::new(),
                "   ".to_string(),
                // Each of these anchors to the relative path base or above it.
                ".".to_string(),
                "./".to_string(),
                "..".to_string(),
                PathBuf::from("nested")
                    .join("..")
                    .to_string_lossy()
                    .into_owned(),
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

    /// Accepts an absolute root equal to the relative path base: only relative anchoring is suspect.
    ///
    /// In production the base is Ora's start directory, so starting Ora inside a repository and
    /// picking that repository yields exactly this root.
    #[test]
    fn accepts_an_absolute_main_workspace_equal_to_the_relative_path_base() {
        ora_logging::with_trace_logging(|| {
            let temporary = TempDir::new().expect("create temporary backend directory");
            let backend = Backend::open(backend_paths(temporary.path(), temporary.path()))
                .expect("open shared backend");

            backend
                .projects()
                .create(CreateProjectRequest {
                    name: "Base".to_string(),
                    main_workspace_path: temporary.path().to_string_lossy().into_owned(),
                })
                .expect("create project at the base chosen explicitly");
            let workspace_id = only_workspace(&backend).id;

            assert_eq!(
                backend
                    .workspaces()
                    .resolve_workspace_cwd(&workspace_id)
                    .expect("resolve the base root"),
                temporary.path()
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

    /// Lets `directory` alone decide a project an older build created while its folder was absent.
    ///
    /// That build stranded the Workspace in `provisioning`: `listWorkspaces` reported the folder
    /// `present` once it appeared, yet every session start and model listing still failed with
    /// `workspace_unavailable`, so the client showed no reason. After the upgrade the client's
    /// `directory: missing` gate (sidebar, banner, send, model selector) covers the absent folder,
    /// and creating the folder makes the project usable with no other step.
    #[test]
    fn upgrade_lets_the_folder_decide_a_project_created_before_its_folder() {
        ora_logging::with_trace_logging(|| {
            let temporary = TempDir::new().expect("create temporary backend directory");
            let paths = backend_paths(temporary.path(), temporary.path());
            let folder = temporary.path().join("repository");
            seed_project_created_before_its_folder(
                &paths.app_data_directory.join("ora.sqlite3"),
                &folder,
            );
            let backend = Backend::open(paths).expect("open upgraded backend");

            let workspace = only_workspace(&backend);
            let expected = |directory| Workspace {
                id: workspace.id.clone(),
                project_id: "stranded".to_string(),
                kind: WorkspaceKind::Main,
                lifecycle: WorkspaceLifecycle::Active,
                directory,
            };
            assert_eq!(
                workspace,
                expected(WorkspaceDirectory::Missing {
                    path: folder.to_string_lossy().into_owned(),
                })
            );
            assert_eq!(
                backend
                    .workspaces()
                    .resolve_workspace_cwd(&workspace.id)
                    .expect_err("an absent folder is unavailable")
                    .public_error()
                    .clone(),
                PublicError::WorkspaceUnavailable(EmptyErrorParams {})
            );

            std::fs::create_dir(&folder).expect("create the project folder");
            assert_eq!(
                only_workspace(&backend),
                expected(WorkspaceDirectory::Present)
            );
            assert_eq!(
                backend
                    .workspaces()
                    .resolve_workspace_cwd(&workspace.id)
                    .expect("the folder alone makes the project usable"),
                folder
            );
        });
    }
}
