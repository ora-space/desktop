use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use ora_contracts::{ListProjectBranchesRequest, ListProjectBranchesResponse, ProjectBranch};
use ora_domain::{
    AuditFields, Project, ProjectId, Task, TaskId, TaskStatus, Worktree, WorktreeActivity,
    WorktreeId,
};
use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;

use crate::{
    ApplicationError, BranchLister, BranchListingError, BranchReference,
    ListProjectBranchesHandler, ProjectRepository, ProjectRepositoryError, TaskRepository,
    TaskRepositoryError, WorktreeRepository, WorktreeRepositoryError,
};

/// Verifies branch labels are derived from project-owned tasks without leaking persistence into adapters.
#[test]
fn maps_managed_branch_names_to_task_titles() {
    with_trace_logging(|| {
        let branch_lister = Rc::new(FakeBranchLister::succeed(vec![
            BranchReference {
                name: "main".to_string(),
                ref_name: "upstream/main".to_string(),
            },
            BranchReference {
                name: "ora/12345678".to_string(),
                ref_name: "ora/12345678".to_string(),
            },
            BranchReference {
                name: "feature/unmanaged".to_string(),
                ref_name: "upstream/feature/unmanaged".to_string(),
            },
        ]));
        let handler = ListProjectBranchesHandler::new(
            Rc::new(FakeProjectRepository::with_projects(
                vec![project_fixture()],
            )),
            Rc::new(FakeTaskRepository::with_tasks(vec![
                task_fixture("task-1", "project-1", "Review branch"),
                task_fixture("task-2", "project-2", "Another project"),
            ])),
            Rc::new(FakeWorktreeRepository::with_worktrees(vec![
                worktree_fixture("worktree-1", "task-1", Some("ora/12345678")),
                worktree_fixture("worktree-2", "task-2", Some("feature/unmanaged")),
                worktree_fixture("worktree-3", "task-1", None),
            ])),
            branch_lister.clone(),
        );

        let response = handler
            .handle(ListProjectBranchesRequest {
                project_id: "project-1".to_string(),
            })
            .unwrap_or_else(|error| panic!("list project branches failed: {error}"));

        assert_eq!(
            response,
            ListProjectBranchesResponse {
                branches: vec![
                    ProjectBranch {
                        name: "main".to_string(),
                        ref_name: "upstream/main".to_string(),
                        display_name: "main".to_string(),
                    },
                    ProjectBranch {
                        name: "ora/12345678".to_string(),
                        ref_name: "ora/12345678".to_string(),
                        display_name: "Review branch".to_string(),
                    },
                    ProjectBranch {
                        name: "feature/unmanaged".to_string(),
                        ref_name: "upstream/feature/unmanaged".to_string(),
                        display_name: "feature/unmanaged".to_string(),
                    },
                ],
            }
        );
        assert_eq!(
            branch_lister.requested_roots(),
            vec![PathBuf::from("/workspace/ora")]
        );
    });
}

/// Verifies missing projects are rejected before Git or cross-aggregate repositories are queried.
#[test]
fn rejects_missing_projects_before_listing_branches() {
    with_trace_logging(|| {
        let branch_lister = Rc::new(FakeBranchLister::succeed(Vec::new()));
        let handler = ListProjectBranchesHandler::new(
            Rc::new(FakeProjectRepository::with_projects(Vec::new())),
            Rc::new(FakeTaskRepository::with_tasks(Vec::new())),
            Rc::new(FakeWorktreeRepository::with_worktrees(Vec::new())),
            branch_lister.clone(),
        );

        let error = handler
            .handle(ListProjectBranchesRequest {
                project_id: "missing".to_string(),
            })
            .expect_err("missing project should be rejected");

        assert_eq!(
            error,
            ApplicationError::ProjectNotFound {
                project_id: "missing".to_string(),
            }
        );
        assert!(branch_lister.requested_roots().is_empty());
    });
}

/// Verifies Git-facing failures retain the public non-repository and internal-error semantics.
#[test]
fn normalizes_branch_lister_failures() {
    with_trace_logging(|| {
        let project_repository =
            Rc::new(FakeProjectRepository::with_projects(
                vec![project_fixture()],
            ));
        let task_repository = Rc::new(FakeTaskRepository::with_tasks(Vec::new()));
        let worktree_repository = Rc::new(FakeWorktreeRepository::with_worktrees(Vec::new()));
        let non_repository_error = ListProjectBranchesHandler::new(
            project_repository.clone(),
            task_repository.clone(),
            worktree_repository.clone(),
            Rc::new(FakeBranchLister::fail(BranchListingError::NotARepository)),
        )
        .handle(ListProjectBranchesRequest {
            project_id: "project-1".to_string(),
        })
        .expect_err("non-repository project root should be rejected");
        let listing_error = ListProjectBranchesHandler::new(
            project_repository,
            task_repository,
            worktree_repository,
            Rc::new(FakeBranchLister::fail(BranchListingError::OperationFailed(
                "fetch failed".to_string(),
            ))),
        )
        .handle(ListProjectBranchesRequest {
            project_id: "project-1".to_string(),
        })
        .expect_err("branch infrastructure failure should be normalized");

        assert_eq!(
            non_repository_error,
            ApplicationError::TaskWorktreeRequiresGitRepository
        );
        assert_eq!(
            listing_error,
            ApplicationError::ProjectBranchListing {
                message: "fetch failed".to_string(),
            }
        );
    });
}

/// Creates the project whose repository path should reach the branch lister.
fn project_fixture() -> Project {
    Project::new(
        ProjectId::new("project-1"),
        "Ora",
        "/workspace/ora",
        AuditFields::new(1, 1, false),
    )
}

/// Creates a visible task used by branch-title mapping tests.
fn task_fixture(task_id: &str, project_id: &str, title: &str) -> Task {
    Task::new(
        TaskId::new(task_id),
        ProjectId::new(project_id),
        title,
        TaskStatus::Todo,
        None,
        AuditFields::new(1, 1, false),
    )
}

/// Creates a visible worktree whose optional branch can be mapped to its owning task.
fn worktree_fixture(worktree_id: &str, task_id: &str, branch_name: Option<&str>) -> Worktree {
    Worktree::new(
        WorktreeId::new(worktree_id),
        TaskId::new(task_id),
        branch_name.map(str::to_string),
        WorktreeActivity::Active,
        AuditFields::new(1, 1, false),
    )
}

#[derive(Debug)]
struct FakeBranchLister {
    result: Result<Vec<BranchReference>, BranchListingError>,
    requested_roots: RefCell<Vec<PathBuf>>,
}

impl FakeBranchLister {
    /// Builds a branch lister that returns the supplied refs.
    fn succeed(branches: Vec<BranchReference>) -> Self {
        Self {
            result: Ok(branches),
            requested_roots: RefCell::new(Vec::new()),
        }
    }

    /// Builds a branch lister that returns one deterministic infrastructure failure.
    fn fail(error: BranchListingError) -> Self {
        Self {
            result: Err(error),
            requested_roots: RefCell::new(Vec::new()),
        }
    }

    /// Returns every repository root received by the fake.
    fn requested_roots(&self) -> Vec<PathBuf> {
        self.requested_roots.borrow().clone()
    }
}

impl BranchLister for Rc<FakeBranchLister> {
    fn list_branches(
        &self,
        repository_root: &Path,
    ) -> Result<Vec<BranchReference>, BranchListingError> {
        self.requested_roots
            .borrow_mut()
            .push(repository_root.to_path_buf());
        self.result.clone()
    }
}

#[derive(Debug)]
struct FakeProjectRepository {
    projects: Vec<Project>,
}

impl FakeProjectRepository {
    /// Builds a project repository seeded with visible project rows.
    fn with_projects(projects: Vec<Project>) -> Self {
        Self { projects }
    }
}

impl ProjectRepository for Rc<FakeProjectRepository> {
    fn create_project(&self, _project: Project) -> Result<Project, ProjectRepositoryError> {
        Err(ProjectRepositoryError::OperationFailed(
            "create is unsupported in branch-list tests".to_string(),
        ))
    }

    fn find_project(
        &self,
        project_id: &ProjectId,
    ) -> Result<Option<Project>, ProjectRepositoryError> {
        Ok(self
            .projects
            .iter()
            .find(|project| project.id == *project_id)
            .cloned())
    }

    fn find_project_by_name(
        &self,
        _project_name: &str,
    ) -> Result<Option<Project>, ProjectRepositoryError> {
        Err(ProjectRepositoryError::OperationFailed(
            "name lookup is unsupported in branch-list tests".to_string(),
        ))
    }

    fn list_projects(&self) -> Result<Vec<Project>, ProjectRepositoryError> {
        Err(ProjectRepositoryError::OperationFailed(
            "listing is unsupported in branch-list tests".to_string(),
        ))
    }

    fn update_project(&self, _project: Project) -> Result<Project, ProjectRepositoryError> {
        Err(ProjectRepositoryError::OperationFailed(
            "update is unsupported in branch-list tests".to_string(),
        ))
    }

    fn soft_delete_project(
        &self,
        _project_id: &ProjectId,
        _deleted_at: i64,
    ) -> Result<bool, ProjectRepositoryError> {
        Err(ProjectRepositoryError::OperationFailed(
            "delete is unsupported in branch-list tests".to_string(),
        ))
    }
}

#[derive(Debug)]
struct FakeTaskRepository {
    tasks: Vec<Task>,
}

impl FakeTaskRepository {
    /// Builds a task repository seeded with visible task rows.
    fn with_tasks(tasks: Vec<Task>) -> Self {
        Self { tasks }
    }
}

impl TaskRepository for Rc<FakeTaskRepository> {
    fn create_task(&self, _task: Task) -> Result<Task, TaskRepositoryError> {
        Err(TaskRepositoryError::OperationFailed(
            "create is unsupported in branch-list tests".to_string(),
        ))
    }

    fn find_task(&self, _task_id: &TaskId) -> Result<Option<Task>, TaskRepositoryError> {
        Err(TaskRepositoryError::OperationFailed(
            "lookup is unsupported in branch-list tests".to_string(),
        ))
    }

    fn list_tasks(&self) -> Result<Vec<Task>, TaskRepositoryError> {
        Ok(self.tasks.clone())
    }

    fn update_task(&self, _task: Task) -> Result<Task, TaskRepositoryError> {
        Err(TaskRepositoryError::OperationFailed(
            "update is unsupported in branch-list tests".to_string(),
        ))
    }

    fn soft_delete_task(
        &self,
        _task_id: &TaskId,
        _deleted_at: i64,
    ) -> Result<bool, TaskRepositoryError> {
        Err(TaskRepositoryError::OperationFailed(
            "delete is unsupported in branch-list tests".to_string(),
        ))
    }
}

#[derive(Debug)]
struct FakeWorktreeRepository {
    worktrees: Vec<Worktree>,
}

impl FakeWorktreeRepository {
    /// Builds a worktree repository seeded with visible worktree rows.
    fn with_worktrees(worktrees: Vec<Worktree>) -> Self {
        Self { worktrees }
    }
}

impl WorktreeRepository for Rc<FakeWorktreeRepository> {
    fn create_worktree(&self, _worktree: Worktree) -> Result<Worktree, WorktreeRepositoryError> {
        Err(WorktreeRepositoryError::OperationFailed(
            "create is unsupported in branch-list tests".to_string(),
        ))
    }

    fn find_worktree(
        &self,
        _worktree_id: &WorktreeId,
    ) -> Result<Option<Worktree>, WorktreeRepositoryError> {
        Err(WorktreeRepositoryError::OperationFailed(
            "lookup is unsupported in branch-list tests".to_string(),
        ))
    }

    fn list_worktrees(&self) -> Result<Vec<Worktree>, WorktreeRepositoryError> {
        Ok(self.worktrees.clone())
    }

    fn update_worktree(&self, _worktree: Worktree) -> Result<Worktree, WorktreeRepositoryError> {
        Err(WorktreeRepositoryError::OperationFailed(
            "update is unsupported in branch-list tests".to_string(),
        ))
    }

    fn soft_delete_worktree(
        &self,
        _worktree_id: &WorktreeId,
        _deleted_at: i64,
    ) -> Result<bool, WorktreeRepositoryError> {
        Err(WorktreeRepositoryError::OperationFailed(
            "delete is unsupported in branch-list tests".to_string(),
        ))
    }
}
