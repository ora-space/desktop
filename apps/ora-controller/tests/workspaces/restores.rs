//! Sessions resuming a prior Revision through the real Cloud adapter and WebSocket transport: the
//! capability gate on registration, and the prior Revision and Workspace directory reaching the
//! Node exactly as Cloud fixed them. The Node fetches the prior Revision from the Git Server
//! itself; the Controller relays no grant for it.
use super::*;
use pretty_assertions::assert_eq;
use workspace_cloud::{PRIOR_BASE, PRIOR_DELIVERY_REF, PRIOR_FINAL, REPOSITORY, WORKSPACE};

/// The restore half of the fake sandbox Node: whether it advertises `revision_restore`.
#[derive(Clone, Default)]
pub(super) struct RestoreNode {
    pub(super) enabled: Arc<AtomicBool>,
}

/// The prior Revision as the Node must receive it.
fn prior_revision() -> PriorRevision {
    PriorRevision {
        revision_id: RevisionId::new("revision-1"),
        final_commit: CommitId::new(PRIOR_FINAL),
        git_repository_url: GitRepositoryUrl::parse(REPOSITORY).unwrap(),
        delivery_ref: DeliveryRef::parse(PRIOR_DELIVERY_REF).unwrap(),
        base_commit: CommitId::new(PRIOR_BASE),
    }
}

/// The Workspace directory as the Node must receive it: verbatim, with the empty run fields of a
/// non-run Workspace absent.
fn workspace_directory() -> Vec<WorkspaceListing> {
    vec![
        WorkspaceListing {
            workspace_id: WORKSPACE.into(),
            kind: "run".into(),
            issue_id: Some("issue-7".into()),
            issue_title: Some("Fix the parser".into()),
            issue_run_id: Some("run-3".into()),
            run_status: Some("running".into()),
        },
        WorkspaceListing {
            workspace_id: "00000000-0000-4000-8000-000000000003".into(),
            kind: "main".into(),
            issue_id: None,
            issue_title: None,
            issue_run_id: None,
            run_status: None,
        },
    ]
}

/// A session that resumes a prior Revision stays queued in Cloud while its Node cannot restore,
/// and reaches the Node with the prior Revision and the Workspace directory verbatim once a
/// restore-capable handshake happened.
#[test]
fn resumed_sessions_wait_for_a_restore_capable_handshake() {
    scenario("main", |world| async move {
        world.node.agents.enabled.store(true, Ordering::SeqCst);
        let create = world.cloud.queue(proto::OperationKind::CreateWorkspace);
        settled(&world, &create, proto::OperationState::Succeeded).await;
        world.cloud.queue_resumed_agent("run");
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            world.cloud.agent_records().is_empty(),
            "a Node that cannot restore must not receive a resumed session"
        );
        world.node.restores.enabled.store(true, Ordering::SeqCst);
        world.restart().await;
        world
            .timeline
            .until(|events| {
                events
                    .iter()
                    .any(|e| matches!(e, Event::AgentStarted { .. }))
            })
            .await;
        let records = world.cloud.agent_records();
        assert_eq!(records.len(), 1);
        let start = world
            .node
            .agents
            .started(&ExecutionId::new(records[0].execution_id.clone()));
        assert_eq!(
            (
                start.payload.spec.prior_revision,
                start.payload.spec.workspace_directory
            ),
            (Some(prior_revision()), workspace_directory())
        );
    });
}

/// Neither negotiated capability can substitute for the other when a session needs both snapshots.
#[test]
fn model_backed_resumes_require_both_capabilities_and_preserve_both_inputs() {
    for (restore_enabled, model_enabled) in [(false, true), (true, false), (false, false)] {
        scenario("main", |world| async move {
            world.node.agents.enabled.store(true, Ordering::SeqCst);
            world
                .node
                .restores
                .enabled
                .store(restore_enabled, Ordering::SeqCst);
            world
                .node
                .agents
                .model_proxy
                .store(model_enabled, Ordering::SeqCst);
            let create = world.cloud.queue(proto::OperationKind::CreateWorkspace);
            settled(&world, &create, proto::OperationState::Succeeded).await;
            world.cloud.queue_session_with_model(
                "run",
                Some(workspace_cloud::WorkspaceCloud::prior_revision()),
                "binding-1",
            );
            tokio::time::sleep(Duration::from_millis(/*millis*/ 300)).await;
            assert_eq!(world.cloud.agent_records(), Vec::new());
            world.node.restores.enabled.store(true, Ordering::SeqCst);
            world.node.agents.model_proxy.store(true, Ordering::SeqCst);
            world.restart().await;
            world
                .timeline
                .until(|events| {
                    events
                        .iter()
                        .any(|e| matches!(e, Event::AgentStarted { .. }))
                })
                .await;
            let records = world.cloud.agent_records();
            assert_eq!(records.len(), 1);
            let start = world
                .node
                .agents
                .started(&ExecutionId::new(records[0].execution_id.clone()));
            let spec = start.payload.spec;
            assert_eq!(
                spec.model_binding_id,
                Some(ModelBindingId::new("binding-1"))
            );
            assert_eq!(spec.prior_revision, Some(prior_revision()));
        });
    }
}
