//! Real SQLite coverage of the scheduled-artifact declaration, admission, replay and restart rule.
use super::*;
use ora_node_protocol::*;
use pretty_assertions::assert_eq;

/// A digest the wire accepts as canonical.
const DIGEST: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// The incarnation that admits and settles an upload in these tests.
fn identity() -> NodeRuntimeIdentity {
    NodeRuntimeIdentity {
        node_id: NodeId::new("node"),
        incarnation_id: NodeIncarnationId::new("first"),
    }
}

/// Supplies a valid upload of the session fixture's execution.
fn upload() -> DeliverScheduledArtifactsMessage {
    DeliverScheduledArtifactsMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: OperationId::new("upload-op"),
        execution_id: ExecutionId::new("upload-execution"),
        payload: DeliverScheduledArtifacts {
            spec: DeliverScheduledArtifactsSpec {
                session_execution_id: super::session::command().execution_id,
            },
        },
    }
}

/// A declaration of two artifacts, the same shape the collector reports.
fn manifest() -> ScheduledArtifactManifest {
    ScheduledArtifactManifest {
        items: vec![
            ScheduledArtifactItem {
                index: 0,
                display_name: "reports/review.md".to_owned(),
                kind: ScheduledArtifactKind::Markdown,
                size_bytes: 5,
                sha256: DIGEST.to_owned(),
            },
            ScheduledArtifactItem {
                index: 1,
                display_name: "data.json".to_owned(),
                kind: ScheduledArtifactKind::Json,
                size_bytes: 7,
                sha256: DIGEST.to_owned(),
            },
        ],
        rejections: Vec::new(),
        omitted_rejections: 0,
        walk_truncated: false,
    }
}

/// Ends one scheduled session normally with the given declaration.
fn end_session(
    db: &mut NodeDatabase,
    command: &StartAgentSessionMessage,
    declaration: Option<ScheduledArtifactManifest>,
) {
    db.accept_session(command).unwrap();
    db.start_session(command, &NodeIncarnationId::new("incarnation"))
        .unwrap();
    let journal = db.session_journal().unwrap();
    journal
        .end_session(
            &command.execution_id,
            AgentSessionEnded {
                node: NodeRuntimeIdentity {
                    node_id: NodeId::new("node"),
                    incarnation_id: NodeIncarnationId::new("incarnation"),
                },
                reason: AgentSessionEndReason::IdleTimeout,
                detail: None,
                plugin_id: None,
                setting_id: None,
                transport: None,
                scheduled_result: None,
                artifact_manifest: declaration,
            },
        )
        .unwrap();
}

/// A session that ended without a declaration and an unknown execution both read as unavailable.
#[test]
fn only_a_stored_declaration_names_artifacts_to_upload() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = NodeDatabase::open(
        &dir.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
    )
    .unwrap();
    let declared = super::session::command();
    end_session(&mut db, &declared, Some(manifest()));
    let mut undeclared = super::session::command();
    undeclared.operation_id = OperationId::new("undeclared-op");
    undeclared.execution_id = ExecutionId::new("undeclared-execution");
    end_session(&mut db, &undeclared, None);
    assert_eq!(
        db.scheduled_artifact_declaration(&declared.execution_id)
            .unwrap(),
        Some(ScheduledArtifactDeclaration {
            checkout_execution_id: declared.payload.spec.checkout_execution_id.clone(),
            manifest: manifest(),
        })
    );
    assert_eq!(
        db.scheduled_artifact_declaration(&undeclared.execution_id)
            .unwrap(),
        None
    );
    assert_eq!(
        db.scheduled_artifact_declaration(&ExecutionId::new("unknown"))
            .unwrap(),
        None
    );
}

/// Admission is idempotent and starts running; the terminal event replays to the owner, answers a
/// status query and is released only by its exact ACK.
#[test]
fn upload_admits_completes_replays_and_acknowledges() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut db = NodeDatabase::open(&path, NodeIdentity::Require(NodeId::new("node"))).unwrap();
    let owner = ControllerId::new("owner");
    db.bind_controller(&owner).unwrap();
    let input = upload();
    let accepted = db.accept_scheduled_artifacts(&input).unwrap();
    assert_eq!(
        accepted,
        ScheduledArtifactsDelivery {
            command: input.clone(),
            progress: ScheduledArtifactsProgress::Running,
        }
    );
    assert_eq!(db.accept_scheduled_artifacts(&input).unwrap(), accepted);
    let mut changed = input.clone();
    changed.payload.spec.session_execution_id = ExecutionId::new("other-session");
    assert!(matches!(
        db.accept_scheduled_artifacts(&changed),
        Err(Error::IdentityConflict)
    ));
    assert_eq!(
        db.execution_state(&input.operation_id, &input.execution_id)
            .unwrap(),
        ExecutionState::Running
    );
    assert_eq!(
        db.unfinished_runtime_executions().unwrap(),
        vec![input.execution_id.as_str().to_owned()]
    );
    let result = ScheduledArtifactsExecutionResult::Delivered(ScheduledArtifactsDelivered {
        artifacts: vec![StoredObject {
            key: ObjectKey::new("scheduled-runs/tenant/run/0"),
            size: 5,
            sha256: Sha256Digest::new(DIGEST),
        }],
    });
    db.complete_scheduled_artifacts(&input.execution_id, &identity(), result.clone())
        .unwrap();
    assert!(matches!(
        db.complete_scheduled_artifacts(&input.execution_id, &identity(), result.clone()),
        Err(Error::InvalidTransition)
    ));
    assert_eq!(
        db.execution_state(&input.operation_id, &input.execution_id)
            .unwrap(),
        ExecutionState::Completed(ExecutionResult::ScheduledArtifacts(Box::new(
            ScheduledArtifactsEvidence {
                node: identity(),
                result: result.clone(),
            }
        )))
    );
    let event =
        NodeToControllerMessage::ScheduledArtifactsResult(ScheduledArtifactsResultMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: input.operation_id.clone(),
            execution_id: input.execution_id.clone(),
            sequence: Sequence::new(/*value*/ 1),
            payload: result,
        });
    assert_eq!(db.controller_events(&owner).unwrap(), vec![event.clone()]);
    assert_eq!(db.pending_events().unwrap(), vec![event]);
    assert!(db.recoverable_scheduled_artifacts().unwrap().is_empty());
    assert!(db.unfinished_runtime_executions().unwrap().is_empty());
    db.acknowledge(&EventAckMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: input.operation_id.clone(),
        execution_id: input.execution_id.clone(),
        sequence: Sequence::new(/*value*/ 1),
        payload: EventAck {
            node_id: NodeId::new("node"),
        },
    })
    .unwrap();
    assert_eq!(db.controller_events(&owner).unwrap(), vec![]);
}

/// A restart fails the attempt it cannot resume as interrupted, retaining the replayable event.
#[test]
fn a_restarted_node_settles_a_running_upload_as_interrupted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut db = NodeDatabase::open(&path, NodeIdentity::Require(NodeId::new("node"))).unwrap();
    let owner = ControllerId::new("owner");
    db.bind_controller(&owner).unwrap();
    let input = upload();
    db.accept_scheduled_artifacts(&input).unwrap();
    drop(db);
    let mut reopened =
        NodeDatabase::open(&path, NodeIdentity::Require(NodeId::new("node"))).unwrap();
    assert_eq!(
        reopened.interrupt_scheduled_artifacts(&identity()).unwrap(),
        1
    );
    assert_eq!(
        reopened
            .find_scheduled_artifacts(&input.operation_id, &input.execution_id)
            .unwrap(),
        Some(ScheduledArtifactsDelivery {
            command: input.clone(),
            progress: ScheduledArtifactsProgress::Completed(Box::new(ScheduledArtifactsEvidence {
                node: identity(),
                result: ScheduledArtifactsExecutionResult::Failed(ScheduledArtifactsFailed {
                    reason: ScheduledArtifactsFailureReason::Interrupted,
                }),
            })),
        })
    );
    assert!(
        reopened
            .recoverable_scheduled_artifacts()
            .unwrap()
            .is_empty()
    );
    assert!(reopened.unfinished_runtime_executions().unwrap().is_empty());
    assert_eq!(reopened.controller_events(&owner).unwrap().len(), 1);
}

/// A controlled upload must carry a live permit bound to exactly this execution and operation.
#[test]
fn controlled_admission_checks_the_runtime_permit() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = NodeDatabase::open(
        &dir.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
    )
    .unwrap();
    let mut scope = super::repository::runtime_scope(db.node_id());
    scope.operation_id = upload().operation_id.as_str().to_owned();
    db.bind_runtime(&scope).unwrap();
    let mut binding = scope;
    binding.execution_id = upload().execution_id.as_str().to_owned();
    binding.node_operation_id = upload().operation_id.as_str().to_owned();
    let envelope = ControlledDeliverScheduledArtifacts {
        binding,
        command: upload(),
    };
    assert_eq!(
        db.accept_controlled_scheduled_artifacts(&envelope).unwrap(),
        ScheduledArtifactsDelivery {
            command: upload(),
            progress: ScheduledArtifactsProgress::Running,
        }
    );
    let mut other = envelope.clone();
    other.command.operation_id = OperationId::new("other-op");
    other.command.execution_id = ExecutionId::new("other-execution");
    other.binding.execution_id = "other-execution".to_owned();
    other.binding.node_operation_id = "other-op".to_owned();
    assert!(matches!(
        db.accept_controlled_scheduled_artifacts(&other),
        Err(Error::ResourceConflict)
    ));
    let mut stale = envelope;
    stale.binding.issued_at_ms -= 30_000;
    stale.binding.expires_at_ms -= 30_000;
    assert!(matches!(
        db.accept_controlled_scheduled_artifacts(&stale),
        Err(Error::InvalidTransition)
    ));
}

/// A Revision delivery and the upload of the same run are the one pair Cloud dispatches together
/// (`0065_scheduled_artifact_execution_kind.sql`), so neither may refuse the other. A second
/// execution of either family, and every other liability, still closes the slot.
#[test]
fn a_revision_delivery_and_the_upload_of_one_run_run_together() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = NodeDatabase::open(
        &dir.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
    )
    .unwrap();
    db.enforce_runtime_control().unwrap();
    // Both attempts belong to one Cloud run, and each has its own Node-local operation.
    let mut run = super::repository::runtime_scope(db.node_id());
    run.operation_id = "run-1".to_owned();
    db.bind_runtime(&run).unwrap();
    let permit = |command_operation: &OperationId, execution: &ExecutionId| {
        let mut binding = run.clone();
        binding.node_operation_id = command_operation.as_str().to_owned();
        binding.execution_id = execution.as_str().to_owned();
        binding
    };
    let delivery = super::revision::deliver();
    let delivery_envelope = ControlledDeliverRevision {
        binding: permit(&delivery.operation_id, &delivery.execution_id),
        command: delivery.clone(),
    };
    db.accept_controlled_delivery(&delivery_envelope).unwrap();
    // Cloud registers both deliveries when the session ends and dispatches them in either order,
    // so the upload must be admitted while the Revision delivery still runs.
    let upload_envelope = ControlledDeliverScheduledArtifacts {
        binding: permit(&upload().operation_id, &upload().execution_id),
        command: upload(),
    };
    assert_eq!(
        db.accept_controlled_scheduled_artifacts(&upload_envelope)
            .unwrap(),
        ScheduledArtifactsDelivery {
            command: upload(),
            progress: ScheduledArtifactsProgress::Running,
        }
    );
    // Only the peer family is excused: a second execution of either family still conflicts.
    assert_eq!(
        db.unfinished_conflicting_executions(DeliveryFamily::Revision)
            .unwrap(),
        vec![delivery.execution_id.as_str().to_owned()]
    );
    assert_eq!(
        db.unfinished_conflicting_executions(DeliveryFamily::ScheduledArtifacts)
            .unwrap(),
        vec![upload().execution_id.as_str().to_owned()]
    );
    // A retried attempt arrives as a new Cloud execution and a new Node-local operation.
    let second_upload = ControlledDeliverScheduledArtifacts {
        binding: permit(
            &OperationId::new("upload-op-2"),
            &ExecutionId::new("upload-execution-2"),
        ),
        command: DeliverScheduledArtifactsMessage {
            operation_id: OperationId::new("upload-op-2"),
            execution_id: ExecutionId::new("upload-execution-2"),
            ..upload()
        },
    };
    assert!(matches!(
        db.accept_controlled_scheduled_artifacts(&second_upload),
        Err(Error::ResourceConflict)
    ));
    let second_delivery = ControlledDeliverRevision {
        binding: permit(
            &OperationId::new("deliver-op-2"),
            &ExecutionId::new("deliver-execution-2"),
        ),
        command: DeliverRevisionMessage {
            operation_id: OperationId::new("deliver-op-2"),
            execution_id: ExecutionId::new("deliver-execution-2"),
            ..delivery.clone()
        },
    };
    assert!(matches!(
        db.accept_controlled_delivery(&second_delivery),
        Err(Error::ResourceConflict)
    ));
    // Both stay local liabilities for everything that is not the peer delivery.
    assert_eq!(
        db.unfinished_runtime_executions().unwrap(),
        vec![
            delivery.execution_id.as_str().to_owned(),
            upload().execution_id.as_str().to_owned(),
        ]
    );
    db.complete_delivery(
        &delivery.execution_id,
        super::revision::plan().outcome.result(),
    )
    .unwrap();
    db.complete_scheduled_artifacts(
        &upload().execution_id,
        &identity(),
        ScheduledArtifactsExecutionResult::Failed(ScheduledArtifactsFailed {
            reason: ScheduledArtifactsFailureReason::UploadFailed,
        }),
    )
    .unwrap();
    assert!(db.unfinished_runtime_executions().unwrap().is_empty());
    assert!(
        db.unfinished_conflicting_executions(DeliveryFamily::Revision)
            .unwrap()
            .is_empty()
    );
}
