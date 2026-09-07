use super::WorkflowApi;
use crate::clock::SystemClock;
use ora_contracts::*;
use ora_db::{DatabaseBootstrapper, DatabaseLocation, default_migration_catalog};
use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;

/// Definition and draft persistence can be tested without constructing a workflow execution engine.
#[test]
fn definitions_reopen_with_their_drafts_without_application_runtime() {
    with_trace_logging(|| {
        let temporary = tempfile::tempdir().expect("workflow fixture");
        let path = temporary.path().join("ora.sqlite3");
        let catalog = default_migration_catalog().expect("catalog");
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(&DatabaseLocation::path(&path), &catalog)
            .expect("database");
        let workflows = WorkflowApi::new(pool, SystemClock);
        let created = workflows
            .create(CreateWorkflowRequest {
                name: "Review".to_string(),
                graph: None,
            })
            .expect("create workflow");
        let workflow_id = created.workflow.id.clone();
        let expected = GetWorkflowResponse {
            workflow: created.workflow,
            draft: created.draft,
            published: None,
        };
        assert_eq!(
            workflows
                .get(GetWorkflowRequest {
                    workflow_id: workflow_id.clone()
                })
                .expect("get workflow"),
            expected
        );
        assert_eq!(
            workflows
                .get_draft(GetDraftRequest {
                    workflow_id: workflow_id.clone()
                })
                .expect("get draft"),
            GetDraftResponse {
                snapshot: expected.draft.clone()
            }
        );
        drop(workflows);
        let reopened = WorkflowApi::new(
            DatabaseBootstrapper::system()
                .bootstrap_repository_pool(&DatabaseLocation::path(&path), &catalog)
                .expect("reopen database"),
            SystemClock,
        );
        assert_eq!(
            reopened
                .get(GetWorkflowRequest {
                    workflow_id: workflow_id.clone()
                })
                .expect("reopened workflow"),
            expected
        );
        assert_eq!(
            reopened
                .delete(DeleteWorkflowRequest {
                    workflow_id: workflow_id.clone()
                })
                .expect("delete workflow"),
            DeleteWorkflowResponse {
                workflow_id: workflow_id.clone()
            }
        );
        assert_eq!(
            reopened
                .get(GetWorkflowRequest { workflow_id })
                .expect_err("deleted workflow is unavailable")
                .public_error(),
            &PublicError::WorkflowNotFound(EmptyErrorParams {})
        );
    });
}
