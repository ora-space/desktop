use super::*;
use pretty_assertions::assert_eq;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[test]
fn target_watermarks_reject_unproven_readiness() {
    assert_eq!(
        TargetProgress::restore(
            Generation::new(3),
            Generation::new(2),
            Generation::new(1),
            Generation::new(2),
        ),
        Err(StatusTransitionError::InvalidTargetWatermarks)
    );
}

#[test]
fn operation_intent_rejects_a_mutation_label_that_disagrees_with_exact_states() {
    let native_identity = NativeResourceIdentity::parse("skill")
        .unwrap_or_else(|error| panic!("native identity: {error}"));
    let managed_identity = ManagedIdentity::new("managed-1");
    let result = EffectOperation::prepare(
        EffectOperationId::new("operation-1"),
        EffectOperationIntent {
            attempt: ReconcileAttemptId::new("attempt-1"),
            resource: EffectResourceId::new("resource-1"),
            generation: Generation::new(1),
            sequence: 0,
            mutation: EffectMutation::Delete,
            expected: ExactPreviousState::Present {
                native_identity: native_identity.clone(),
                fingerprint: Fingerprint::sha256(b"previous"),
                managed_identity: managed_identity.clone(),
            },
            planned: ExactPlannedState::Present {
                native_identity,
                fingerprint: Fingerprint::sha256(b"planned"),
                managed_identity,
            },
            payload: VersionedAdapterPlan::FilesystemDirectoryV1(FilesystemOperationPlan {
                workspace_root: PathBuf::from("/workspace"),
                resource_relative_path: ResourcePath::parse(".agents/skills")
                    .unwrap_or_else(|error| panic!("resource path: {error}")),
                resource_root: PathBuf::from("/workspace/.agents/skills"),
                source_root: None,
                staging_path: PathBuf::from("/workspace/.agents/staging"),
                backup_path: PathBuf::from("/workspace/.agents/backup"),
            }),
        },
        LocalTimestamp::from_millis(1),
    );

    assert_eq!(result, Err(OperationTransitionError::InvalidMutationStates));
}

#[test]
fn static_condition_code_rejects_an_empty_literal_in_release_builds() {
    let result = std::panic::catch_unwind(|| StableConditionCode::from_static("   "));

    assert!(result.is_err());
}

/// Builds one Prepared create operation with the exact states recovery must prove.
fn prepared_create_operation() -> EffectOperation {
    let native_identity = NativeResourceIdentity::parse("review")
        .unwrap_or_else(|error| panic!("native identity: {error}"));
    EffectOperation::prepare(
        EffectOperationId::new("operation-recovery"),
        EffectOperationIntent {
            attempt: ReconcileAttemptId::new("attempt-recovery"),
            resource: EffectResourceId::new("resource-1"),
            generation: Generation::new(1),
            sequence: 0,
            mutation: EffectMutation::Create,
            expected: ExactPreviousState::Missing,
            planned: ExactPlannedState::Present {
                native_identity,
                fingerprint: Fingerprint::sha256(b"planned"),
                managed_identity: ManagedIdentity::new("managed-1"),
            },
            payload: VersionedAdapterPlan::FilesystemDirectoryV1(FilesystemOperationPlan {
                workspace_root: PathBuf::from("/workspace"),
                resource_relative_path: ResourcePath::parse(".agents/skills")
                    .unwrap_or_else(|error| panic!("resource path: {error}")),
                resource_root: PathBuf::from("/workspace/.agents/skills"),
                source_root: None,
                staging_path: PathBuf::from("/workspace/.agents/staging"),
                backup_path: PathBuf::from("/workspace/.agents/backup"),
            }),
        },
        LocalTimestamp::from_millis(5),
    )
    .unwrap_or_else(|error| panic!("prepare operation: {error}"))
}

#[test]
fn recovery_closures_require_an_explicit_recovery_journal() {
    let mut operation = prepared_create_operation();
    assert_eq!(
        operation.finalize_recovered(LocalTimestamp::from_millis(9)),
        Err(OperationTransitionError::ExpectedRecoveryRequired)
    );
    assert_eq!(
        operation.close_without_effect(LocalTimestamp::from_millis(9)),
        Err(OperationTransitionError::ExpectedRecoveryRequired)
    );
    operation
        .require_recovery(LocalTimestamp::from_millis(7))
        .unwrap_or_else(|error| panic!("require recovery: {error}"));
    assert_eq!(
        operation.progress(),
        &OperationProgress::RecoveryRequired {
            prepared_at: LocalTimestamp::from_millis(5),
            applied_at: None,
            detected_at: LocalTimestamp::from_millis(7),
        }
    );
}

#[test]
fn a_proven_planned_recovery_finalizes_with_honest_apply_evidence() {
    let mut operation = prepared_create_operation();
    operation
        .require_recovery(LocalTimestamp::from_millis(7))
        .unwrap_or_else(|error| panic!("require recovery: {error}"));
    // A Prepared journal that proves planned lost its apply evidence to the crash, so the
    // detection time is the latest honest bound on when the apply happened.
    operation
        .finalize_recovered(LocalTimestamp::from_millis(9))
        .unwrap_or_else(|error| panic!("finalize recovered: {error}"));
    assert_eq!(
        operation.progress(),
        &OperationProgress::Finalized {
            prepared_at: LocalTimestamp::from_millis(5),
            applied_at: LocalTimestamp::from_millis(7),
            finalized_at: LocalTimestamp::from_millis(9),
        }
    );
    // A closed journal never re-enters recovery.
    assert_eq!(
        operation.require_recovery(LocalTimestamp::from_millis(11)),
        Err(OperationTransitionError::CannotRequireRecovery)
    );
}

#[test]
fn a_proven_baseline_recovery_closes_without_effect() {
    let mut operation = prepared_create_operation();
    operation
        .require_recovery(LocalTimestamp::from_millis(7))
        .unwrap_or_else(|error| panic!("require recovery: {error}"));
    operation
        .close_without_effect(LocalTimestamp::from_millis(9))
        .unwrap_or_else(|error| panic!("close without effect: {error}"));
    assert_eq!(
        operation.progress(),
        &OperationProgress::FinalizedWithoutEffect {
            prepared_at: LocalTimestamp::from_millis(5),
            applied_at: None,
            finalized_at: LocalTimestamp::from_millis(9),
        }
    );
    // A closed-without-effect journal never re-enters recovery either.
    assert_eq!(
        operation.require_recovery(LocalTimestamp::from_millis(11)),
        Err(OperationTransitionError::CannotRequireRecovery)
    );
}

#[test]
fn quarantined_attempts_close_directly_to_finalized() {
    let operation = prepared_create_operation();
    let mut attempt = ReconcileAttempt::prepare(
        ReconcileAttemptId::new("attempt-recovery"),
        ReconcileAttemptIntent {
            target: EffectTargetId::new("target-1"),
            generation: Generation::new(1),
            consumer_revision: ConsumerRevisionId::new("revision-1"),
            target_projection: ProjectionDigest::new(Digest::sha256(b"projection")),
            resource_projections: BTreeSet::from([ProjectionDigest::new(Digest::sha256(
                b"resource",
            ))]),
            coordination: CoordinationPlan::new(
                BTreeSet::from([EffectResourceId::new("resource-1")]),
                BTreeMap::from([(
                    EffectTargetId::new("target-1"),
                    CoordinationRequirement::Uninterrupted,
                )]),
            )
            .unwrap_or_else(|error| panic!("coordination plan: {error}")),
            operations: vec![operation.identity().clone()],
        },
    )
    .unwrap_or_else(|error| panic!("prepare attempt: {error}"));
    assert_eq!(
        attempt.finalize_recovered(),
        Err(OperationTransitionError::UnexpectedAttemptPhase {
            expected: ReconcileAttemptPhase::RecoveryRequired,
            actual: ReconcileAttemptPhase::Prepared,
        })
    );
    attempt
        .require_recovery()
        .unwrap_or_else(|error| panic!("require attempt recovery: {error}"));
    attempt
        .finalize_recovered()
        .unwrap_or_else(|error| panic!("finalize recovered attempt: {error}"));
    assert_eq!(attempt.phase(), ReconcileAttemptPhase::Finalized);
    assert_eq!(
        attempt.require_recovery(),
        Err(OperationTransitionError::CannotRequireAttemptRecovery)
    );
}

#[test]
fn statuses_leave_manual_recovery_only_through_the_explicit_resume_paths() {
    let mut target = TargetStatus::pending(EffectTargetId::new("target-1"), Generation::new(1));
    assert_eq!(
        target.resume_from_recovery(),
        Err(StatusTransitionError::NotInRecovery)
    );
    target
        .require_recovery(EffectOperationId::new("operation-recovery"))
        .unwrap_or_else(|error| panic!("require recovery: {error}"));
    target
        .resume_from_recovery()
        .unwrap_or_else(|error| panic!("resume target: {error}"));
    assert_eq!(target.phase(), &TargetPhase::Pending);
    target
        .require_recovery(EffectOperationId::new("operation-recovery"))
        .unwrap_or_else(|error| panic!("require recovery: {error}"));
    target
        .resume_retiring_from_recovery()
        .unwrap_or_else(|error| panic!("resume retiring target: {error}"));
    assert_eq!(target.phase(), &TargetPhase::Retiring);

    let mut resource = ResourceStatus::pending(
        EffectResourceId::new("resource-1"),
        Generation::new(1),
    );
    assert_eq!(
        resource.resume_from_recovery(),
        Err(StatusTransitionError::NotInRecovery)
    );
    resource
        .require_recovery(EffectOperationId::new("operation-recovery"))
        .unwrap_or_else(|error| panic!("require recovery: {error}"));
    resource
        .resume_from_recovery()
        .unwrap_or_else(|error| panic!("resume resource: {error}"));
    assert_eq!(resource.phase(), &ResourcePhase::Pending);
}
