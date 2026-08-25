use crate::{
    ApplicationError, Clock, CreateProjectHandler, GetProjectHandler, ListProjectsHandler,
    ProjectIdGenerator, ProjectRepository, RepositoryError, UpdateProjectHandler,
};
use ora_contracts::{
    CreateProjectRequest, CreateProjectResponse, GetProjectRequest, GetProjectResponse,
    ListProjectsRequest, ListProjectsResponse, Project as ContractProject, UpdateProjectRequest,
    UpdateProjectResponse,
};
use ora_domain::{AuditFields, Project, ProjectId, WorkspaceLocation};
use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;
use std::cell::RefCell;
use std::rc::Rc;

/// Verifies create handlers build domain projects and return the shared contract response.
#[test]
fn creates_projects_with_generated_identity_and_clock_values() {
    with_trace_logging(|| {
        let repository = Rc::new(FakeProjectRepository::default());
        let handler = CreateProjectHandler::new(
            repository.clone(),
            FixedProjectIdGenerator::new("project-1"),
            FixedClock::new(1_700_000_000_000),
        );

        let response = match handler.handle(CreateProjectRequest {
            name: "Ora".to_string(),
            main_workspace_path: "/workspace/ora".to_string(),
        }) {
            Ok(response) => response,
            Err(error) => panic!("create handler failed: {error}"),
        };

        assert_eq!(
            response,
            CreateProjectResponse {
                project: ContractProject {
                    id: "project-1".to_string(),
                    name: "Ora".to_string(),
                },
            }
        );
        assert_eq!(
            repository.visible_projects(),
            vec![Project::new(
                ProjectId::new("project-1"),
                "Ora",
                AuditFields::new(1_700_000_000_000, 1_700_000_000_000, false),
            )]
        );
    });
}

/// Verifies get handlers return the shared contract projection for existing projects.
#[test]
fn gets_projects_by_identifier() {
    with_trace_logging(|| {
        let repository = Rc::new(FakeProjectRepository::with_projects(vec![Project::new(
            ProjectId::new("project-1"),
            "Ora",
            AuditFields::new(1, 2, false),
        )]));
        let handler = GetProjectHandler::new(repository);

        let response = match handler.handle(GetProjectRequest {
            project_id: "project-1".to_string(),
        }) {
            Ok(response) => response,
            Err(error) => panic!("get handler failed: {error}"),
        };

        assert_eq!(
            response,
            GetProjectResponse {
                project: ContractProject {
                    id: "project-1".to_string(),
                    name: "Ora".to_string(),
                },
            }
        );
    });
}

/// Verifies list handlers map every stored project into the shared contract payload.
#[test]
fn lists_visible_projects() {
    with_trace_logging(|| {
        let repository = Rc::new(FakeProjectRepository::with_projects(vec![
            Project::new(
                ProjectId::new("project-1"),
                "Ora",
                AuditFields::new(1, 2, false),
            ),
            Project::new(
                ProjectId::new("project-2"),
                "Ora Docs",
                AuditFields::new(3, 4, false),
            ),
        ]));
        let handler = ListProjectsHandler::new(repository);

        let response = match handler.handle(ListProjectsRequest {}) {
            Ok(response) => response,
            Err(error) => panic!("list handler failed: {error}"),
        };

        assert_eq!(
            response,
            ListProjectsResponse {
                projects: vec![
                    ContractProject {
                        id: "project-1".to_string(),
                        name: "Ora".to_string(),
                    },
                    ContractProject {
                        id: "project-2".to_string(),
                        name: "Ora Docs".to_string(),
                    },
                ],
            }
        );
    });
}

/// Verifies update handlers preserve created timestamps while refreshing mutable fields.
#[test]
fn updates_projects_with_refreshed_timestamps() {
    with_trace_logging(|| {
        let repository = Rc::new(FakeProjectRepository::with_projects(vec![Project::new(
            ProjectId::new("project-1"),
            "Ora",
            AuditFields::new(10, 20, false),
        )]));
        let handler = UpdateProjectHandler::new(repository.clone(), FixedClock::new(30));

        let response = match handler.handle(UpdateProjectRequest {
            project_id: "project-1".to_string(),
            name: "Ora Updated".to_string(),
        }) {
            Ok(response) => response,
            Err(error) => panic!("update handler failed: {error}"),
        };

        assert_eq!(
            response,
            UpdateProjectResponse {
                project: ContractProject {
                    id: "project-1".to_string(),
                    name: "Ora Updated".to_string(),
                },
            }
        );
        assert_eq!(
            repository.visible_projects(),
            vec![Project::new(
                ProjectId::new("project-1"),
                "Ora Updated",
                AuditFields::new(10, 30, false),
            )]
        );
    });
}

/// Verifies handlers expose stable application errors for missing projects and repository failures.
#[test]
fn reports_application_errors() {
    with_trace_logging(|| {
        let missing_repository = Rc::new(FakeProjectRepository::default());
        let get_handler = GetProjectHandler::new(missing_repository);
        let failing_repository = Rc::new(FakeProjectRepository::default());
        failing_repository.fail_next(RepositoryError::from_message(
            "storage unavailable".to_string(),
        ));
        let list_handler = ListProjectsHandler::new(failing_repository);

        let missing_error = match get_handler.handle(GetProjectRequest {
            project_id: "missing".to_string(),
        }) {
            Ok(response) => panic!("expected missing error, got response: {response:?}"),
            Err(error) => error,
        };
        let repository_error = match list_handler.handle(ListProjectsRequest {}) {
            Ok(response) => panic!("expected repository error, got response: {response:?}"),
            Err(error) => error,
        };

        assert_eq!(
            missing_error,
            ApplicationError::ProjectNotFound {
                project_id: "missing".to_string(),
            }
        );
        assert_eq!(
            repository_error,
            ApplicationError::ProjectRepository {
                source: RepositoryError::from_message("storage unavailable"),
            }
        );
    });
}

#[derive(Debug, Default)]
struct FakeProjectRepository {
    projects: RefCell<Vec<Project>>,
    next_error: RefCell<Option<RepositoryError>>,
}

impl FakeProjectRepository {
    fn with_projects(projects: Vec<Project>) -> Self {
        Self {
            projects: RefCell::new(projects),
            next_error: RefCell::new(None),
        }
    }

    /// Configures the next repository call to fail with a deterministic error.
    fn fail_next(&self, error: RepositoryError) {
        self.next_error.replace(Some(error));
    }

    /// Returns every non-deleted project so tests can assert visible repository state.
    fn visible_projects(&self) -> Vec<Project> {
        self.projects
            .borrow()
            .iter()
            .filter(|project| !project.audit_fields.is_deleted)
            .cloned()
            .collect()
    }

    /// Returns a queued error when a test wants to simulate repository failure.
    fn take_error(&self) -> Result<(), RepositoryError> {
        match self.next_error.borrow_mut().take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl ProjectRepository for Rc<FakeProjectRepository> {
    fn create_project(
        &self,
        project: Project,
        _main_workspace_location: WorkspaceLocation,
    ) -> Result<Project, RepositoryError> {
        self.take_error()?;

        self.projects.borrow_mut().push(project.clone());
        Ok(project)
    }

    fn find_project(&self, project_id: &ProjectId) -> Result<Option<Project>, RepositoryError> {
        self.take_error()?;

        Ok(self
            .projects
            .borrow()
            .iter()
            .find(|project| project.id == *project_id && !project.audit_fields.is_deleted)
            .cloned())
    }

    fn list_projects(&self) -> Result<Vec<Project>, RepositoryError> {
        self.take_error()?;

        Ok(self.visible_projects())
    }

    fn update_project(&self, project: Project) -> Result<Project, RepositoryError> {
        self.take_error()?;

        let mut projects = self.projects.borrow_mut();
        if let Some(existing_project) = projects.iter_mut().find(|existing_project| {
            existing_project.id == project.id && !existing_project.audit_fields.is_deleted
        }) {
            *existing_project = project.clone();
            Ok(project)
        } else {
            Err(RepositoryError::from_message(format!(
                "missing project during update: {}",
                project.id
            )))
        }
    }

    fn soft_delete_project(
        &self,
        project_id: &ProjectId,
        deleted_at: i64,
    ) -> Result<bool, RepositoryError> {
        self.take_error()?;

        let mut projects = self.projects.borrow_mut();
        if let Some(project) = projects
            .iter_mut()
            .find(|project| project.id == *project_id && !project.audit_fields.is_deleted)
        {
            project.audit_fields.updated_at = deleted_at;
            project.audit_fields.is_deleted = true;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

struct FixedProjectIdGenerator {
    project_id: ProjectId,
}

impl FixedProjectIdGenerator {
    fn new(project_id: impl Into<String>) -> Self {
        Self {
            project_id: ProjectId::new(project_id),
        }
    }
}

impl ProjectIdGenerator for FixedProjectIdGenerator {
    fn generate_project_id(&self) -> ProjectId {
        self.project_id.clone()
    }
}

struct FixedClock {
    timestamp_millis: i64,
}

impl FixedClock {
    fn new(timestamp_millis: i64) -> Self {
        Self { timestamp_millis }
    }
}

impl Clock for FixedClock {
    fn now_timestamp_millis(&self) -> i64 {
        self.timestamp_millis
    }
}
