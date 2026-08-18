use crate::{
    ApplicationError, Clock, CreateTaskHandler, CreateTaskWorktreeRequest,
    CreateTaskWorktreeResponse, DeleteTaskWorktreeRequest, GetTaskHandler, ListTasksHandler,
    RepositoryError, TaskIdGenerator, TaskRepository, TaskWorkspaceCommit, TaskWorktreeProvisioner,
    TaskWorktreeProvisionerError, UpdateTaskHandler, WorkspaceCommitOutcome, WorktreeIdGenerator,
    WorktreeProvisioningLeaseStore, WorktreeRepository,
};
use ora_contracts::{
    CreateTaskRequest, CreateTaskResponse, GetTaskRequest, GetTaskResponse, ListTasksRequest,
    ListTasksResponse, Task as ContractTask, TaskType as ContractTaskType, TaskWorkspaceMode,
    UpdateTaskRequest, UpdateTaskResponse,
};
use ora_domain::{
    AuditFields, ProjectId, Task, TaskId, Worktree, WorktreeActivity as DomainWorktreeActivity,
    WorktreeId, WorktreeProvisioningLease, WorktreeProvisioningLeaseId,
};
use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

const REPO_ROOT: &str = "/repos/project-1";

const TASK_ID: &str = "12345678-1234-5678-90ab-1234567890ab";
const WORK_DIR: &str = "/tmp/ora-worktrees";

/// Verifies create handlers provision and persist task-owned worktrees before returning the shared response.
#[test]
fn creates_tasks_with_owned_worktrees_and_clock_values() {
    with_trace_logging(|| {
        let workspace_commit = Rc::new(FakeWorkspaceCommit::default());
        let lease_store = FakeLeaseStore::default();
        let provisioner = Rc::new(FakeTaskWorktreeProvisioner::default());
        let handler = CreateTaskHandler::new(
            workspace_commit.clone(),
            lease_store.clone(),
            FixedTaskIdGenerator::new(TASK_ID),
            FixedWorktreeIdGenerator::new("worktree-1"),
            provisioner.clone(),
            PathBuf::from(REPO_ROOT),
            PathBuf::from(WORK_DIR),
            FixedClock::new(1_700_000_000_000),
        );

        let response = handler
            .handle(CreateTaskRequest {
                project_id: "project-1".to_string(),
                title: "Ship handlers".to_string(),
                workspace_mode: None,
                base_branch: Some("main".to_string()),
            })
            .unwrap_or_else(|error| panic!("create handler failed: {error}"));

        assert_eq!(
            response,
            CreateTaskResponse {
                task: ContractTask {
                    id: TASK_ID.to_string(),
                    project_id: "project-1".to_string(),
                    title: "Ship handlers".to_string(),
                    workspace_mode: TaskWorkspaceMode::Worktree,
                    task_type: ContractTaskType::Default,
                    workflow_run_id: None,
                },
            }
        );
        assert_eq!(
            provisioner.created_requests(),
            vec![CreateTaskWorktreeRequest {
                branch_name: "ora/12345678".to_string(),
                base_reference_name: "main".to_string(),
                worktree_path: Path::new(WORK_DIR).join(TASK_ID),
            }]
        );
        // The provisioning lease is written before Git work with the exact
        // identity the cleanup path would need to reclaim the resources.
        let created_leases = lease_store.created_leases();
        assert_eq!(created_leases.len(), 1);
        let lease = &created_leases[0];
        assert_eq!(
            (
                lease.project_id.clone(),
                lease.task_id.clone(),
                lease.repository_root.clone(),
                lease.checkout_root.clone(),
                lease.branch_name.clone(),
            ),
            (
                ProjectId::new("project-1"),
                TaskId::new(TASK_ID),
                REPO_ROOT.to_string(),
                Path::new(WORK_DIR)
                    .join(TASK_ID)
                    .to_string_lossy()
                    .into_owned(),
                "ora/12345678".to_string(),
            )
        );
        assert!(lease_store.released_leases().is_empty());
        let committed = workspace_commit.committed_worktree_tasks();
        assert_eq!(committed.len(), 1);
        let (task, worktree, lease_id) = &committed[0];
        assert_eq!(lease_id, &lease.id);
        assert_eq!(
            worktree,
            &Worktree::new(
                WorktreeId::new("worktree-1"),
                TaskId::new(TASK_ID),
                Some("ora/12345678".to_string()),
                Some(
                    Path::new(WORK_DIR)
                        .join(TASK_ID)
                        .to_string_lossy()
                        .into_owned()
                ),
                ora_domain::WorktreeBaseline::recorded("base-commit").unwrap(),
                DomainWorktreeActivity::Active,
                AuditFields::new(1_700_000_000_000, 1_700_000_000_000, false),
            )
        );
        assert_eq!(
            task,
            &Task::new(
                TaskId::new(TASK_ID),
                ProjectId::new("project-1"),
                "Ship handlers",
                Some(WorktreeId::new("worktree-1")),
                AuditFields::new(1_700_000_000_000, 1_700_000_000_000, false),
            )
        );
    });
}

/// Verifies project-root tasks persist without provisioning Git or a worktree record.
#[test]
fn creates_project_root_tasks_without_worktrees() {
    with_trace_logging(|| {
        let workspace_commit = Rc::new(FakeWorkspaceCommit::default());
        let lease_store = FakeLeaseStore::default();
        let provisioner = Rc::new(FakeTaskWorktreeProvisioner::default());
        let handler = CreateTaskHandler::new(
            workspace_commit.clone(),
            lease_store.clone(),
            FixedTaskIdGenerator::new(TASK_ID),
            FixedWorktreeIdGenerator::new("worktree-1"),
            provisioner.clone(),
            PathBuf::from(REPO_ROOT),
            PathBuf::from(WORK_DIR),
            FixedClock::new(1_700_000_000_000),
        );

        let response = handler
            .handle(CreateTaskRequest {
                project_id: "project-1".to_string(),
                title: "Chat in project root".to_string(),
                workspace_mode: Some(TaskWorkspaceMode::ProjectRoot),
                base_branch: None,
            })
            .unwrap_or_else(|error| panic!("create handler failed: {error}"));

        assert_eq!(
            response,
            CreateTaskResponse {
                task: ContractTask {
                    id: TASK_ID.to_string(),
                    project_id: "project-1".to_string(),
                    title: "Chat in project root".to_string(),
                    workspace_mode: TaskWorkspaceMode::ProjectRoot,
                    task_type: ContractTaskType::Default,
                    workflow_run_id: None,
                },
            }
        );
        assert!(provisioner.created_requests().is_empty());
        assert!(lease_store.created_leases().is_empty());
        assert_eq!(
            workspace_commit.committed_project_root_tasks()[0].worktree_id,
            None,
        );
    });
}

/// Verifies worktree mode rejects an ordinary directory before persisting any Ora records.
#[test]
fn rejects_worktree_tasks_outside_git_repositories() {
    with_trace_logging(|| {
        let workspace_commit = Rc::new(FakeWorkspaceCommit::default());
        let lease_store = FakeLeaseStore::default();
        let provisioner = Rc::new(FakeTaskWorktreeProvisioner::default());
        provisioner.fail_repository_validation(TaskWorktreeProvisionerError::NotARepository);
        let handler = CreateTaskHandler::new(
            workspace_commit.clone(),
            lease_store.clone(),
            FixedTaskIdGenerator::new(TASK_ID),
            FixedWorktreeIdGenerator::new("worktree-1"),
            provisioner.clone(),
            PathBuf::from(REPO_ROOT),
            PathBuf::from(WORK_DIR),
            FixedClock::new(1_700_000_000_000),
        );

        let error = handler
            .handle(CreateTaskRequest {
                project_id: "project-1".to_string(),
                title: "Cannot create worktree here".to_string(),
                workspace_mode: Some(TaskWorkspaceMode::Worktree),
                base_branch: Some("main".to_string()),
            })
            .expect_err("non-Git project root should be rejected");

        assert_eq!(error, ApplicationError::TaskWorktreeRequiresGitRepository);
        assert!(provisioner.created_requests().is_empty());
        assert!(lease_store.created_leases().is_empty());
        assert!(workspace_commit.committed_worktree_tasks().is_empty());
    });
}

/// Verifies missing base branches retain their name and do not persist task or worktree records.
#[test]
fn rejects_missing_base_branches_without_persisting_ora_records() {
    with_trace_logging(|| {
        let workspace_commit = Rc::new(FakeWorkspaceCommit::default());
        let lease_store = FakeLeaseStore::default();
        let provisioner = Rc::new(FakeTaskWorktreeProvisioner::default());
        provisioner.fail_next_create(TaskWorktreeProvisionerError::BaseBranchNotFound {
            branch_name: "ghost-branch".to_string(),
        });
        let handler = CreateTaskHandler::new(
            workspace_commit.clone(),
            lease_store.clone(),
            FixedTaskIdGenerator::new(TASK_ID),
            FixedWorktreeIdGenerator::new("worktree-1"),
            provisioner.clone(),
            PathBuf::from(REPO_ROOT),
            PathBuf::from(WORK_DIR),
            FixedClock::new(1_700_000_000_000),
        );

        let error = handler
            .handle(CreateTaskRequest {
                project_id: "project-1".to_string(),
                title: "Cannot find base branch".to_string(),
                workspace_mode: Some(TaskWorkspaceMode::Worktree),
                base_branch: Some("ghost-branch".to_string()),
            })
            .expect_err("missing base branch should be rejected");

        assert_eq!(
            error,
            ApplicationError::TaskBaseBranchNotFound {
                branch_name: "ghost-branch".to_string(),
            }
        );
        assert!(provisioner.created_requests().is_empty());
        // The failed provisioning released its write-ahead lease to cleanup.
        assert_eq!(lease_store.released_leases().len(), 1);
        assert!(workspace_commit.committed_worktree_tasks().is_empty());
    });
}

/// Verifies worktree mode rejects an omitted base before touching Git or persistence.
#[test]
fn rejects_worktree_tasks_without_a_base_branch() {
    with_trace_logging(|| {
        let workspace_commit = Rc::new(FakeWorkspaceCommit::default());
        let lease_store = FakeLeaseStore::default();
        let provisioner = Rc::new(FakeTaskWorktreeProvisioner::default());
        let handler = CreateTaskHandler::new(
            workspace_commit.clone(),
            lease_store.clone(),
            FixedTaskIdGenerator::new(TASK_ID),
            FixedWorktreeIdGenerator::new("worktree-1"),
            provisioner.clone(),
            PathBuf::from(REPO_ROOT),
            PathBuf::from(WORK_DIR),
            FixedClock::new(1_700_000_000_000),
        );

        let error = handler
            .handle(CreateTaskRequest {
                project_id: "project-1".to_string(),
                title: "Missing base branch".to_string(),
                workspace_mode: Some(TaskWorkspaceMode::Worktree),
                base_branch: None,
            })
            .expect_err("worktree tasks should require a base branch");

        assert_eq!(error, ApplicationError::TaskBaseBranchRequired);
        assert!(provisioner.created_requests().is_empty());
        assert!(lease_store.created_leases().is_empty());
        assert!(workspace_commit.committed_worktree_tasks().is_empty());
    });
}

/// Verifies task creation regenerates ids when the short branch prefix already exists as a worktree folder.
#[test]
fn regenerates_task_ids_when_branch_prefix_folder_exists() {
    with_trace_logging(|| {
        let work_dir = unique_test_work_dir("task-prefix-collision");
        fs::create_dir_all(work_dir.join("12345678-existing-worktree"))
            .unwrap_or_else(|error| panic!("failed to create prefix collision fixture: {error}"));
        let workspace_commit = Rc::new(FakeWorkspaceCommit::default());
        let provisioner = Rc::new(FakeTaskWorktreeProvisioner::default());
        let handler = CreateTaskHandler::new(
            workspace_commit.clone(),
            FakeLeaseStore::default(),
            SequenceTaskIdGenerator::new(vec![
                "12345678-1234-5678-90ab-1234567890ab",
                "87654321-1234-5678-90ab-1234567890ab",
            ]),
            FixedWorktreeIdGenerator::new("worktree-1"),
            provisioner.clone(),
            PathBuf::from(REPO_ROOT),
            work_dir.clone(),
            FixedClock::new(1_700_000_000_000),
        );

        let response = handler
            .handle(CreateTaskRequest {
                project_id: "project-1".to_string(),
                title: "Ship handlers".to_string(),
                workspace_mode: None,
                base_branch: Some("main".to_string()),
            })
            .unwrap_or_else(|error| panic!("create handler failed: {error}"));

        assert_eq!(
            response,
            CreateTaskResponse {
                task: ContractTask {
                    id: "87654321-1234-5678-90ab-1234567890ab".to_string(),
                    project_id: "project-1".to_string(),
                    title: "Ship handlers".to_string(),
                    workspace_mode: TaskWorkspaceMode::Worktree,
                    task_type: ContractTaskType::Default,
                    workflow_run_id: None,
                },
            }
        );
        assert_eq!(
            provisioner.created_requests(),
            vec![CreateTaskWorktreeRequest {
                branch_name: "ora/87654321".to_string(),
                base_reference_name: "main".to_string(),
                worktree_path: work_dir.join("87654321-1234-5678-90ab-1234567890ab"),
            }]
        );
        assert_eq!(
            workspace_commit
                .committed_worktree_tasks()
                .into_iter()
                .map(|(task, _, _)| task.id)
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
            Rc::new(FakeWorkspaceCommit::default()),
            FakeLeaseStore::default(),
            SequenceTaskIdGenerator::new(vec![
                "12345678-1234-5678-90ab-1234567890ab",
                "87654321-1234-5678-90ab-1234567890ab",
            ]),
            FixedWorktreeIdGenerator::new("worktree-1"),
            provisioner.clone(),
            PathBuf::from(REPO_ROOT),
            work_dir.clone(),
            FixedClock::new(1_700_000_000_000),
        );

        let response = handler
            .handle(CreateTaskRequest {
                project_id: "project-1".to_string(),
                title: "Ship handlers".to_string(),
                workspace_mode: None,
                base_branch: Some("main".to_string()),
            })
            .unwrap_or_else(|error| panic!("create handler failed: {error}"));

        assert_eq!(
            response,
            CreateTaskResponse {
                task: ContractTask {
                    id: "87654321-1234-5678-90ab-1234567890ab".to_string(),
                    project_id: "project-1".to_string(),
                    title: "Ship handlers".to_string(),
                    workspace_mode: TaskWorkspaceMode::Worktree,
                    task_type: ContractTaskType::Default,
                    workflow_run_id: None,
                },
            }
        );
        assert_eq!(
            provisioner.created_requests(),
            vec![CreateTaskWorktreeRequest {
                branch_name: "ora/87654321".to_string(),
                base_reference_name: "main".to_string(),
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
            Rc::new(FakeWorkspaceCommit::default()),
            FakeLeaseStore::default(),
            FixedTaskIdGenerator::new(TASK_ID),
            FixedWorktreeIdGenerator::new("worktree-1"),
            provisioner.clone(),
            PathBuf::from(REPO_ROOT),
            work_dir.clone(),
            FixedClock::new(1_700_000_000_000),
        );

        let response = handler
            .handle(CreateTaskRequest {
                project_id: "project-1".to_string(),
                title: "Ship handlers".to_string(),
                workspace_mode: None,
                base_branch: Some("main".to_string()),
            })
            .unwrap_or_else(|error| panic!("create handler failed: {error}"));

        assert_eq!(
            response,
            CreateTaskResponse {
                task: ContractTask {
                    id: TASK_ID.to_string(),
                    project_id: "project-1".to_string(),
                    title: "Ship handlers".to_string(),
                    workspace_mode: TaskWorkspaceMode::Worktree,
                    task_type: ContractTaskType::Default,
                    workflow_run_id: None,
                },
            }
        );
        assert_eq!(
            provisioner.created_requests(),
            vec![CreateTaskWorktreeRequest {
                branch_name: "ora/12345678".to_string(),
                base_reference_name: "main".to_string(),
                worktree_path: work_dir.join(TASK_ID),
            }]
        );
    });
}

/// Verifies repeated branch-prefix collisions return a stable application error.
#[test]
fn reports_task_worktree_error_when_task_id_retries_are_exhausted() {
    let work_dir = unique_test_work_dir("task-prefix-exhaustion");
    fs::create_dir_all(work_dir.join("12345678-existing-worktree"))
        .unwrap_or_else(|error| panic!("failed to create prefix collision fixture: {error}"));
    let handler = CreateTaskHandler::new(
        Rc::new(FakeWorkspaceCommit::default()),
        FakeLeaseStore::default(),
        FixedTaskIdGenerator::new(TASK_ID),
        FixedWorktreeIdGenerator::new("worktree-1"),
        Rc::new(FakeTaskWorktreeProvisioner::default()),
        PathBuf::from(REPO_ROOT),
        work_dir.clone(),
        FixedClock::new(1_700_000_000_000),
    );

    assert_eq!(
        handler
            .handle(CreateTaskRequest {
                project_id: "project-1".to_string(),
                title: "Ship handlers".to_string(),
                workspace_mode: None,
                base_branch: Some("main".to_string()),
            })
            .unwrap_err(),
        ApplicationError::TaskWorktreeIdExhausted { attempts: 3 }
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
                    workspace_mode: TaskWorkspaceMode::ProjectRoot,
                    task_type: ContractTaskType::Default,
                    workflow_run_id: None,
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
                None,
                AuditFields::new(1, 2, false),
            ),
            Task::new(
                TaskId::new("task-2"),
                ProjectId::new("project-2"),
                "Wire exports",
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
                        workspace_mode: TaskWorkspaceMode::ProjectRoot,
                        task_type: ContractTaskType::Default,
                        workflow_run_id: None,
                    },
                    ContractTask {
                        id: "task-2".to_string(),
                        project_id: "project-2".to_string(),
                        title: "Wire exports".to_string(),
                        workspace_mode: TaskWorkspaceMode::Worktree,
                        task_type: ContractTaskType::Default,
                        workflow_run_id: None,
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
            None,
            AuditFields::new(10, 20, false),
        )]));
        let handler = UpdateTaskHandler::new(repository.clone(), FixedClock::new(30));

        let response = handler
            .handle(UpdateTaskRequest {
                task_id: "task-1".to_string(),
                title: "Ship updated handlers".to_string(),
            })
            .unwrap_or_else(|error| panic!("update handler failed: {error}"));

        assert_eq!(
            response,
            UpdateTaskResponse {
                task: ContractTask {
                    id: "task-1".to_string(),
                    project_id: "project-1".to_string(),
                    title: "Ship updated handlers".to_string(),
                    workspace_mode: TaskWorkspaceMode::ProjectRoot,
                    task_type: ContractTaskType::Default,
                    workflow_run_id: None,
                },
            }
        );
        assert_eq!(
            repository.visible_tasks(),
            vec![Task::new(
                TaskId::new("task-1"),
                ProjectId::new("project-1"),
                "Ship updated handlers",
                None,
                AuditFields::new(10, 30, false),
            )]
        );
    });
}

/// Verifies a failed workspace commit releases the lease to durable cleanup.
#[test]
fn releases_lease_to_cleanup_when_workspace_commit_fails() {
    with_trace_logging(|| {
        let workspace_commit = Rc::new(FakeWorkspaceCommit::default());
        let lease_store = FakeLeaseStore::default();
        let provisioner = Rc::new(FakeTaskWorktreeProvisioner::default());
        workspace_commit.fail_next(RepositoryError::from_message(
            "task write failed".to_string(),
        ));
        let handler = CreateTaskHandler::new(
            workspace_commit.clone(),
            lease_store.clone(),
            FixedTaskIdGenerator::new(TASK_ID),
            FixedWorktreeIdGenerator::new("worktree-1"),
            provisioner.clone(),
            PathBuf::from(REPO_ROOT),
            PathBuf::from(WORK_DIR),
            FixedClock::new(50),
        );

        let error = handler
            .handle(CreateTaskRequest {
                project_id: "project-1".to_string(),
                title: "Ship handlers".to_string(),
                workspace_mode: None,
                base_branch: Some("main".to_string()),
            })
            .unwrap_err();

        assert_eq!(
            error,
            ApplicationError::TaskRepository {
                source: RepositoryError::from_message("task write failed"),
            }
        );
        let created = lease_store.created_leases();
        assert_eq!(
            lease_store.released_leases(),
            vec![created[0].id.clone()],
            "the provisioned Git resources must be handed to durable cleanup"
        );
    });
}

/// Verifies losing the race to a project deletion reports project-not-found
/// and releases the provisioned resources to durable cleanup.
#[test]
fn releases_lease_to_cleanup_when_project_was_deleted_concurrently() {
    with_trace_logging(|| {
        let workspace_commit = Rc::new(FakeWorkspaceCommit::default());
        let lease_store = FakeLeaseStore::default();
        let provisioner = Rc::new(FakeTaskWorktreeProvisioner::default());
        workspace_commit.reject_next_as_project_not_visible();
        let handler = CreateTaskHandler::new(
            workspace_commit.clone(),
            lease_store.clone(),
            FixedTaskIdGenerator::new(TASK_ID),
            FixedWorktreeIdGenerator::new("worktree-1"),
            provisioner.clone(),
            PathBuf::from(REPO_ROOT),
            PathBuf::from(WORK_DIR),
            FixedClock::new(50),
        );

        let error = handler
            .handle(CreateTaskRequest {
                project_id: "project-1".to_string(),
                title: "Ship handlers".to_string(),
                workspace_mode: None,
                base_branch: Some("main".to_string()),
            })
            .unwrap_err();

        assert_eq!(
            error,
            ApplicationError::ProjectNotFound {
                project_id: "project-1".to_string(),
            }
        );
        let created = lease_store.created_leases();
        assert_eq!(lease_store.released_leases(), vec![created[0].id.clone()]);
    });
}

/// Verifies a deleted project rejects new project-root tasks atomically.
#[test]
fn rejects_project_root_tasks_when_project_was_deleted_concurrently() {
    with_trace_logging(|| {
        let workspace_commit = Rc::new(FakeWorkspaceCommit::default());
        workspace_commit.reject_next_as_project_not_visible();
        let handler = CreateTaskHandler::new(
            workspace_commit.clone(),
            FakeLeaseStore::default(),
            FixedTaskIdGenerator::new(TASK_ID),
            FixedWorktreeIdGenerator::new("worktree-1"),
            Rc::new(FakeTaskWorktreeProvisioner::default()),
            PathBuf::from(REPO_ROOT),
            PathBuf::from(WORK_DIR),
            FixedClock::new(50),
        );

        let error = handler
            .handle(CreateTaskRequest {
                project_id: "project-1".to_string(),
                title: "Chat in project root".to_string(),
                workspace_mode: Some(TaskWorkspaceMode::ProjectRoot),
                base_branch: None,
            })
            .unwrap_err();

        assert_eq!(
            error,
            ApplicationError::ProjectNotFound {
                project_id: "project-1".to_string(),
            }
        );
    });
}

/// Verifies provisioning failures become stable application errors before any persistence occurs.
#[test]
fn reports_application_errors() {
    with_trace_logging(|| {
        let missing_repository = Rc::new(FakeTaskRepository::default());
        let get_handler = GetTaskHandler::new(missing_repository);
        let provisioner = Rc::new(FakeTaskWorktreeProvisioner::default());
        provisioner.fail_next_create(TaskWorktreeProvisionerError::operation_failed(
            "failed to create task worktree",
            std::io::Error::other("failed to create linked worktree"),
        ));
        let create_handler = CreateTaskHandler::new(
            Rc::new(FakeWorkspaceCommit::default()),
            FakeLeaseStore::default(),
            FixedTaskIdGenerator::new(TASK_ID),
            FixedWorktreeIdGenerator::new("worktree-1"),
            provisioner,
            PathBuf::from(REPO_ROOT),
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
                workspace_mode: None,
                base_branch: Some("main".to_string()),
            })
            .unwrap_err();

        assert_eq!(
            missing_error,
            ApplicationError::TaskNotFound {
                task_id: "missing".to_string(),
            }
        );
        assert!(matches!(
            provisioning_error,
            ApplicationError::TaskWorktreeProvisioner { .. }
        ));
    });
}

#[derive(Debug, Default)]
struct FakeTaskRepository {
    tasks: RefCell<Vec<Task>>,
    next_error: RefCell<Option<RepositoryError>>,
}

impl FakeTaskRepository {
    /// Builds a fake repository seeded with the provided task rows.
    fn with_tasks(tasks: Vec<Task>) -> Self {
        Self {
            tasks: RefCell::new(tasks),
            next_error: RefCell::new(None),
        }
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
    fn take_error(&self) -> Result<(), RepositoryError> {
        match self.next_error.borrow_mut().take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl TaskRepository for Rc<FakeTaskRepository> {
    fn create_task(&self, task: Task) -> Result<Task, RepositoryError> {
        self.take_error()?;
        self.tasks.borrow_mut().push(task.clone());
        Ok(task)
    }

    fn find_task(&self, task_id: &TaskId) -> Result<Option<Task>, RepositoryError> {
        self.take_error()?;

        Ok(self
            .tasks
            .borrow()
            .iter()
            .find(|task| task.id == *task_id && !task.audit_fields.is_deleted)
            .cloned())
    }

    fn list_tasks(&self) -> Result<Vec<Task>, RepositoryError> {
        self.take_error()?;
        Ok(self.visible_tasks())
    }

    fn update_task(&self, task: Task) -> Result<Task, RepositoryError> {
        self.take_error()?;

        let mut tasks = self.tasks.borrow_mut();
        if let Some(existing_task) = tasks.iter_mut().find(|existing_task| {
            existing_task.id == task.id && !existing_task.audit_fields.is_deleted
        }) {
            *existing_task = task.clone();
            Ok(task)
        } else {
            Err(RepositoryError::from_message(format!(
                "missing task during update: {}",
                task.id
            )))
        }
    }

    fn soft_delete_task(&self, task_id: &TaskId, deleted_at: i64) -> Result<bool, RepositoryError> {
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
    next_error: RefCell<Option<RepositoryError>>,
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
    fn take_error(&self) -> Result<(), RepositoryError> {
        match self.next_error.borrow_mut().take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl WorktreeRepository for Rc<FakeWorktreeRepository> {
    fn create_worktree(&self, worktree: Worktree) -> Result<Worktree, RepositoryError> {
        self.take_error()?;
        self.worktrees.borrow_mut().push(worktree.clone());
        Ok(worktree)
    }

    fn find_worktree(&self, worktree_id: &WorktreeId) -> Result<Option<Worktree>, RepositoryError> {
        self.take_error()?;

        Ok(self
            .worktrees
            .borrow()
            .iter()
            .find(|worktree| worktree.id == *worktree_id && !worktree.audit_fields.is_deleted)
            .cloned())
    }

    fn list_worktrees(&self) -> Result<Vec<Worktree>, RepositoryError> {
        self.take_error()?;
        Ok(self.visible_worktrees())
    }

    fn update_worktree(&self, worktree: Worktree) -> Result<Worktree, RepositoryError> {
        self.take_error()?;

        let mut worktrees = self.worktrees.borrow_mut();
        if let Some(existing_worktree) = worktrees.iter_mut().find(|existing_worktree| {
            existing_worktree.id == worktree.id && !existing_worktree.audit_fields.is_deleted
        }) {
            *existing_worktree = worktree.clone();
            Ok(worktree)
        } else {
            Err(RepositoryError::from_message(format!(
                "missing worktree during update: {}",
                worktree.id
            )))
        }
    }

    fn soft_delete_worktree(
        &self,
        worktree_id: &WorktreeId,
        deleted_at: i64,
    ) -> Result<bool, RepositoryError> {
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

#[derive(Clone, Copy)]
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

/// Records workspace commits so tests can assert atomic-persistence inputs.
#[derive(Debug, Default)]
struct FakeWorkspaceCommit {
    worktree_tasks: RefCell<Vec<(Task, Worktree, WorktreeProvisioningLeaseId)>>,
    project_root_tasks: RefCell<Vec<Task>>,
    next_error: RefCell<Option<RepositoryError>>,
    reject_next: RefCell<bool>,
}

impl FakeWorkspaceCommit {
    /// Configures the next commit call to fail with a deterministic error.
    fn fail_next(&self, error: RepositoryError) {
        self.next_error.replace(Some(error));
    }

    /// Configures the next commit call to lose against a project deletion.
    fn reject_next_as_project_not_visible(&self) {
        self.reject_next.replace(true);
    }

    /// Returns every committed worktree task with its worktree and lease id.
    fn committed_worktree_tasks(&self) -> Vec<(Task, Worktree, WorktreeProvisioningLeaseId)> {
        self.worktree_tasks.borrow().clone()
    }

    /// Returns every committed project-root task.
    fn committed_project_root_tasks(&self) -> Vec<Task> {
        self.project_root_tasks.borrow().clone()
    }

    /// Applies the queued failure or rejection, if any.
    fn take_outcome(&self) -> Result<Option<WorkspaceCommitOutcome>, RepositoryError> {
        if let Some(error) = self.next_error.borrow_mut().take() {
            return Err(error);
        }
        if self.reject_next.replace(false) {
            return Ok(Some(WorkspaceCommitOutcome::ProjectNotVisible));
        }
        Ok(None)
    }
}

impl TaskWorkspaceCommit for Rc<FakeWorkspaceCommit> {
    fn commit_worktree_task(
        &self,
        task: &Task,
        worktree: &Worktree,
        lease_id: &WorktreeProvisioningLeaseId,
    ) -> Result<WorkspaceCommitOutcome, RepositoryError> {
        if let Some(outcome) = self.take_outcome()? {
            return Ok(outcome);
        }
        self.worktree_tasks
            .borrow_mut()
            .push((task.clone(), worktree.clone(), lease_id.clone()));
        Ok(WorkspaceCommitOutcome::Committed)
    }

    fn commit_project_root_task(
        &self,
        task: &Task,
    ) -> Result<WorkspaceCommitOutcome, RepositoryError> {
        if let Some(outcome) = self.take_outcome()? {
            return Ok(outcome);
        }
        self.project_root_tasks.borrow_mut().push(task.clone());
        Ok(WorkspaceCommitOutcome::Committed)
    }
}

/// Records lease lifecycle calls; `Arc`-shared because renewal runs on a thread.
#[derive(Clone, Debug, Default)]
struct FakeLeaseStore {
    state: Arc<Mutex<FakeLeaseStoreState>>,
}

#[derive(Debug, Default)]
struct FakeLeaseStoreState {
    created: Vec<WorktreeProvisioningLease>,
    released: Vec<WorktreeProvisioningLeaseId>,
}

impl FakeLeaseStore {
    /// Returns every lease created through the store.
    fn created_leases(&self) -> Vec<WorktreeProvisioningLease> {
        self.state
            .lock()
            .expect("lease store state")
            .created
            .clone()
    }

    /// Returns every lease id released to durable cleanup.
    fn released_leases(&self) -> Vec<WorktreeProvisioningLeaseId> {
        self.state
            .lock()
            .expect("lease store state")
            .released
            .clone()
    }
}

impl WorktreeProvisioningLeaseStore for FakeLeaseStore {
    fn create_lease(&self, lease: &WorktreeProvisioningLease) -> Result<(), RepositoryError> {
        self.state
            .lock()
            .expect("lease store state")
            .created
            .push(lease.clone());
        Ok(())
    }

    fn renew_lease(
        &self,
        _lease_id: &WorktreeProvisioningLeaseId,
        _lease_expires_at: i64,
        _now: i64,
    ) -> Result<bool, RepositoryError> {
        Ok(true)
    }

    fn release_to_cleanup(
        &self,
        lease_id: &WorktreeProvisioningLeaseId,
        _now: i64,
    ) -> Result<(), RepositoryError> {
        self.state
            .lock()
            .expect("lease store state")
            .released
            .push(lease_id.clone());
        Ok(())
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
