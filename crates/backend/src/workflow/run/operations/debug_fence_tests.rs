//! Run-local debug refusals leave the backend as their own public errors.
//!
//! The repository refuses new work for a run whose confirmation is still stopping, and refuses
//! targets it cannot prove. These tests pin that the backend keeps both meanings for the page
//! instead of reporting an internal failure.

use super::WorkflowRuns;
use crate::Backend;
use crate::error::ErrorClassification;
use crate::test_backend::backend_paths;
use crate::workflow::run::test_fixture::{SeededWorkflow, seeded_workflow};
use ora_contracts::{
    CancelWorkflowRunRequest, CreateWorkflowRunRequest, EmptyErrorParams,
    GetWorkflowDebugAttemptRequest, GetWorkflowRunRequest, PublicError, RestartWorkflowRunRequest,
    StartWorkflowRunRequest, WorkflowDebugTargetRejectedParams, WorkflowRunLocale,
    WorkflowRunStatus,
};
use pretty_assertions::assert_eq;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

/// One agent that needs no plugin, so starting the run passes the dependency precheck.
const AGENT_GRAPH: &str = r#"{"nodes":[
    {"id":"start","data":{"kind":"start"}},
    {"id":"agent","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"official/ora-space.opencode","modelId":"m"},"prompt":"do"}}}
],"edges":[{"source":"start","target":"agent"}]}"#;

/// One backend with a published single-agent workflow.
struct Fixture {
    /// Owns the data directory for as long as the backend is open.
    _temporary: TempDir,
    runs: Arc<WorkflowRuns>,
    seeded: SeededWorkflow,
    /// The database behind `runs`; opened directly only to arrange a precondition.
    database: PathBuf,
}

impl Fixture {
    /// Opens the backend through its real composition and publishes the fixture graph.
    fn open() -> Self {
        let temporary = TempDir::new().expect("debug fence fixture");
        let backend = Backend::open(backend_paths(temporary.path(), temporary.path()))
            .expect("backend composition");
        let runs = backend.workflow_runs();
        let seeded = seeded_workflow(&temporary, &runs.pool, AGENT_GRAPH);
        let database = temporary.path().join("ora.sqlite3");
        Self {
            _temporary: temporary,
            runs,
            seeded,
            database,
        }
    }

    /// Creates and starts a run the way the run screen does, then cancels it so a restart is
    /// allowed again.
    async fn cancelled_run(&self) -> String {
        let run_id = self
            .runs
            .create(CreateWorkflowRunRequest {
                workspace_id: self.seeded.workspace_id.clone(),
                workflow_id: self.seeded.workflow_id.clone(),
                locale: WorkflowRunLocale::EnUs,
                snapshot_id: None,
                kickoff_input: None,
                name: None,
                inject_last_failure: None,
            })
            .expect("create run from the published snapshot")
            .run
            .id
            .clone();
        self.runs
            .start(StartWorkflowRunRequest {
                run_id: run_id.clone(),
            })
            .expect("start the run");
        let cancelled = self
            .runs
            .cancel(CancelWorkflowRunRequest {
                run_id: run_id.clone(),
            })
            .await
            .expect("cancel the started run");
        assert_eq!(cancelled.run.status, WorkflowRunStatus::Cancelled);
        run_id
    }

    /// Records a confirmation that is still stopping, as the coordinator leaves it while it
    /// waits for the affected attempts to settle.
    fn stopping_confirmation(&self, run_id: &str) {
        let connection = rusqlite::Connection::open(&self.database).expect("open the run database");
        connection
            .execute(
                "INSERT INTO workflow_debug_previews(id,run_id,snapshot_id,root_scope,node_type,
                    read_nodes,read_facts,base_revision,patch,plan)
                 SELECT 'preview',r.id,r.snapshot_id,root.scope_id,'agent','[]','[]',0,'[]','{}'
                 FROM workflow_runs r JOIN workflow_run_root_scopes root ON root.run_id=r.id
                 WHERE r.id=?1",
                [run_id],
            )
            .expect("record the preview the confirmation names");
        connection
            .execute(
                "INSERT INTO workflow_debug_operations(run_id,request_id,preview_id,state,
                    prior_status)
                 VALUES (?1,'request','preview','stopping',1)",
                [run_id],
            )
            .expect("record the stopping confirmation");
    }

    /// Reads the run status the run screen would show.
    fn status(&self, run_id: &str) -> WorkflowRunStatus {
        self.runs
            .get(GetWorkflowRunRequest {
                run_id: run_id.to_owned(),
            })
            .expect("read the run")
            .run
            .status
    }
}

/// A restart meets the fence and reports it, leaving the run exactly as it was.
#[tokio::test]
async fn restart_reports_an_active_debug_operation_and_changes_nothing() {
    let fixture = Fixture::open();
    let run_id = fixture.cancelled_run().await;
    fixture.stopping_confirmation(&run_id);

    let error = fixture
        .runs
        .restart(RestartWorkflowRunRequest {
            run_id: run_id.clone(),
        })
        .expect_err("a restart must not overtake a stopping confirmation");
    assert_eq!(
        (
            error.classification(),
            error.public_error().clone(),
            fixture.status(&run_id),
        ),
        (
            ErrorClassification::Conflict,
            PublicError::WorkflowDebugOperationActive(EmptyErrorParams {}),
            WorkflowRunStatus::Cancelled,
        )
    );
}

/// A target the store cannot prove is refused with its reason, not as an internal failure.
#[tokio::test]
async fn an_unknown_debug_attempt_is_rejected_with_its_reason() {
    let fixture = Fixture::open();
    let run_id = fixture.cancelled_run().await;

    let error = fixture
        .runs
        .debug_attempt(GetWorkflowDebugAttemptRequest {
            run_id,
            node_run_id: "missing".to_string(),
        })
        .expect_err("an unknown attempt cannot be read");
    assert_eq!(
        (error.classification(), error.public_error().clone(),),
        (
            ErrorClassification::InvalidRequest,
            PublicError::WorkflowDebugTargetRejected(WorkflowDebugTargetRejectedParams {
                reason: "the target is not a selectable attempt or position of this run"
                    .to_string(),
            }),
        )
    );
}
