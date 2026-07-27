use crate::{
    ApplicationError, Clock, CreateTaskHandler, CreateTaskWorktreeRequest,
    CreateTaskWorktreeResponse, DeleteTaskWorktreeRequest, GetTaskHandler, ListTasksHandler,
    TaskIdGenerator, TaskRepository, TaskRepositoryError, TaskWorktreeDeletionMode,
    TaskWorktreeProvisioner, TaskWorktreeProvisionerError, UpdateTaskHandler, WorktreeIdGenerator,
    WorktreeRepository, WorktreeRepositoryError,
};
use ora_contracts::{
    CreateTaskRequest, CreateTaskResponse, GetTaskRequest, GetTaskResponse, ListTasksRequest,
    ListTasksResponse, Task as ContractTask, TaskStatus as ContractTaskStatus, TaskWorkspaceMode,
    UpdateTaskRequest, UpdateTaskResponse,
};
use ora_domain::{
    AuditFields, ProjectId, Task, TaskId, TaskStatus as DomainTaskStatus, Worktree,
    WorktreeActivity as DomainWorktreeActivity, WorktreeId,
};
use ora_logging::{with_recorded_trace_logging, with_trace_logging};
use pretty_assertions::assert_eq;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

const TASK_ID: &str = "12345678-1234-5678-90ab-1234567890ab";
const WORK_DIR: &str = "/tmp/ora-worktrees";

/// Verifies create handlers provision and persist task-owned worktrees before returning the shared response.
#[test]
fn creates_tasks_with_owned_worktrees_and_clock_values() {
    with_trace_logging(|| {
        let task_repository = Rc::new(FakeTaskRepository::default());
        let worktree_repository = Rc::new(FakeWorktreeRepository::default());
        let provisioner = Rc::new(FakeTaskWorktreeProvisioner::default());
        let handler = CreateTaskHandler::new(
            task_repository.clone(),
            worktree_repository.clone(),
            FixedTaskIdGenerator::new(TASK_ID),
            FixedWorktreeIdGenerator::new("worktree-1"),
            provisioner.clone(),
            PathBuf::from(WORK_DIR),
            FixedClock::new(1_700_000_000_000),
        );

        let response = handler
            .handle(CreateTaskRequest {
                project_id: "project-1".to_string(),
                title: "Ship handlers".to_string(),
                status: ContractTaskStatus::Doing,
                workspace_mode: None,
            })
            .unwrap_or_else(|error| panic!("create handler failed: {error}"));

        assert_eq!(
            response,
            CreateTaskResponse {
                task: ContractTask {
                    id: TASK_ID.to_string(),
                    project_id: "project-1".to_string(),
                    title: "Ship handlers".to_string(),
                    status: ContractTaskStatus::Doing,
                    workspace_mode: TaskWorkspaceMode::Worktree,
                },
            }
        );
        assert_eq!(
            provisioner.created_requests(),
            vec![CreateTaskWorktreeRequest {
                branch_name: "ora/12345678".to_string(),
                worktree_path: Path::new(WORK_DIR).join(TASK_ID),
            }]
        );
        assert_eq!(
            worktree_repository.visible_worktrees(),
            vec![Worktree::new(
                WorktreeId::new("worktree-1"),
                TaskId::new(TASK_ID),
                Some("ora/12345678".to_string()),
                ora_domain::WorktreeBaseline::recorded("base-commit").unwrap(),
                DomainWorktreeActivity::Active,
                AuditFields::new(1_700_000_000_000, 1_700_000_000_000, false),
            )]
        );
        assert_eq!(
            task_repository.visible_tasks(),
            vec![Task::new(
                TaskId::new(TASK_ID),
                ProjectId::new("project-1"),
                "Ship handlers",
                DomainTaskStatus::Doing,
                Some(WorktreeId::new("worktree-1")),
                AuditFields::new(1_700_000_000_000, 1_700_000_000_000, false),
            )]
        );
    });
}

/// Verifies project-root tasks persist without provisioning Git or a worktree record.
#[test]
fn creates_project_root_tasks_without_worktrees() {
    with_trace_logging(|| {
        let task_repository = Rc::new(FakeTaskRepository::default());
        let worktree_repository = Rc::new(FakeWorktreeRepository::default());
        let provisioner = Rc::new(FakeTaskWorktreeProvisioner::default());
        let handler = CreateTaskHandler::new(
            task_repository.clone(),
            worktree_repository.clone(),
            FixedTaskIdGenerator::new(TASK_ID),
            FixedWorktreeIdGenerator::new("worktree-1"),
            provisioner.clone(),
            PathBuf::from(WORK_DIR),
            FixedClock::new(1_700_000_000_000),
        );

        let response = handler
            .handle(CreateTaskRequest {
                project_id: "project-1".to_string(),
                title: "Chat in project root".to_string(),
                status: ContractTaskStatus::Doing,
                workspace_mode: Some(TaskWorkspaceMode::ProjectRoot),
            })
            .unwrap_or_else(|error| panic!("create handler failed: {error}"));

        assert_eq!(
            response,
            CreateTaskResponse {
                task: ContractTask {
                    id: TASK_ID.to_string(),
                    project_id: "project-1".to_string(),
                    title: "Chat in project root".to_string(),
                    status: ContractTaskStatus::Doing,
                    workspace_mode: TaskWorkspaceMode::ProjectRoot,
                },
            }
        );
        assert!(provisioner.created_requests().is_empty());
        assert!(worktree_repository.visible_worktrees().is_empty());
        assert_eq!(task_repository.visible_tasks()[0].worktree_id, None,);
    });
}

/// Verifies worktree mode rejects an ordinary directory before persisting any Ora records.
#[test]
fn rejects_worktree_tasks_outside_git_repositories() {
    with_trace_logging(|| {
        let task_repository = Rc::new(FakeTaskRepository::default());
        let worktree_repository = Rc::new(FakeWorktreeRepository::default());
        let provisioner = Rc::new(FakeTaskWorktreeProvisioner::default());
        provisioner.fail_repository_validation(TaskWorktreeProvisionerError::NotARepository);
        let handler = CreateTaskHandler::new(
            task_repository.clone(),
            worktree_repository.clone(),
            FixedTaskIdGenerator::new(TASK_ID),
            FixedWorktreeIdGenerator::new("worktree-1"),
            provisioner.clone(),
            PathBuf::from(WORK_DIR),
            FixedClock::new(1_700_000_000_000),
        );

        let error = handler
            .handle(CreateTaskRequest {
                project_id: "project-1".to_string(),
                title: "Cannot create worktree here".to_string(),
                status: ContractTaskStatus::Doing,
                workspace_mode: Some(TaskWorkspaceMode::Worktree),
            })
            .expect_err("non-Git project root should be rejected");

        assert_eq!(error, ApplicationError::TaskWorktreeRequiresGitRepository);
        assert!(provisioner.created_requests().is_empty());
        assert!(worktree_repository.visible_worktrees().is_empty());
        assert!(task_repository.visible_tasks().is_empty());
    });
}

/// Verifies task creation regenerates ids when the short branch prefix already exists as a worktree folder.
#[test]
fn regenerates_task_ids_when_branch_prefix_folder_exists() {
    with_trace_logging(|| {
        let work_dir = unique_test_work_dir("task-prefix-collision");
        fs::create_dir_all(work_dir.join("12345678-existing-worktree"))
            .unwrap_or_else(|error| panic!("failed to create prefix collision fixture: {error}"));
        let task_repository = Rc::new(FakeTaskRepository::default());
        let worktree_repository = Rc::new(FakeWorktreeRepository::default());
        let provisioner = Rc::new(FakeTaskWorktreeProvisioner::default());
        let handler = CreateTaskHandler::new(
            task_repository.clone(),
            worktree_repository,
            SequenceTaskIdGenerator::new(vec![
                "12345678-1234-5678-90ab-1234567890ab",
                "87654321-1234-5678-90ab-1234567890ab",
            ]),
            FixedWorktreeIdGenerator::new("worktree-1"),
            provisioner.clone(),
            work_dir.clone(),
            FixedClock::new(1_700_000_000_000),
        );

        let response = handler
            .handle(CreateTaskRequest {
                project_id: "project-1".to_string(),
                title: "Ship handlers".to_string(),
                status: ContractTaskStatus::Doing,
                workspace_mode: None,
            })
            .unwrap_or_else(|error| panic!("create handler failed: {error}"));

        assert_eq!(
            response,
            CreateTaskResponse {
                task: ContractTask {
                    id: "87654321-1234-5678-90ab-1234567890ab".to_string(),
                    project_id: "project-1".to_string(),
                    title: "Ship handlers".to_string(),
                    status: ContractTaskStatus::Doing,
                    workspace_mode: TaskWorkspaceMode::Worktree,
                },
            }
        );
        assert_eq!(
            provisioner.created_requests(),
            vec![CreateTaskWorktreeRequest {
                branch_name: "ora/87654321".to_string(),
                worktree_path: work_dir.join("87654321-1234-5678-90ab-1234567890ab"),
            }]
        );
        assert_eq!(
            task_repository
                .visible_tasks()
                .into_iter()
                .map(|task| task.id)
                .collect::<Vec<_>>(),
            vec![TaskId::new("87654321-1234-5678-90ab-1234567890ab")]
        );

        fs::remove_dir_all(&work_dir)
            .unwrap_or_else(|error| panic!("failed to remove prefix collision fixture: {error}"));
    });
}

/// Verifies an orphaned task branch reserves its short prefix even after the worktree folder is deleted.
#[test]
fn regenerates_task_ids_when_orphaned_branch_exists() {
    with_trace_logging(|| {
        let work_dir = unique_test_work_dir("orphaned-task-branch");
        let provisioner = Rc::new(FakeTaskWorktreeProvisioner::with_existing_branches(vec![
            "ora/12345678",
        ]));
        let handler = CreateTaskHandler::new(
            Rc::new(FakeTaskRepository::default()),
            Rc::new(FakeWorktreeRepository::default()),
            SequenceTaskIdGenerator::new(vec![
                "12345678-1234-5678-90ab-1234567890ab",
                "87654321-1234-5678-90ab-1234567890ab",
            ]),
            FixedWorktreeIdGenerator::new("worktree-1"),
            provisioner.clone(),
            work_dir.clone(),
            FixedClock::new(1_700_000_000_000),
        );

        let response = handler
            .handle(CreateTaskRequest {
                project_id: "project-1".to_string(),
                title: "Ship handlers".to_string(),
                status: ContractTaskStatus::Doing,
                workspace_mode: None,
            })
            .unwrap_or_else(|error| panic!("create handler failed: {error}"));

        assert_eq!(
            response,
            CreateTaskResponse {
                task: ContractTask {
                    id: "87654321-1234-5678-90ab-1234567890ab".to_string(),
                    project_id: "project-1".to_string(),
                    title: "Ship handlers".to_string(),
                    status: ContractTaskStatus::Doing,
                    workspace_mode: TaskWorkspaceMode::Worktree,
                },
            }
        );
        assert_eq!(
            provisioner.created_requests(),
            vec![CreateTaskWorktreeRequest {
                branch_name: "ora/87654321".to_string(),
                worktree_path: work_dir.join("87654321-1234-5678-90ab-1234567890ab"),
            }]
        );
    });
}

/// Verifies first-time task creation succeeds before the configured worktree root exists.
#[test]
fn creates_task_when_work_dir_does_not_exist() {
    with_trace_logging(|| {
        let work_dir = unique_test_work_dir("missing-work-dir");
        let provisioner = Rc::new(FakeTaskWorktreeProvisioner::default());
        let handler = CreateTaskHandler::new(
            Rc::new(FakeTaskRepository::default()),
            Rc::new(FakeWorktreeRepository::default()),
            FixedTaskIdGenerator::new(TASK_ID),
            FixedWorktreeIdGenerator::new("worktree-1"),
            provisioner.clone(),
            work_dir.clone(),
            FixedClock::new(1_700_000_000_000),
        );

        let response = handler
            .handle(CreateTaskRequest {
                project_id: "project-1".to_string(),
                title: "Ship handlers".to_string(),
                status: ContractTaskStatus::Doing,
                workspace_mode: None,
            })
            .unwrap_or_else(|error| panic!("create handler failed: {error}"));

        assert_eq!(
            response,
            CreateTaskResponse {
                task: ContractTask {
                    id: TASK_ID.to_string(),
                    project_id: "project-1".to_string(),
                    title: "Ship handlers".to_string(),
                    status: ContractTaskStatus::Doing,
                    workspace_mode: TaskWorkspaceMode::Worktree,
                },
            }
        );
        assert_eq!(
            provisioner.created_requests(),
            vec![CreateTaskWorktreeRequest {
                branch_name: "ora/12345678".to_string(),
                worktree_path: work_dir.join(TASK_ID),
            }]
        );
    });
}

/// Verifies repeated branch-prefix collisions return a stable error and emit the shared failure event.
#[test]
fn reports_task_worktree_error_when_task_id_retries_are_exhausted() {
    let work_dir = unique_test_work_dir("task-prefix-exhaustion");
    fs::create_dir_all(work_dir.join("12345678-existing-worktree"))
        .unwrap_or_else(|error| panic!("failed to create prefix collision fixture: {error}"));
    let recorder = EventRecorder::default();

    with_recorded_trace_logging(recorder.layer(), || {
        let handler = CreateTaskHandler::new(
            Rc::new(FakeTaskRepository::default()),
            Rc::new(FakeWorktreeRepository::default()),
            FixedTaskIdGenerator::new(TASK_ID),
            FixedWorktreeIdGenerator::new("worktree-1"),
            Rc::new(FakeTaskWorktreeProvisioner::default()),
            work_dir.clone(),
            FixedClock::new(1_700_000_000_000),
        );

        assert_eq!(
            handler
                .handle(CreateTaskRequest {
                    project_id: "project-1".to_string(),
                    title: "Ship handlers".to_string(),
                    status: ContractTaskStatus::Doing,
                    workspace_mode: None,
                })
                .unwrap_err(),
            ApplicationError::TaskWorktree {
                message:
                    "failed to generate a task branch prefix without collision after 3 attempts"
                        .to_string(),
            }
        );
    });

    assert_eq!(
        recorder.events(),
        vec![LoggedEvent {
            level: "ERROR".to_string(),
            target: "ora_application::task::handlers".to_string(),
            fields: BTreeMap::from([
                ("error.kind".to_string(), "task_worktree".to_string()),
                (
                    "error.message".to_string(),
                    "task worktree operation failed: failed to generate a task branch prefix without collision after 3 attempts"
                        .to_string(),
                ),
                ("message".to_string(), "task operation failed".to_string()),
                ("method".to_string(), "log_task_failure".to_string()),
                ("operation".to_string(), "create_task".to_string()),
            ]),
        }]
    );

    fs::remove_dir_all(&work_dir)
        .unwrap_or_else(|error| panic!("failed to remove prefix collision fixture: {error}"));
}

/// Verifies get handlers return the shared contract projection for existing tasks.
#[test]
fn gets_tasks_by_identifier() {
    with_trace_logging(|| {
        let repository = Rc::new(FakeTaskRepository::with_tasks(vec![Task::new(
            TaskId::new("task-1"),
            ProjectId::new("project-1"),
            "Ship handlers",
            DomainTaskStatus::Todo,
            None,
            AuditFields::new(1, 2, false),
        )]));
        let handler = GetTaskHandler::new(repository);

        let response = handler
            .handle(GetTaskRequest {
                task_id: "task-1".to_string(),
            })
            .unwrap_or_else(|error| panic!("get handler failed: {error}"));

        assert_eq!(
            response,
            GetTaskResponse {
                task: ContractTask {
                    id: "task-1".to_string(),
                    project_id: "project-1".to_string(),
                    title: "Ship handlers".to_string(),
                    status: ContractTaskStatus::Todo,
                    workspace_mode: TaskWorkspaceMode::ProjectRoot,
                },
            }
        );
    });
}

/// Verifies list handlers map every stored task into the shared contract payload.
#[test]
fn lists_visible_tasks() {
    with_trace_logging(|| {
        let repository = Rc::new(FakeTaskRepository::with_tasks(vec![
            Task::new(
                TaskId::new("task-1"),
                ProjectId::new("project-1"),
                "Ship handlers",
                DomainTaskStatus::Todo,
                None,
                AuditFields::new(1, 2, false),
            ),
            Task::new(
                TaskId::new("task-2"),
                ProjectId::new("project-2"),
                "Wire exports",
                DomainTaskStatus::Done,
                Some(WorktreeId::new("worktree-2")),
                AuditFields::new(3, 4, false),
            ),
        ]));
        let handler = ListTasksHandler::new(repository);

        let response = handler
            .handle(ListTasksRequest {})
            .unwrap_or_else(|error| panic!("list handler failed: {error}"));

        assert_eq!(
            response,
            ListTasksResponse {
                tasks: vec![
                    ContractTask {
                        id: "task-1".to_string(),
                        project_id: "project-1".to_string(),
                        title: "Ship handlers".to_string(),
                        status: ContractTaskStatus::Todo,
                        workspace_mode: TaskWorkspaceMode::ProjectRoot,
                    },
                    ContractTask {
                        id: "task-2".to_string(),
                        project_id: "project-2".to_string(),
                        title: "Wire exports".to_string(),
                        status: ContractTaskStatus::Done,
                        workspace_mode: TaskWorkspaceMode::Worktree,
                    },
                ],
            }
        );
    });
}

/// Verifies update handlers preserve created timestamps while refreshing mutable fields.
#[test]
fn updates_tasks_with_refreshed_timestamps() {
    with_trace_logging(|| {
        let repository = Rc::new(FakeTaskRepository::with_tasks(vec![Task::new(
            TaskId::new("task-1"),
            ProjectId::new("project-1"),
            "Ship handlers",
            DomainTaskStatus::Todo,
            None,
            AuditFields::new(10, 20, false),
        )]));
        let handler = UpdateTaskHandler::new(repository.clone(), FixedClock::new(30));

        let response = handler
            .handle(UpdateTaskRequest {
                task_id: "task-1".to_string(),
                title: "Ship updated handlers".to_string(),
                status: ContractTaskStatus::Done,
            })
            .unwrap_or_else(|error| panic!("update handler failed: {error}"));

        assert_eq!(
            response,
            UpdateTaskResponse {
                task: ContractTask {
                    id: "task-1".to_string(),
                    project_id: "project-1".to_string(),
                    title: "Ship updated handlers".to_string(),
                    status: ContractTaskStatus::Done,
                    workspace_mode: TaskWorkspaceMode::ProjectRoot,
                },
            }
        );
        assert_eq!(
            repository.visible_tasks(),
            vec![Task::new(
                TaskId::new("task-1"),
                ProjectId::new("project-1"),
                "Ship updated handlers",
                DomainTaskStatus::Done,
                None,
                AuditFields::new(10, 30, false),
            )]
        );
    });
}

/// Verifies create handlers compensate by deleting the created worktree when task persistence fails.
#[test]
fn cleans_up_created_worktree_when_task_persistence_fails() {
    with_trace_logging(|| {
        let task_repository = Rc::new(FakeTaskRepository::default());
        let worktree_repository = Rc::new(FakeWorktreeRepository::default());
        let provisioner = Rc::new(FakeTaskWorktreeProvisioner::default());
        task_repository.fail_next(TaskRepositoryError::OperationFailed(
            "task write failed".to_string(),
        ));
        let handler = CreateTaskHandler::new(
            task_repository.clone(),
            worktree_repository,
            FixedTaskIdGenerator::new(TASK_ID),
            FixedWorktreeIdGenerator::new("worktree-1"),
            provisioner.clone(),
            PathBuf::from(WORK_DIR),
            FixedClock::new(50),
        );

        let error = handler
            .handle(CreateTaskRequest {
                project_id: "project-1".to_string(),
                title: "Ship handlers".to_string(),
                status: ContractTaskStatus::Todo,
                workspace_mode: None,
            })
            .unwrap_err();

        assert_eq!(
            error,
            ApplicationError::TaskRepository {
                message: "task write failed".to_string(),
            }
        );
        assert_eq!(
            provisioner.deleted_requests(),
            vec![DeleteTaskWorktreeRequest {
                branch_name: "ora/12345678".to_string(),
                mode: TaskWorktreeDeletionMode::Force,
            }]
        );
    });
}

/// Verifies provisioning failures become stable application errors before any persistence occurs.
#[test]
fn reports_application_errors() {
    with_trace_logging(|| {
        let missing_repository = Rc::new(FakeTaskRepository::default());
        let get_handler = GetTaskHandler::new(missing_repository);
        let task_repository = Rc::new(FakeTaskRepository::default());
        let worktree_repository = Rc::new(FakeWorktreeRepository::default());
        let provisioner = Rc::new(FakeTaskWorktreeProvisioner::default());
        provisioner.fail_next_create(TaskWorktreeProvisionerError::OperationFailed(
            "failed to create linked worktree".to_string(),
        ));
        let create_handler = CreateTaskHandler::new(
            task_repository,
            worktree_repository,
            FixedTaskIdGenerator::new(TASK_ID),
            FixedWorktreeIdGenerator::new("worktree-1"),
            provisioner,
            PathBuf::from(WORK_DIR),
            FixedClock::new(60),
        );

        let missing_error = get_handler
            .handle(GetTaskRequest {
                task_id: "missing".to_string(),
            })
            .unwrap_err();
        let provisioning_error = create_handler
            .handle(CreateTaskRequest {
                project_id: "project-1".to_string(),
                title: "Ship handlers".to_string(),
                status: ContractTaskStatus::Todo,
                workspace_mode: None,
            })
            .unwrap_err();

        assert_eq!(
            missing_error,
            ApplicationError::TaskNotFound {
                task_id: "missing".to_string(),
            }
        );
        assert_eq!(
            provisioning_error,
            ApplicationError::TaskWorktree {
                message: "failed to create linked worktree".to_string(),
            }
        );
    });
}

/// Verifies task handlers emit structured success and provisioning-failure events under a scoped subscriber.
#[test]
fn emits_structured_operational_events() {
    let recorder = EventRecorder::default();
    with_recorded_trace_logging(recorder.layer(), || {
        let create_task_repository = Rc::new(FakeTaskRepository::default());
        let create_worktree_repository = Rc::new(FakeWorktreeRepository::default());
        let create_provisioner = Rc::new(FakeTaskWorktreeProvisioner::default());
        let create_handler = CreateTaskHandler::new(
            create_task_repository,
            create_worktree_repository,
            FixedTaskIdGenerator::new(TASK_ID),
            FixedWorktreeIdGenerator::new("worktree-1"),
            create_provisioner,
            PathBuf::from(WORK_DIR),
            FixedClock::new(5),
        );
        let failing_task_repository = Rc::new(FakeTaskRepository::default());
        let failing_worktree_repository = Rc::new(FakeWorktreeRepository::default());
        let failing_provisioner = Rc::new(FakeTaskWorktreeProvisioner::default());
        failing_provisioner.fail_next_create(TaskWorktreeProvisionerError::OperationFailed(
            "failed to create linked worktree".to_string(),
        ));
        let failing_handler = CreateTaskHandler::new(
            failing_task_repository,
            failing_worktree_repository,
            FixedTaskIdGenerator::new("87654321-1234-5678-90ab-1234567890ab"),
            FixedWorktreeIdGenerator::new("worktree-2"),
            failing_provisioner,
            PathBuf::from(WORK_DIR),
            FixedClock::new(6),
        );

        create_handler
            .handle(CreateTaskRequest {
                project_id: "project-1".to_string(),
                title: "Ship handlers".to_string(),
                status: ContractTaskStatus::Todo,
                workspace_mode: None,
            })
            .unwrap();
        assert_eq!(
            failing_handler
                .handle(CreateTaskRequest {
                    project_id: "project-1".to_string(),
                    title: "Ship handlers".to_string(),
                    status: ContractTaskStatus::Todo,
                    workspace_mode: None,
                })
                .unwrap_err(),
            ApplicationError::TaskWorktree {
                message: "failed to create linked worktree".to_string(),
            }
        );
    });

    assert_eq!(
        recorder.events(),
        vec![
            LoggedEvent {
                level: "INFO".to_string(),
                target: "ora_application::task::handlers".to_string(),
                fields: BTreeMap::from([
                    (
                        "message".to_string(),
                        "task operation completed".to_string()
                    ),
                    ("method".to_string(), "log_task_success".to_string()),
                    ("operation".to_string(), "create_task".to_string()),
                    ("task_id".to_string(), TASK_ID.to_string()),
                ]),
            },
            LoggedEvent {
                level: "ERROR".to_string(),
                target: "ora_application::task::handlers".to_string(),
                fields: BTreeMap::from([
                    ("error.kind".to_string(), "task_worktree".to_string()),
                    (
                        "error.message".to_string(),
                        "task worktree operation failed: failed to create linked worktree"
                            .to_string(),
                    ),
                    ("message".to_string(), "task operation failed".to_string()),
                    ("method".to_string(), "log_task_failure".to_string()),
                    ("operation".to_string(), "create_task".to_string()),
                    (
                        "task_id".to_string(),
                        "87654321-1234-5678-90ab-1234567890ab".to_string(),
                    ),
                ]),
            },
        ]
    );
}

#[derive(Debug, Default)]
struct FakeTaskRepository {
    tasks: RefCell<Vec<Task>>,
    next_error: RefCell<Option<TaskRepositoryError>>,
}

impl FakeTaskRepository {
    /// Builds a fake repository seeded with the provided task rows.
    fn with_tasks(tasks: Vec<Task>) -> Self {
        Self {
            tasks: RefCell::new(tasks),
            next_error: RefCell::new(None),
        }
    }

    /// Configures the next repository call to fail with a deterministic error.
    fn fail_next(&self, error: TaskRepositoryError) {
        self.next_error.replace(Some(error));
    }

    /// Returns every non-deleted task so tests can assert visible repository state.
    fn visible_tasks(&self) -> Vec<Task> {
        self.tasks
            .borrow()
            .iter()
            .filter(|task| !task.audit_fields.is_deleted)
            .cloned()
            .collect()
    }

    /// Returns a queued error when a test wants to simulate repository failure.
    fn take_error(&self) -> Result<(), TaskRepositoryError> {
        match self.next_error.borrow_mut().take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl TaskRepository for Rc<FakeTaskRepository> {
    fn create_task(&self, task: Task) -> Result<Task, TaskRepositoryError> {
        self.take_error()?;
        self.tasks.borrow_mut().push(task.clone());
        Ok(task)
    }

    fn find_task(&self, task_id: &TaskId) -> Result<Option<Task>, TaskRepositoryError> {
        self.take_error()?;

        Ok(self
            .tasks
            .borrow()
            .iter()
            .find(|task| task.id == *task_id && !task.audit_fields.is_deleted)
            .cloned())
    }

    fn list_tasks(&self) -> Result<Vec<Task>, TaskRepositoryError> {
        self.take_error()?;
        Ok(self.visible_tasks())
    }

    fn update_task(&self, task: Task) -> Result<Task, TaskRepositoryError> {
        self.take_error()?;

        let mut tasks = self.tasks.borrow_mut();
        if let Some(existing_task) = tasks.iter_mut().find(|existing_task| {
            existing_task.id == task.id && !existing_task.audit_fields.is_deleted
        }) {
            *existing_task = task.clone();
            Ok(task)
        } else {
            Err(TaskRepositoryError::OperationFailed(format!(
                "missing task during update: {}",
                task.id
            )))
        }
    }

    fn soft_delete_task(
        &self,
        task_id: &TaskId,
        deleted_at: i64,
    ) -> Result<bool, TaskRepositoryError> {
        self.take_error()?;

        let mut tasks = self.tasks.borrow_mut();
        if let Some(task) = tasks
            .iter_mut()
            .find(|task| task.id == *task_id && !task.audit_fields.is_deleted)
        {
            task.audit_fields.updated_at = deleted_at;
            task.audit_fields.is_deleted = true;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

#[derive(Debug, Default)]
struct FakeWorktreeRepository {
    worktrees: RefCell<Vec<Worktree>>,
    next_error: RefCell<Option<WorktreeRepositoryError>>,
}

impl FakeWorktreeRepository {
    /// Returns every non-deleted worktree so tests can assert visible repository state.
    fn visible_worktrees(&self) -> Vec<Worktree> {
        self.worktrees
            .borrow()
            .iter()
            .filter(|worktree| !worktree.audit_fields.is_deleted)
            .cloned()
            .collect()
    }

    /// Returns a queued error when a test wants to simulate repository failure.
    fn take_error(&self) -> Result<(), WorktreeRepositoryError> {
        match self.next_error.borrow_mut().take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl WorktreeRepository for Rc<FakeWorktreeRepository> {
    fn create_worktree(&self, worktree: Worktree) -> Result<Worktree, WorktreeRepositoryError> {
        self.take_error()?;
        self.worktrees.borrow_mut().push(worktree.clone());
        Ok(worktree)
    }

    fn find_worktree(
        &self,
        worktree_id: &WorktreeId,
    ) -> Result<Option<Worktree>, WorktreeRepositoryError> {
        self.take_error()?;

        Ok(self
            .worktrees
            .borrow()
            .iter()
            .find(|worktree| worktree.id == *worktree_id && !worktree.audit_fields.is_deleted)
            .cloned())
    }

    fn list_worktrees(&self) -> Result<Vec<Worktree>, WorktreeRepositoryError> {
        self.take_error()?;
        Ok(self.visible_worktrees())
    }

    fn update_worktree(&self, worktree: Worktree) -> Result<Worktree, WorktreeRepositoryError> {
        self.take_error()?;

        let mut worktrees = self.worktrees.borrow_mut();
        if let Some(existing_worktree) = worktrees.iter_mut().find(|existing_worktree| {
            existing_worktree.id == worktree.id && !existing_worktree.audit_fields.is_deleted
        }) {
            *existing_worktree = worktree.clone();
            Ok(worktree)
        } else {
            Err(WorktreeRepositoryError::OperationFailed(format!(
                "missing worktree during update: {}",
                worktree.id
            )))
        }
    }

    fn soft_delete_worktree(
        &self,
        worktree_id: &WorktreeId,
        deleted_at: i64,
    ) -> Result<bool, WorktreeRepositoryError> {
        self.take_error()?;

        let mut worktrees = self.worktrees.borrow_mut();
        if let Some(worktree) = worktrees
            .iter_mut()
            .find(|worktree| worktree.id == *worktree_id && !worktree.audit_fields.is_deleted)
        {
            worktree.audit_fields.updated_at = deleted_at;
            worktree.audit_fields.is_deleted = true;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

#[derive(Debug, Default)]
struct FakeTaskWorktreeProvisioner {
    existing_branches: RefCell<Vec<String>>,
    created_requests: RefCell<Vec<CreateTaskWorktreeRequest>>,
    deleted_requests: RefCell<Vec<DeleteTaskWorktreeRequest>>,
    next_repository_error: RefCell<Option<TaskWorktreeProvisionerError>>,
    next_create_error: RefCell<Option<TaskWorktreeProvisionerError>>,
    next_delete_error: RefCell<Option<TaskWorktreeProvisionerError>>,
}

impl FakeTaskWorktreeProvisioner {
    /// Builds a fake provisioner seeded with repository-local branches.
    fn with_existing_branches(branches: Vec<&str>) -> Self {
        Self {
            existing_branches: RefCell::new(branches.into_iter().map(str::to_string).collect()),
            ..Self::default()
        }
    }

    /// Configures repository validation to fail once with a deterministic error.
    fn fail_repository_validation(&self, error: TaskWorktreeProvisionerError) {
        self.next_repository_error.replace(Some(error));
    }

    /// Configures the next create request to fail with a deterministic error.
    fn fail_next_create(&self, error: TaskWorktreeProvisionerError) {
        self.next_create_error.replace(Some(error));
    }

    /// Returns the create requests recorded by this fake provisioner.
    fn created_requests(&self) -> Vec<CreateTaskWorktreeRequest> {
        self.created_requests.borrow().clone()
    }

    /// Returns the delete requests recorded by this fake provisioner.
    fn deleted_requests(&self) -> Vec<DeleteTaskWorktreeRequest> {
        self.deleted_requests.borrow().clone()
    }

    /// Returns the next queued create failure, if any.
    fn take_create_error(&self) -> Result<(), TaskWorktreeProvisionerError> {
        match self.next_create_error.borrow_mut().take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Returns the next queued delete failure, if any.
    fn take_delete_error(&self) -> Result<(), TaskWorktreeProvisionerError> {
        match self.next_delete_error.borrow_mut().take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl TaskWorktreeProvisioner for Rc<FakeTaskWorktreeProvisioner> {
    fn validate_repository(&self) -> Result<(), TaskWorktreeProvisionerError> {
        match self.next_repository_error.borrow_mut().take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn task_branch_exists(&self, branch_name: &str) -> Result<bool, TaskWorktreeProvisionerError> {
        Ok(self
            .existing_branches
            .borrow()
            .iter()
            .any(|branch| branch == branch_name))
    }

    fn create_task_worktree(
        &self,
        request: CreateTaskWorktreeRequest,
    ) -> Result<CreateTaskWorktreeResponse, TaskWorktreeProvisionerError> {
        self.take_create_error()?;
        self.created_requests.borrow_mut().push(request);
        Ok(CreateTaskWorktreeResponse {
            base_commit_id: "base-commit".to_string(),
        })
    }

    fn delete_task_worktree(
        &self,
        request: DeleteTaskWorktreeRequest,
    ) -> Result<(), TaskWorktreeProvisionerError> {
        self.take_delete_error()?;
        self.deleted_requests.borrow_mut().push(request);
        Ok(())
    }
}

struct FixedTaskIdGenerator {
    task_id: TaskId,
}

impl FixedTaskIdGenerator {
    /// Builds an identifier generator that always returns the provided task id.
    fn new(task_id: impl Into<String>) -> Self {
        Self {
            task_id: TaskId::new(task_id),
        }
    }
}

impl TaskIdGenerator for FixedTaskIdGenerator {
    fn generate_task_id(&self) -> TaskId {
        self.task_id.clone()
    }
}

struct SequenceTaskIdGenerator {
    task_ids: RefCell<Vec<TaskId>>,
}

impl SequenceTaskIdGenerator {
    /// Builds an identifier generator that returns ids in the provided order.
    fn new(task_ids: Vec<&str>) -> Self {
        Self {
            task_ids: RefCell::new(task_ids.into_iter().rev().map(TaskId::new).collect()),
        }
    }
}

impl TaskIdGenerator for SequenceTaskIdGenerator {
    fn generate_task_id(&self) -> TaskId {
        self.task_ids
            .borrow_mut()
            .pop()
            .unwrap_or_else(|| panic!("sequence task id generator exhausted"))
    }
}

struct FixedWorktreeIdGenerator {
    worktree_id: WorktreeId,
}

impl FixedWorktreeIdGenerator {
    /// Builds an identifier generator that always returns the provided worktree id.
    fn new(worktree_id: impl Into<String>) -> Self {
        Self {
            worktree_id: WorktreeId::new(worktree_id),
        }
    }
}

impl WorktreeIdGenerator for FixedWorktreeIdGenerator {
    fn generate_worktree_id(&self) -> WorktreeId {
        self.worktree_id.clone()
    }
}

struct FixedClock {
    timestamp_millis: i64,
}

impl FixedClock {
    /// Builds a clock that always returns the provided timestamp.
    fn new(timestamp_millis: i64) -> Self {
        Self { timestamp_millis }
    }
}

impl Clock for FixedClock {
    fn now_timestamp_millis(&self) -> i64 {
        self.timestamp_millis
    }
}

/// Builds a process-scoped temp path for tests that need filesystem-backed worktree roots.
fn unique_test_work_dir(name: &str) -> PathBuf {
    let work_dir =
        std::env::temp_dir().join(format!("ora-application-{name}-{}", std::process::id()));
    if work_dir.exists() {
        fs::remove_dir_all(&work_dir)
            .unwrap_or_else(|error| panic!("failed to reset test work dir: {error}"));
    }

    work_dir
}

/// Captures one emitted event in a comparison-friendly structure for logging assertions.
#[derive(Clone, Debug, Eq, PartialEq)]
struct LoggedEvent {
    level: String,
    target: String,
    fields: BTreeMap<String, String>,
}

/// Records tracing events into shared memory so tests can assert full structured outcomes.
#[derive(Clone, Debug, Default)]
struct EventRecorder {
    events: Arc<Mutex<Vec<LoggedEvent>>>,
}

impl EventRecorder {
    /// Builds the recording layer attached to one scoped test subscriber.
    fn layer(&self) -> RecordingLayer {
        RecordingLayer {
            events: self.events.clone(),
        }
    }

    /// Returns every captured event in emission order.
    fn events(&self) -> Vec<LoggedEvent> {
        self.events.lock().unwrap().clone()
    }
}

/// Pushes each tracing event into the shared recorder without relying on global subscriber state.
#[derive(Clone, Debug)]
struct RecordingLayer {
    events: Arc<Mutex<Vec<LoggedEvent>>>,
}

impl<S> Layer<S> for RecordingLayer
where
    S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    /// Converts each event into a stable, fully comparable structure for test assertions.
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, S>) {
        let mut visitor = EventFieldVisitor::default();
        event.record(&mut visitor);
        self.events.lock().unwrap().push(LoggedEvent {
            level: event.metadata().level().to_string(),
            target: event.metadata().target().to_string(),
            fields: visitor.fields,
        });
    }
}

/// Records tracing fields as strings because these tests care about semantic content, not JSON formatting.
#[derive(Debug, Default)]
struct EventFieldVisitor {
    fields: BTreeMap<String, String>,
}

impl tracing::field::Visit for EventFieldVisitor {
    /// Preserves string fields exactly as handler logs emitted them.
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.fields
            .insert(field.name().to_string(), value.to_string());
    }

    /// Preserves signed integers in decimal form for stable assertions.
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.fields
            .insert(field.name().to_string(), value.to_string());
    }

    /// Preserves unsigned integers in decimal form for stable assertions.
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.fields
            .insert(field.name().to_string(), value.to_string());
    }

    /// Falls back to debug formatting for field types without a more specific visitor hook.
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.fields.insert(
            field.name().to_string(),
            format!("{value:?}").trim_matches('"').to_string(),
        );
    }
}
