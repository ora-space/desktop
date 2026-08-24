use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::WorkflowRunCreateOutcome;
use crate::task::WorktreeProvisioningLeaseStore;
use crate::task::{
    CreateTaskWorktreeRequest, CreateTaskWorktreeResponse, DeleteTaskWorktreeRequest,
    TaskIdGenerator, TaskWorktreeProvisioner, TaskWorktreeProvisionerError,
};
use crate::workflow::WorkflowRepository;
use crate::workflow_run::handlers::{
    CreateWorkflowRunHandler, DeleteWorkflowRunHandler, GetWorkflowRunHandler,
    ListWorkflowNodeRunsHandler, ListWorkflowRunsByWorkflowHandler, ListWorkflowRunsHandler,
};
use crate::workflow_run::mapper::{map_node_run, map_run, map_run_summary};
use crate::workflow_run::{DeleteWorkflowRunResult, WorkflowRunIdGenerator, WorkflowRunRepository};
use crate::worktree::WorktreeIdGenerator;
use crate::{
    ApplicationError, Clock, RepositoryError, SkillMaterializationReceipt, StartPrerequisitesError,
    WorkflowGraph, WorkflowRunWorktreeInitializer,
};
use ora_contracts::{
    CreateWorkflowRunRequest, DeleteWorkflowRunRequest, DeleteWorkflowRunResponse,
    GetWorkflowRunRequest, GetWorkflowRunResponse, ListWorkflowNodeRunsRequest,
    ListWorkflowNodeRunsResponse, ListWorkflowRunsByWorkflowRequest,
    ListWorkflowRunsByWorkflowResponse, ListWorkflowRunsRequest, ListWorkflowRunsResponse,
    WorkflowRunLocale, WorkflowRunStatus as ContractRunStatus,
};
use ora_domain::Namespace;
use ora_domain::{
    AuditFields, CreatedWorkflow, ProjectId, Task, TaskId, Workflow, WorkflowDetail, WorkflowId,
    WorkflowNodeRun, WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRun, WorkflowRunDetail,
    WorkflowRunId, WorkflowRunStatus, WorkflowRunSummary, WorkflowSnapshot, WorkflowSnapshotId,
    WorkflowSummary, WorkflowVersion, Worktree, WorktreeId,
};
use ora_domain::{WorktreeProvisioningLease, WorktreeProvisioningLeaseId};
use pretty_assertions::assert_eq;

const TASK_ID: &str = "12345678-1234-5678-90ab-1234567890ab";
const WORK_DIR: &str = "/tmp/ora-worktrees";
const REPO_ROOT: &str = "/repos/project-1";

/// Verifies a run is created against an explicitly provided published snapshot.
#[test]
fn creates_run_with_explicit_snapshot() {
    let workflow = workflow_fixture(Some("snapshot-a"));
    let snapshot = snapshot_fixture("snapshot-a", "v1");
    let workflow_repository = MockWorkflowRepository::with(workflow, vec![snapshot.clone()]);
    let run_repository = MockWorkflowRunRepository::default();
    let provisioner = Arc::new(FakeTaskWorktreeProvisioner::default());
    let handler = CreateWorkflowRunHandler::new(
        Arc::new(workflow_repository),
        Arc::new(run_repository),
        FixedRunIdGenerator::new("run-1"),
        FixedTaskIdGenerator::new(TASK_ID),
        FixedWorktreeIdGenerator::new("worktree-1"),
        provisioner.clone(),
        MockWorktreeInitializer::default(),
        FakeLeaseStore::default(),
        PathBuf::from(REPO_ROOT),
        PathBuf::from(WORK_DIR),
        FixedClock::new(30),
    );

    let response = handler
        .handle(CreateWorkflowRunRequest {
            project_id: "project-1".to_string(),
            workflow_id: "workflow-a".to_string(),
            locale: WorkflowRunLocale::ZhCn,
            snapshot_id: Some("snapshot-a".to_string()),
            kickoff_input: Some("kickoff".to_string()),
            name: None,
            base_branch: None,
        })
        .unwrap();

    assert_eq!(
        response.run,
        map_run(WorkflowRun::new(
            WorkflowRunId::new("run-1"),
            WorkflowId::new("workflow-a"),
            WorkflowSnapshotId::new("snapshot-a"),
            WorkflowRunStatus::Pending,
            Some("{\"current_nodes\":[]}".to_string()),
            Some("kickoff".to_string()),
            None,
            None,
            Some(r#"{"locale":"zh-CN","skillMaterialization":{"bindings":[]}}"#.to_string(),),
            None,
            None,
            AuditFields::new(30, 30, /*is_deleted*/ false),
        ))
    );
    assert_eq!(response.task_id, TASK_ID.to_string());
    assert_eq!(
        provisioner.created_requests(),
        vec![CreateTaskWorktreeRequest {
            branch_name: format!("ora/{}", &TASK_ID[..8]),
            base_reference_name: "main".to_string(),
            worktree_path: PathBuf::from(WORK_DIR).join(TASK_ID),
        }]
    );
}

/// Verifies deployment inherits the frozen Start instruction when no kickoff override is supplied.
#[test]
fn defaults_kickoff_input_to_frozen_start_instruction() {
    let workflow = workflow_fixture(Some("snapshot-a"));
    let snapshot = WorkflowSnapshot::new(
        WorkflowSnapshotId::new("snapshot-a"),
        WorkflowId::new("workflow-a"),
        "v1",
        r#"{"nodes":[{"id":"start","data":{"kind":"start","title":"Start","instruction":"Review the repository"}}],"edges":[]}"#,
        20,
        Some(20),
        /*is_deleted*/ false,
    );
    let workflow_repository = MockWorkflowRepository::with(workflow, vec![snapshot]);
    let run_repository = MockWorkflowRunRepository::default();
    let handler = CreateWorkflowRunHandler::new(
        Arc::new(workflow_repository),
        Arc::new(run_repository),
        FixedRunIdGenerator::new("run-1"),
        FixedTaskIdGenerator::new(TASK_ID),
        FixedWorktreeIdGenerator::new("worktree-1"),
        Arc::new(FakeTaskWorktreeProvisioner::default()),
        MockWorktreeInitializer::default(),
        FakeLeaseStore::default(),
        PathBuf::from(REPO_ROOT),
        PathBuf::from(WORK_DIR),
        FixedClock::new(30),
    );

    let response = handler
        .handle(CreateWorkflowRunRequest {
            project_id: "project-1".to_string(),
            workflow_id: "workflow-a".to_string(),
            locale: WorkflowRunLocale::ZhCn,
            snapshot_id: Some("snapshot-a".to_string()),
            kickoff_input: None,
            name: None,
            base_branch: None,
        })
        .unwrap();

    assert_eq!(
        response.run,
        map_run(WorkflowRun::new(
            WorkflowRunId::new("run-1"),
            WorkflowId::new("workflow-a"),
            WorkflowSnapshotId::new("snapshot-a"),
            WorkflowRunStatus::Pending,
            Some("{\"current_nodes\":[]}".to_string()),
            Some("Review the repository".to_string()),
            None,
            None,
            Some(r#"{"locale":"zh-CN","skillMaterialization":{"bindings":[]}}"#.to_string(),),
            None,
            None,
            AuditFields::new(30, 30, /*is_deleted*/ false),
        ))
    );
}

/// Verifies a failing worktree initialization aborts creation and hands the
/// provisioned resources to durable cleanup.
#[test]
fn fails_creation_and_releases_lease_when_worktree_initialization_fails() {
    let workflow = workflow_fixture(Some("snapshot-a"));
    let snapshot = snapshot_fixture("snapshot-a", "v1");
    let workflow_repository = MockWorkflowRepository::with(workflow, vec![snapshot]);
    let run_repository = MockWorkflowRunRepository::default();
    let provisioner = Arc::new(FakeTaskWorktreeProvisioner::default());
    let lease_store = FakeLeaseStore::default();
    let handler = CreateWorkflowRunHandler::new(
        Arc::new(workflow_repository),
        Arc::new(run_repository),
        FixedRunIdGenerator::new("run-1"),
        FixedTaskIdGenerator::new(TASK_ID),
        FixedWorktreeIdGenerator::new("worktree-1"),
        provisioner.clone(),
        MockWorktreeInitializer {
            worktrees: Arc::new(Mutex::new(Vec::new())),
            fail: true,
        },
        lease_store.clone(),
        PathBuf::from(REPO_ROOT),
        PathBuf::from(WORK_DIR),
        FixedClock::new(30),
    );

    let error = handler
        .handle(CreateWorkflowRunRequest {
            project_id: "project-1".to_string(),
            workflow_id: "workflow-a".to_string(),
            locale: WorkflowRunLocale::ZhCn,
            snapshot_id: Some("snapshot-a".to_string()),
            kickoff_input: None,
            name: None,
            base_branch: None,
        })
        .unwrap_err();

    assert!(matches!(
        error,
        ApplicationError::WorkflowRunStartFailed { .. }
    ));
    // The provisioned resources are handed to durable cleanup via the lease.
    let created = lease_store.created_leases();
    assert_eq!(lease_store.released_leases(), vec![created[0].id.clone()]);
}

/// Verifies creation falls back to the workflow's published snapshot without an explicit id.
#[test]
fn uses_published_snapshot_when_no_explicit_id() {
    let workflow = workflow_fixture(Some("snapshot-a"));
    let snapshot = snapshot_fixture("snapshot-a", "v1");
    let workflow_repository = MockWorkflowRepository::with(workflow, vec![snapshot]);
    let run_repository = MockWorkflowRunRepository::default();
    let handler = CreateWorkflowRunHandler::new(
        Arc::new(workflow_repository),
        Arc::new(run_repository),
        FixedRunIdGenerator::new("run-1"),
        FixedTaskIdGenerator::new(TASK_ID),
        FixedWorktreeIdGenerator::new("worktree-1"),
        Arc::new(FakeTaskWorktreeProvisioner::default()),
        MockWorktreeInitializer::default(),
        FakeLeaseStore::default(),
        PathBuf::from(REPO_ROOT),
        PathBuf::from(WORK_DIR),
        FixedClock::new(30),
    );

    let response = handler
        .handle(CreateWorkflowRunRequest {
            project_id: "project-1".to_string(),
            workflow_id: "workflow-a".to_string(),
            locale: WorkflowRunLocale::ZhCn,
            snapshot_id: None,
            kickoff_input: None,
            name: Some("Manual name".to_string()),
            base_branch: None,
        })
        .unwrap();

    assert_eq!(response.run.snapshot_id, "snapshot-a");
    assert_eq!(response.run.status, ContractRunStatus::Pending);
}

/// Verifies a workflow without a published snapshot rejects creation when no id is supplied.
#[test]
fn rejects_workflow_without_published_snapshot() {
    let workflow_repository = MockWorkflowRepository::with(workflow_fixture(None), Vec::new());
    let handler = CreateWorkflowRunHandler::new(
        Arc::new(workflow_repository),
        Arc::new(MockWorkflowRunRepository::default()),
        FixedRunIdGenerator::new("run-1"),
        FixedTaskIdGenerator::new(TASK_ID),
        FixedWorktreeIdGenerator::new("worktree-1"),
        Arc::new(FakeTaskWorktreeProvisioner::default()),
        MockWorktreeInitializer::default(),
        FakeLeaseStore::default(),
        PathBuf::from(REPO_ROOT),
        PathBuf::from(WORK_DIR),
        FixedClock::new(30),
    );

    let error = handler
        .handle(CreateWorkflowRunRequest {
            project_id: "project-1".to_string(),
            workflow_id: "workflow-a".to_string(),
            locale: WorkflowRunLocale::ZhCn,
            snapshot_id: None,
            kickoff_input: None,
            name: None,
            base_branch: None,
        })
        .unwrap_err();

    assert_eq!(error, ApplicationError::WorkflowNoPublishedSnapshot);
}

/// Verifies the workflow draft cannot be frozen by a run.
#[test]
fn rejects_draft_snapshot() {
    let workflow = workflow_fixture(Some("snapshot-a"));
    let draft = snapshot_fixture("draft-1", "draft");
    let workflow_repository = MockWorkflowRepository::with(workflow, vec![draft]);
    let handler = CreateWorkflowRunHandler::new(
        Arc::new(workflow_repository),
        Arc::new(MockWorkflowRunRepository::default()),
        FixedRunIdGenerator::new("run-1"),
        FixedTaskIdGenerator::new(TASK_ID),
        FixedWorktreeIdGenerator::new("worktree-1"),
        Arc::new(FakeTaskWorktreeProvisioner::default()),
        MockWorktreeInitializer::default(),
        FakeLeaseStore::default(),
        PathBuf::from(REPO_ROOT),
        PathBuf::from(WORK_DIR),
        FixedClock::new(30),
    );

    let error = handler
        .handle(CreateWorkflowRunRequest {
            project_id: "project-1".to_string(),
            workflow_id: "workflow-a".to_string(),
            locale: WorkflowRunLocale::ZhCn,
            snapshot_id: Some("draft-1".to_string()),
            kickoff_input: None,
            name: None,
            base_branch: None,
        })
        .unwrap_err();

    assert_eq!(error, ApplicationError::WorkflowRunCannotUseDraftSnapshot);
}

/// Verifies a snapshot that does not belong to the workflow is rejected before any provisioning.
#[test]
fn rejects_snapshot_not_in_workflow() {
    let workflow_repository = MockWorkflowRepository::with(workflow_fixture(None), Vec::new());
    let provisioner = Arc::new(FakeTaskWorktreeProvisioner::default());
    let handler = CreateWorkflowRunHandler::new(
        Arc::new(workflow_repository),
        Arc::new(MockWorkflowRunRepository::default()),
        FixedRunIdGenerator::new("run-1"),
        FixedTaskIdGenerator::new(TASK_ID),
        FixedWorktreeIdGenerator::new("worktree-1"),
        provisioner.clone(),
        MockWorktreeInitializer::default(),
        FakeLeaseStore::default(),
        PathBuf::from(REPO_ROOT),
        PathBuf::from(WORK_DIR),
        FixedClock::new(30),
    );

    let error = handler
        .handle(CreateWorkflowRunRequest {
            project_id: "project-1".to_string(),
            workflow_id: "workflow-a".to_string(),
            locale: WorkflowRunLocale::ZhCn,
            snapshot_id: Some("snapshot-missing".to_string()),
            kickoff_input: None,
            name: None,
            base_branch: None,
        })
        .unwrap_err();

    assert_eq!(
        error,
        ApplicationError::WorkflowSnapshotNotFoundById {
            snapshot_id: "snapshot-missing".to_string(),
        }
    );
    assert!(provisioner.created_requests().is_empty());
}

/// Verifies a persistence failure hands the provisioned worktree to durable cleanup.
#[test]
fn releases_lease_when_persistence_fails() {
    let workflow = workflow_fixture(Some("snapshot-a"));
    let snapshot = snapshot_fixture("snapshot-a", "v1");
    let workflow_repository = MockWorkflowRepository::with(workflow, vec![snapshot]);
    let run_repository = MockWorkflowRunRepository::default();
    run_repository.fail_next_create(RepositoryError::from_message(
        "run write failed".to_string(),
    ));
    let provisioner = Arc::new(FakeTaskWorktreeProvisioner::default());
    let lease_store = FakeLeaseStore::default();
    let handler = CreateWorkflowRunHandler::new(
        Arc::new(workflow_repository),
        Arc::new(run_repository),
        FixedRunIdGenerator::new("run-1"),
        FixedTaskIdGenerator::new(TASK_ID),
        FixedWorktreeIdGenerator::new("worktree-1"),
        provisioner.clone(),
        MockWorktreeInitializer::default(),
        lease_store.clone(),
        PathBuf::from(REPO_ROOT),
        PathBuf::from(WORK_DIR),
        FixedClock::new(30),
    );

    let error = handler
        .handle(CreateWorkflowRunRequest {
            project_id: "project-1".to_string(),
            workflow_id: "workflow-a".to_string(),
            locale: WorkflowRunLocale::ZhCn,
            snapshot_id: Some("snapshot-a".to_string()),
            kickoff_input: None,
            name: None,
            base_branch: None,
        })
        .unwrap_err();

    assert_eq!(
        error,
        ApplicationError::WorkflowRunRepository {
            source: RepositoryError::from_message("run write failed"),
        }
    );
    let created = lease_store.created_leases();
    assert_eq!(lease_store.released_leases(), vec![created[0].id.clone()]);
}

/// Verifies a worktree provisioning failure aborts creation before any persistence.
#[test]
fn reports_provisioning_failure() {
    let workflow = workflow_fixture(Some("snapshot-a"));
    let snapshot = snapshot_fixture("snapshot-a", "v1");
    let workflow_repository = MockWorkflowRepository::with(workflow, vec![snapshot]);
    let run_repository = Arc::new(MockWorkflowRunRepository::default());
    let provisioner = Arc::new(FakeTaskWorktreeProvisioner::default());
    provisioner.fail_next_create(TaskWorktreeProvisionerError::operation_failed(
        "failed to create workflow run worktree",
        std::io::Error::other("failed to create linked worktree"),
    ));
    let handler = CreateWorkflowRunHandler::new(
        Arc::new(workflow_repository),
        run_repository.clone(),
        FixedRunIdGenerator::new("run-1"),
        FixedTaskIdGenerator::new(TASK_ID),
        FixedWorktreeIdGenerator::new("worktree-1"),
        provisioner.clone(),
        MockWorktreeInitializer::default(),
        FakeLeaseStore::default(),
        PathBuf::from(REPO_ROOT),
        PathBuf::from(WORK_DIR),
        FixedClock::new(30),
    );

    let error = handler
        .handle(CreateWorkflowRunRequest {
            project_id: "project-1".to_string(),
            workflow_id: "workflow-a".to_string(),
            locale: WorkflowRunLocale::ZhCn,
            snapshot_id: Some("snapshot-a".to_string()),
            kickoff_input: None,
            name: None,
            base_branch: None,
        })
        .unwrap_err();

    assert!(matches!(
        error,
        ApplicationError::TaskWorktreeProvisioner { .. }
    ));
    // No run rows were persisted and no compensation delete ran for a never-created worktree.
    assert!(run_repository.created_runs().is_empty());
    assert!(provisioner.deleted_requests().is_empty());
}

/// Verifies a missing run detail reports not found.
#[test]
fn reports_not_found_on_get() {
    let handler = GetWorkflowRunHandler::new(Arc::new(MockWorkflowRunRepository::default()));

    let error = handler
        .handle(GetWorkflowRunRequest {
            run_id: "run-missing".to_string(),
        })
        .unwrap_err();

    assert_eq!(
        error,
        ApplicationError::WorkflowRunNotFound {
            run_id: "run-missing".to_string(),
        }
    );
}

/// Verifies a run detail is returned with its display name and node runs.
#[test]
fn gets_run_detail() {
    let run = run_fixture("run-1");
    let node = node_fixture("node-1", "run-1");
    let repository = MockWorkflowRunRepository::default();
    repository.with_detail(WorkflowRunDetail {
        run: run.clone(),
        name: "Manual name".to_string(),
        project_id: ProjectId::new("project-1"),
        task_id: TaskId::new("task-1".to_string()),
        nodes: vec![node.clone()],
    });
    let handler = GetWorkflowRunHandler::new(Arc::new(repository));

    let response = handler
        .handle(GetWorkflowRunRequest {
            run_id: "run-1".to_string(),
        })
        .unwrap();

    assert_eq!(
        response,
        GetWorkflowRunResponse {
            run: map_run(run),
            name: "Manual name".to_string(),
            project_id: "project-1".to_string(),
            task_id: "task-1".to_string(),
            nodes: vec![map_node_run(node)],
        }
    );
}

/// Verifies run summaries are listed for a project in repository order.
#[test]
fn lists_runs_by_project() {
    let summary = WorkflowRunSummary {
        id: WorkflowRunId::new("run-1"),
        name: "Manual name".to_string(),
        project_id: ProjectId::new("project-1"),
        workflow_id: WorkflowId::new("workflow-a"),
        status: WorkflowRunStatus::Pending,
        has_awaiting_node: false,
        started_at: None,
        finished_at: None,
        created_at: 30,
    };
    let repository = MockWorkflowRunRepository::default();
    repository.with_summaries(vec![summary.clone()]);
    let handler = ListWorkflowRunsHandler::new(Arc::new(repository));

    let response = handler
        .handle(ListWorkflowRunsRequest {
            project_id: "project-1".to_string(),
        })
        .unwrap();

    assert_eq!(
        response,
        ListWorkflowRunsResponse {
            runs: vec![map_run_summary(summary)],
        }
    );
}

/// Verifies run summaries are listed for a workflow in repository order.
#[test]
fn lists_runs_by_workflow() {
    let summary = WorkflowRunSummary {
        id: WorkflowRunId::new("run-1"),
        name: "Manual name".to_string(),
        project_id: ProjectId::new("project-1"),
        workflow_id: WorkflowId::new("workflow-a"),
        status: WorkflowRunStatus::Pending,
        has_awaiting_node: false,
        started_at: None,
        finished_at: None,
        created_at: 30,
    };
    let repository = MockWorkflowRunRepository::default();
    repository.with_summaries(vec![summary.clone()]);
    let handler = ListWorkflowRunsByWorkflowHandler::new(Arc::new(repository));

    let response = handler
        .handle(ListWorkflowRunsByWorkflowRequest {
            workflow_id: "workflow-a".to_string(),
        })
        .unwrap();

    assert_eq!(
        response,
        ListWorkflowRunsByWorkflowResponse {
            runs: vec![map_run_summary(summary)],
        }
    );
}

/// Verifies node-run history is returned for one run.
#[test]
fn lists_node_runs() {
    let node = node_fixture("node-1", "run-1");
    let repository = MockWorkflowRunRepository::default();
    repository.with_nodes(vec![node.clone()]);
    let handler = ListWorkflowNodeRunsHandler::new(Arc::new(repository));

    let response = handler
        .handle(ListWorkflowNodeRunsRequest {
            run_id: "run-1".to_string(),
        })
        .unwrap();

    assert_eq!(
        response,
        ListWorkflowNodeRunsResponse {
            nodes: vec![map_node_run(node)],
        }
    );
}

/// Verifies run deletion succeeds on the cascade alone: physical Git cleanup
/// is registered durably by the repository, not invoked by the handler.
#[test]
fn deletes_run_without_synchronous_git_cleanup() {
    let repository = MockWorkflowRunRepository::default();
    repository.with_task_id(TaskId::new(TASK_ID));
    let handler = DeleteWorkflowRunHandler::new(Arc::new(repository), FixedClock::new(30));

    let response = handler
        .handle(DeleteWorkflowRunRequest {
            run_id: "run-1".to_string(),
        })
        .unwrap();

    assert_eq!(
        response,
        DeleteWorkflowRunResponse {
            run_id: "run-1".to_string(),
        }
    );
}

/// Verifies an active run is rejected without deleting its rows or worktree.
#[test]
fn reports_active_run_on_delete() {
    let repository = MockWorkflowRunRepository::default();
    repository.with_delete_result(DeleteWorkflowRunResult::ActiveRun);
    let handler = DeleteWorkflowRunHandler::new(Arc::new(repository), FixedClock::new(30));

    let error = handler
        .handle(DeleteWorkflowRunRequest {
            run_id: "run-1".to_string(),
        })
        .unwrap_err();

    assert_eq!(error, ApplicationError::WorkflowRunActive);
}

/// Verifies a missing run is reported as not found without any worktree cleanup.
#[test]
fn reports_not_found_on_delete() {
    let repository = MockWorkflowRunRepository::default();
    repository.with_delete_result(DeleteWorkflowRunResult::NotFound);
    let handler = DeleteWorkflowRunHandler::new(Arc::new(repository), FixedClock::new(30));

    let error = handler
        .handle(DeleteWorkflowRunRequest {
            run_id: "run-1".to_string(),
        })
        .unwrap_err();

    assert_eq!(
        error,
        ApplicationError::WorkflowRunNotFound {
            run_id: "run-1".to_string(),
        }
    );
}

/// Builds a pending run fixture for handler assertions.
fn run_fixture(id: &str) -> WorkflowRun {
    WorkflowRun::new(
        WorkflowRunId::new(id),
        WorkflowId::new("workflow-a"),
        WorkflowSnapshotId::new("snapshot-a"),
        WorkflowRunStatus::Pending,
        Some("{\"current_nodes\":[]}".to_string()),
        None,
        None,
        None,
        None,
        None,
        None,
        AuditFields::new(30, 30, /*is_deleted*/ false),
    )
}

/// Builds a succeeded node-run fixture for handler assertions.
fn node_fixture(id: &str, run_id: &str) -> WorkflowNodeRun {
    WorkflowNodeRun::new(
        WorkflowNodeRunId::new(id),
        WorkflowRunId::new(run_id),
        "start",
        "start",
        None,
        WorkflowNodeStatus::Succeeded,
        None,
        None,
        None,
        None,
        Some(30),
        Some(31),
        AuditFields::new(30, 31, /*is_deleted*/ false),
    )
}

/// Builds a workflow fixture with an optional published snapshot pointer.
fn workflow_fixture(published_snapshot_id: Option<&str>) -> Workflow {
    Workflow::new(
        WorkflowId::new("workflow-a"),
        Namespace::local(),
        "Workflow workflow-a",
        published_snapshot_id.map(WorkflowSnapshotId::new),
        AuditFields::new(10, 10, /*is_deleted*/ false),
    )
    .unwrap()
}

/// Builds a published (or draft) snapshot fixture.
fn snapshot_fixture(id: &str, version: &str) -> WorkflowSnapshot {
    WorkflowSnapshot::new(
        WorkflowSnapshotId::new(id),
        WorkflowId::new("workflow-a"),
        version,
        // A structurally valid graph (nodes + edges) so the create handler can parse it before
        // asking the worktree initializer to set up the run's initial state.
        "{\"nodes\":[],\"edges\":[]}",
        20,
        Some(20),
        /*is_deleted*/ false,
    )
}

/// Returns workflow lookups for a fixed workflow and snapshot set.
struct MockWorkflowRepository {
    workflow: Option<Workflow>,
    snapshots: Vec<WorkflowSnapshot>,
}

impl MockWorkflowRepository {
    fn with(workflow: Workflow, snapshots: Vec<WorkflowSnapshot>) -> Self {
        Self {
            workflow: Some(workflow),
            snapshots,
        }
    }
}

impl WorkflowRepository for MockWorkflowRepository {
    fn create_workflow(
        &self,
        _workflow: Workflow,
        _draft: WorkflowSnapshot,
    ) -> Result<CreatedWorkflow, RepositoryError> {
        unreachable!("create tests never create workflows")
    }

    fn find_workflow(&self, workflow_id: &WorkflowId) -> Result<Option<Workflow>, RepositoryError> {
        Ok(self
            .workflow
            .clone()
            .filter(|workflow| &workflow.id == workflow_id))
    }

    fn find_workflow_by_name(
        &self,
        _namespace: &Namespace,
        _name: &str,
    ) -> Result<Option<Workflow>, RepositoryError> {
        unreachable!("create tests never fetch workflows by name")
    }

    fn get_workflow_detail(
        &self,
        _workflow_id: &WorkflowId,
    ) -> Result<Option<WorkflowDetail>, RepositoryError> {
        unreachable!("create tests never fetch workflow detail")
    }

    fn list_workflows(&self) -> Result<Vec<WorkflowSummary>, RepositoryError> {
        unreachable!("create tests never list workflows")
    }

    fn update_workflow(
        &self,
        _workflow_id: &WorkflowId,
        _name: String,
        _updated_at: i64,
    ) -> Result<crate::workflow::UpdateWorkflowResult, RepositoryError> {
        unreachable!("create tests never update workflows")
    }

    fn soft_delete_workflow(
        &self,
        _workflow_id: &WorkflowId,
        _deleted_at: i64,
    ) -> Result<crate::DeleteWorkflowResult, RepositoryError> {
        unreachable!("create tests never delete workflows")
    }

    fn find_snapshot_by_version(
        &self,
        _workflow_id: &WorkflowId,
        _version: &str,
    ) -> Result<Option<WorkflowSnapshot>, RepositoryError> {
        unreachable!("create tests never resolve snapshots by version")
    }

    fn list_versions(
        &self,
        _workflow_id: &WorkflowId,
    ) -> Result<Vec<WorkflowVersion>, RepositoryError> {
        unreachable!("create tests never list versions")
    }

    fn update_draft(
        &self,
        _workflow_id: &WorkflowId,
        _graph: String,
        _updated_at: i64,
    ) -> Result<crate::workflow::UpdateDraftResult, RepositoryError> {
        unreachable!("create tests never update drafts")
    }

    fn publish_snapshot(
        &self,
        _workflow_id: &WorkflowId,
        _snapshot_id: WorkflowSnapshotId,
        _version: String,
        _created_at: i64,
    ) -> Result<crate::workflow::PublishSnapshotResult, RepositoryError> {
        unreachable!("create tests never publish snapshots")
    }

    fn rollback_draft(
        &self,
        _workflow_id: &WorkflowId,
        _snapshot_id: &WorkflowSnapshotId,
        _updated_at: i64,
    ) -> Result<crate::workflow::RollbackDraftResult, RepositoryError> {
        unreachable!("create tests never roll back drafts")
    }

    fn activate_version(
        &self,
        _workflow_id: &WorkflowId,
        _snapshot_id: &WorkflowSnapshotId,
        _updated_at: i64,
    ) -> Result<crate::workflow::ActivateVersionResult, RepositoryError> {
        unreachable!("create tests never activate versions")
    }

    fn soft_delete_snapshot(
        &self,
        _workflow_id: &WorkflowId,
        _snapshot_id: &WorkflowSnapshotId,
        _deleted_at: i64,
    ) -> Result<crate::workflow::DeleteSnapshotResult, RepositoryError> {
        unreachable!("create tests never delete snapshots")
    }

    fn find_snapshot_by_id(
        &self,
        workflow_id: &WorkflowId,
        snapshot_id: &WorkflowSnapshotId,
    ) -> Result<Option<WorkflowSnapshot>, RepositoryError> {
        Ok(self
            .snapshots
            .iter()
            .find(|snapshot| snapshot.id == *snapshot_id && snapshot.workflow_id == *workflow_id)
            .cloned())
    }

    fn find_snapshot_any_workflow(
        &self,
        snapshot_id: &WorkflowSnapshotId,
    ) -> Result<Option<WorkflowSnapshot>, RepositoryError> {
        Ok(self
            .snapshots
            .iter()
            .find(|snapshot| snapshot.id == *snapshot_id)
            .cloned())
    }
}

/// Records created runs and optionally fails the next create.
#[derive(Default)]
struct MockWorkflowRunRepository {
    fail_next_create: Mutex<Option<RepositoryError>>,
    created: Mutex<Vec<WorkflowRun>>,
    detail: Mutex<Option<WorkflowRunDetail>>,
    summaries: Mutex<Vec<WorkflowRunSummary>>,
    nodes: Mutex<Vec<WorkflowNodeRun>>,
    run_task_id: Mutex<Option<TaskId>>,
    delete_result: Mutex<Option<DeleteWorkflowRunResult>>,
}

impl MockWorkflowRunRepository {
    fn fail_next_create(&self, error: RepositoryError) {
        *self.fail_next_create.lock().unwrap() = Some(error);
    }

    fn created_runs(&self) -> Vec<WorkflowRun> {
        self.created.lock().unwrap().clone()
    }

    fn with_detail(&self, detail: WorkflowRunDetail) {
        *self.detail.lock().unwrap() = Some(detail);
    }

    fn with_summaries(&self, summaries: Vec<WorkflowRunSummary>) {
        *self.summaries.lock().unwrap() = summaries;
    }

    fn with_nodes(&self, nodes: Vec<WorkflowNodeRun>) {
        *self.nodes.lock().unwrap() = nodes;
    }

    fn with_task_id(&self, task_id: TaskId) {
        *self.run_task_id.lock().unwrap() = Some(task_id);
    }

    fn with_delete_result(&self, result: DeleteWorkflowRunResult) {
        *self.delete_result.lock().unwrap() = Some(result);
    }
}

impl WorkflowRunRepository for MockWorkflowRunRepository {
    fn create_run(
        &self,
        run: WorkflowRun,
        _task: Task,
        _worktree: Worktree,
        _lease_id: &WorktreeProvisioningLeaseId,
    ) -> Result<WorkflowRunCreateOutcome, RepositoryError> {
        if let Some(error) = self.fail_next_create.lock().unwrap().take() {
            return Err(error);
        }
        self.created.lock().unwrap().push(run.clone());
        Ok(WorkflowRunCreateOutcome::Created(Box::new(run)))
    }

    fn find_run(&self, _run_id: &WorkflowRunId) -> Result<Option<WorkflowRun>, RepositoryError> {
        unreachable!("create tests never read runs")
    }

    fn get_run_detail(
        &self,
        _run_id: &WorkflowRunId,
    ) -> Result<Option<WorkflowRunDetail>, RepositoryError> {
        Ok(self.detail.lock().unwrap().clone())
    }

    fn list_runs_by_project(
        &self,
        _project_id: &ProjectId,
    ) -> Result<Vec<WorkflowRunSummary>, RepositoryError> {
        Ok(self.summaries.lock().unwrap().clone())
    }

    fn list_runs_by_workflow(
        &self,
        _workflow_id: &WorkflowId,
    ) -> Result<Vec<WorkflowRunSummary>, RepositoryError> {
        Ok(self.summaries.lock().unwrap().clone())
    }

    fn list_node_runs(
        &self,
        _run_id: &WorkflowRunId,
    ) -> Result<Vec<WorkflowNodeRun>, RepositoryError> {
        Ok(self.nodes.lock().unwrap().clone())
    }

    fn find_run_task_id(&self, _run_id: &WorkflowRunId) -> Result<Option<TaskId>, RepositoryError> {
        Ok(self.run_task_id.lock().unwrap().clone())
    }

    fn soft_delete_run(
        &self,
        _run_id: &WorkflowRunId,
        _deleted_at: i64,
    ) -> Result<crate::workflow_run::DeleteWorkflowRunResult, RepositoryError> {
        Ok(self
            .delete_result
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(DeleteWorkflowRunResult::Deleted))
    }
}

/// A deploy-time worktree initializer spy: records the worktree it was asked to set up and can be
/// configured to fail so create-time compensation is testable.
#[derive(Clone, Default)]
struct MockWorktreeInitializer {
    worktrees: Arc<Mutex<Vec<PathBuf>>>,
    fail: bool,
}

impl WorkflowRunWorktreeInitializer for MockWorktreeInitializer {
    fn initialize_worktree(
        &self,
        _graph: &WorkflowGraph,
        worktree_root: &Path,
    ) -> Result<SkillMaterializationReceipt, StartPrerequisitesError> {
        self.worktrees
            .lock()
            .unwrap()
            .push(worktree_root.to_path_buf());
        if self.fail {
            return Err(StartPrerequisitesError::SkillMaterializationError {
                message: "boom".to_string(),
            });
        }
        Ok(SkillMaterializationReceipt::default())
    }
}

/// Records worktree provisioning and deletion requests for create-handler tests.
#[derive(Default)]
struct FakeTaskWorktreeProvisioner {
    created_requests: RefCell<Vec<CreateTaskWorktreeRequest>>,
    deleted_requests: RefCell<Vec<DeleteTaskWorktreeRequest>>,
    next_create_error: RefCell<Option<TaskWorktreeProvisionerError>>,
}

impl FakeTaskWorktreeProvisioner {
    fn created_requests(&self) -> Vec<CreateTaskWorktreeRequest> {
        self.created_requests.borrow().clone()
    }

    fn deleted_requests(&self) -> Vec<DeleteTaskWorktreeRequest> {
        self.deleted_requests.borrow().clone()
    }

    fn fail_next_create(&self, error: TaskWorktreeProvisionerError) {
        *self.next_create_error.borrow_mut() = Some(error);
    }
}

impl TaskWorktreeProvisioner for Arc<FakeTaskWorktreeProvisioner> {
    fn validate_repository(&self) -> Result<(), TaskWorktreeProvisionerError> {
        Ok(())
    }

    fn task_branch_exists(&self, _branch_name: &str) -> Result<bool, TaskWorktreeProvisionerError> {
        Ok(false)
    }

    fn create_task_worktree(
        &self,
        request: CreateTaskWorktreeRequest,
    ) -> Result<CreateTaskWorktreeResponse, TaskWorktreeProvisionerError> {
        if let Some(error) = self.next_create_error.borrow_mut().take() {
            return Err(error);
        }
        self.created_requests.borrow_mut().push(request);
        Ok(CreateTaskWorktreeResponse {
            base_commit_id: "base-commit".to_string(),
        })
    }

    fn delete_task_worktree(
        &self,
        request: DeleteTaskWorktreeRequest,
    ) -> Result<(), TaskWorktreeProvisionerError> {
        self.deleted_requests.borrow_mut().push(request);
        Ok(())
    }
}

/// Produces deterministic run identifiers for create-handler tests.
struct FixedRunIdGenerator {
    run_id: WorkflowRunId,
}

impl FixedRunIdGenerator {
    fn new(run_id: impl Into<String>) -> Self {
        Self {
            run_id: WorkflowRunId::new(run_id),
        }
    }
}

impl WorkflowRunIdGenerator for FixedRunIdGenerator {
    fn generate_run_id(&self) -> WorkflowRunId {
        self.run_id.clone()
    }
}

/// Produces deterministic task identifiers for create-handler tests.
struct FixedTaskIdGenerator {
    task_id: TaskId,
}

impl FixedTaskIdGenerator {
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

/// Produces deterministic worktree identifiers for create-handler tests.
struct FixedWorktreeIdGenerator {
    worktree_id: WorktreeId,
}

impl FixedWorktreeIdGenerator {
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

/// Provides a fixed wall-clock value for deterministic handler assertions.
#[derive(Clone, Copy)]
struct FixedClock(i64);

impl FixedClock {
    fn new(now: i64) -> Self {
        Self(now)
    }
}

impl Clock for FixedClock {
    fn now_timestamp_millis(&self) -> i64 {
        self.0
    }
}

/// Lease store fake recording releases so failure paths can be asserted.
#[derive(Clone, Debug, Default)]
struct FakeLeaseStore {
    created: Arc<Mutex<Vec<WorktreeProvisioningLease>>>,
    released: Arc<Mutex<Vec<WorktreeProvisioningLeaseId>>>,
}

impl FakeLeaseStore {
    /// Returns every lease created through the store.
    fn created_leases(&self) -> Vec<WorktreeProvisioningLease> {
        self.created.lock().unwrap().clone()
    }

    /// Returns every lease id released to durable cleanup.
    fn released_leases(&self) -> Vec<WorktreeProvisioningLeaseId> {
        self.released.lock().unwrap().clone()
    }
}

impl WorktreeProvisioningLeaseStore for FakeLeaseStore {
    fn create_lease(&self, lease: &WorktreeProvisioningLease) -> Result<(), RepositoryError> {
        self.created.lock().unwrap().push(lease.clone());
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
        self.released.lock().unwrap().push(lease_id.clone());
        Ok(())
    }
}
