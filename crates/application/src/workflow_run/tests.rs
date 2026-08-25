use super::{
    DeleteWorkflowRunResult, WorkflowRunCreateOutcome, WorkflowRunIdGenerator,
    WorkflowRunRepository, WorkflowRunWorkspaceInitializer, WorkspaceRepository,
};
use crate::workflow::WorkflowRepository;
use crate::{
    ActivateVersionResult, DeleteSnapshotResult, DeleteWorkflowResult, PublishSnapshotResult,
    RepositoryError, RollbackDraftResult, StartPrerequisitesError, UpdateDraftResult,
    UpdateWorkflowResult,
};
use ora_contracts::{CreateWorkflowRunRequest, WorkflowRunLocale};
use ora_domain::{
    AuditFields, CreatedWorkflow, Namespace, ProjectId, Workflow, WorkflowDetail, WorkflowId,
    WorkflowNodeRun, WorkflowRun, WorkflowRunDetail, WorkflowRunId, WorkflowRunSummary,
    WorkflowSnapshot, WorkflowSnapshotId, WorkflowVersion, Workspace, WorkspaceId, WorkspaceKind,
    WorkspaceLifecycle, WorkspaceLocation,
};
use pretty_assertions::assert_eq;
use std::sync::{Arc, Mutex};

const GRAPH: &str = r#"{"nodes":[],"edges":[]}"#;

/// Uses a deterministic clock so run creation assertions compare complete values.
#[derive(Clone, Copy)]
struct FixedClock;

impl crate::Clock for FixedClock {
    /// Returns the timestamp used by all values created in this test.
    fn now_timestamp_millis(&self) -> i64 {
        30
    }
}

/// Supplies one stable run identifier for the creation test.
#[derive(Clone)]
struct FixedRunIdGenerator;

impl WorkflowRunIdGenerator for FixedRunIdGenerator {
    /// Returns the deterministic identifier expected by the test.
    fn generate_run_id(&self) -> WorkflowRunId {
        WorkflowRunId::new("run-1")
    }
}

/// Returns a workspace that is already admitted for execution.
#[derive(Clone)]
struct FixedWorkspaceRepository {
    workspace: Workspace,
}

impl WorkspaceRepository for FixedWorkspaceRepository {
    /// Returns the configured workspace when its id matches the request.
    fn find_workspace(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Option<Workspace>, RepositoryError> {
        Ok((self.workspace.id == *workspace_id).then(|| self.workspace.clone()))
    }

    /// Returns the configured workspace when it is the project's main workspace.
    fn find_main_workspace(
        &self,
        project_id: &ProjectId,
    ) -> Result<Option<Workspace>, RepositoryError> {
        Ok((self.workspace.project_id == *project_id).then(|| self.workspace.clone()))
    }

    /// Returns ready for the admitted fixture so creation reaches the run repository.
    fn is_provisioning_ready(&self, workspace_id: &WorkspaceId) -> Result<bool, RepositoryError> {
        Ok(self.workspace.id == *workspace_id)
    }
}

/// Returns a workflow and its one published snapshot without persistence.
#[derive(Clone)]
struct FixedWorkflowRepository {
    workflow: Workflow,
    snapshot: WorkflowSnapshot,
}

impl WorkflowRepository for FixedWorkflowRepository {
    /// The creation test does not create workflows through this fake.
    fn create_workflow(
        &self,
        _workflow: Workflow,
        _draft: WorkflowSnapshot,
    ) -> Result<CreatedWorkflow, RepositoryError> {
        unreachable!("workflow creation is outside this test")
    }

    /// Returns the configured workflow by id.
    fn find_workflow(&self, workflow_id: &WorkflowId) -> Result<Option<Workflow>, RepositoryError> {
        Ok((self.workflow.id == *workflow_id).then(|| self.workflow.clone()))
    }

    /// Name lookup is not used by run creation.
    fn find_workflow_by_name(
        &self,
        _namespace: &Namespace,
        _name: &str,
    ) -> Result<Option<Workflow>, RepositoryError> {
        unreachable!("workflow name lookup is outside this test")
    }

    /// Detail lookup is not used by run creation.
    fn get_workflow_detail(
        &self,
        _workflow_id: &WorkflowId,
    ) -> Result<Option<WorkflowDetail>, RepositoryError> {
        unreachable!("workflow detail lookup is outside this test")
    }

    /// Listing is not used by run creation.
    fn list_workflows(&self) -> Result<Vec<ora_domain::WorkflowSummary>, RepositoryError> {
        unreachable!("workflow listing is outside this test")
    }

    /// Updating is not used by run creation.
    fn update_workflow(
        &self,
        _workflow_id: &WorkflowId,
        _name: String,
        _updated_at: i64,
    ) -> Result<UpdateWorkflowResult, RepositoryError> {
        unreachable!("workflow update is outside this test")
    }

    /// Deletion is not used by run creation.
    fn soft_delete_workflow(
        &self,
        _workflow_id: &WorkflowId,
        _deleted_at: i64,
    ) -> Result<DeleteWorkflowResult, RepositoryError> {
        unreachable!("workflow deletion is outside this test")
    }

    /// Returns the configured published snapshot by version.
    fn find_snapshot_by_version(
        &self,
        _workflow_id: &WorkflowId,
        _version: &str,
    ) -> Result<Option<WorkflowSnapshot>, RepositoryError> {
        Ok(Some(self.snapshot.clone()))
    }

    /// Returns the configured published snapshot by id.
    fn find_snapshot_by_id(
        &self,
        workflow_id: &WorkflowId,
        snapshot_id: &WorkflowSnapshotId,
    ) -> Result<Option<WorkflowSnapshot>, RepositoryError> {
        Ok(
            (self.snapshot.workflow_id == *workflow_id && self.snapshot.id == *snapshot_id)
                .then(|| self.snapshot.clone()),
        )
    }

    /// Returns the configured snapshot without workflow filtering.
    fn find_snapshot_any_workflow(
        &self,
        snapshot_id: &WorkflowSnapshotId,
    ) -> Result<Option<WorkflowSnapshot>, RepositoryError> {
        Ok((self.snapshot.id == *snapshot_id).then(|| self.snapshot.clone()))
    }

    /// Version listing is not used by run creation.
    fn list_versions(
        &self,
        _workflow_id: &WorkflowId,
    ) -> Result<Vec<WorkflowVersion>, RepositoryError> {
        unreachable!("workflow version listing is outside this test")
    }

    /// Draft updates are not used by run creation.
    fn update_draft(
        &self,
        _workflow_id: &WorkflowId,
        _graph: String,
        _updated_at: i64,
    ) -> Result<UpdateDraftResult, RepositoryError> {
        unreachable!("workflow draft update is outside this test")
    }

    /// Publishing is not used by run creation.
    fn publish_snapshot(
        &self,
        _workflow_id: &WorkflowId,
        _snapshot_id: WorkflowSnapshotId,
        _version: String,
        _created_at: i64,
    ) -> Result<PublishSnapshotResult, RepositoryError> {
        unreachable!("workflow publish is outside this test")
    }

    /// Draft rollback is not used by run creation.
    fn rollback_draft(
        &self,
        _workflow_id: &WorkflowId,
        _snapshot_id: &WorkflowSnapshotId,
        _updated_at: i64,
    ) -> Result<RollbackDraftResult, RepositoryError> {
        unreachable!("workflow rollback is outside this test")
    }

    /// Version activation is not used by run creation.
    fn activate_version(
        &self,
        _workflow_id: &WorkflowId,
        _snapshot_id: &WorkflowSnapshotId,
        _updated_at: i64,
    ) -> Result<ActivateVersionResult, RepositoryError> {
        unreachable!("workflow activation is outside this test")
    }

    /// Snapshot deletion is not used by run creation.
    fn soft_delete_snapshot(
        &self,
        _workflow_id: &WorkflowId,
        _snapshot_id: &WorkflowSnapshotId,
        _deleted_at: i64,
    ) -> Result<DeleteSnapshotResult, RepositoryError> {
        unreachable!("snapshot deletion is outside this test")
    }
}

/// Captures the run inserted by the handler.
#[derive(Default)]
struct RecordingRunRepository {
    created: Mutex<Vec<WorkflowRun>>,
}

impl WorkflowRunRepository for RecordingRunRepository {
    /// Records the direct workspace-owned run.
    fn create_run(&self, run: WorkflowRun) -> Result<WorkflowRunCreateOutcome, RepositoryError> {
        self.created.lock().unwrap().push(run.clone());
        Ok(WorkflowRunCreateOutcome::Created(Box::new(run)))
    }

    /// Single-run reads are not used by this test.
    fn find_run(&self, _run_id: &WorkflowRunId) -> Result<Option<WorkflowRun>, RepositoryError> {
        unreachable!("run lookup is outside this test")
    }

    /// Detail reads are not used by this test.
    fn get_run_detail(
        &self,
        _run_id: &WorkflowRunId,
    ) -> Result<Option<WorkflowRunDetail>, RepositoryError> {
        unreachable!("run detail lookup is outside this test")
    }

    /// Project listing is not used by this test.
    fn list_runs_by_project(
        &self,
        _project_id: &ProjectId,
    ) -> Result<Vec<WorkflowRunSummary>, RepositoryError> {
        unreachable!("run listing is outside this test")
    }

    /// Workflow listing is not used by this test.
    fn list_runs_by_workflow(
        &self,
        _workflow_id: &WorkflowId,
    ) -> Result<Vec<WorkflowRunSummary>, RepositoryError> {
        unreachable!("run listing is outside this test")
    }

    /// Run renaming is not used by this creation-focused test.
    fn rename_run(
        &self,
        _run_id: &WorkflowRunId,
        _name: String,
        _updated_at: i64,
    ) -> Result<Option<WorkflowRun>, RepositoryError> {
        unreachable!("run renaming is outside this test")
    }

    /// Node history is not used by this test.
    fn list_node_runs(
        &self,
        _run_id: &WorkflowRunId,
    ) -> Result<Vec<WorkflowNodeRun>, RepositoryError> {
        unreachable!("node listing is outside this test")
    }

    /// Deletion is not used by this test.
    fn soft_delete_run(
        &self,
        _run_id: &WorkflowRunId,
        _deleted_at: i64,
    ) -> Result<DeleteWorkflowRunResult, RepositoryError> {
        unreachable!("run deletion is outside this test")
    }
}

/// Supplies no filesystem materialization while still exercising the workspace seam.
#[derive(Clone, Copy)]
struct NoopWorkspaceInitializer;

impl WorkflowRunWorkspaceInitializer for NoopWorkspaceInitializer {
    /// Returns an empty frozen skill receipt for a graph with no skills.
    fn initialize_workspace(
        &self,
        _graph: &crate::WorkflowGraph,
        _workspace_root: &std::path::Path,
    ) -> Result<crate::SkillMaterializationReceipt, StartPrerequisitesError> {
        Ok(crate::SkillMaterializationReceipt::default())
    }
}

/// Verifies creation stores workspace identity and run name without creating a Task or Worktree.
#[test]
fn creates_run_directly_in_workspace() {
    let workflow_id = WorkflowId::new("workflow-1");
    let snapshot_id = WorkflowSnapshotId::new("snapshot-1");
    let workflow = Workflow::new(
        workflow_id.clone(),
        Namespace::local(),
        "Review",
        Some(snapshot_id.clone()),
        AuditFields::new(1, 1, false),
    )
    .unwrap();
    let snapshot = WorkflowSnapshot::new(snapshot_id, workflow_id, "v1", GRAPH, 1, Some(1), false);
    let workspace = Workspace::new(
        WorkspaceId::new("workspace-1"),
        ProjectId::new("project-1"),
        WorkspaceKind::Main,
        WorkspaceLocation::local_filesystem("/tmp/project"),
        WorkspaceLifecycle::Active,
        AuditFields::new(1, 1, false),
    );
    let repository = Arc::new(RecordingRunRepository::default());
    let handler = super::CreateWorkflowRunHandler::new(
        Arc::new(FixedWorkflowRepository { workflow, snapshot }),
        Arc::new(FixedWorkspaceRepository { workspace }),
        repository.clone(),
        FixedRunIdGenerator,
        NoopWorkspaceInitializer,
        FixedClock,
    );

    let response = handler
        .handle(CreateWorkflowRunRequest {
            workspace_id: "workspace-1".to_string(),
            workflow_id: "workflow-1".to_string(),
            locale: WorkflowRunLocale::EnUs,
            snapshot_id: None,
            kickoff_input: Some("Inspect".to_string()),
            name: Some("Manual review".to_string()),
        })
        .unwrap();
    let stored = repository.created.lock().unwrap().clone();
    assert_eq!(stored.len(), 1);
    assert_eq!(response.run.workspace_id, "workspace-1");
    assert_eq!(response.run.name, "Manual review");
    assert_eq!(stored[0].workspace_id, WorkspaceId::new("workspace-1"));
}
