use super::{ReadyConsumer, declaration, fixture, package_fingerprint};
use crate::{PluginSkillProjection, SqliteEffectRepository, SqliteSkillRepository};
use ora_domain::PluginId;
use ora_effect::*;
use ora_effect::{EffectRecoverer, RecoveryOutcome};
use ora_effect_skill::{SkillDirectoryResourceAdapter, SkillPlanner};
use pretty_assertions::assert_eq;
use rusqlite::params;

/// Writes one Skill package directory and returns its validated projection.
fn skill_package(
    directory: &std::path::Path,
    package_name: &str,
    manifest: &[u8],
) -> PluginSkillProjection {
    let package_root = directory.join(package_name);
    std::fs::create_dir_all(&package_root)
        .unwrap_or_else(|error| panic!("create package: {error}"));
    std::fs::write(package_root.join("SKILL.md"), manifest)
        .unwrap_or_else(|error| panic!("write manifest: {error}"));
    PluginSkillProjection {
        name: "review".to_string(),
        description: "Reviews changes".to_string(),
        package_fingerprint: package_fingerprint(&package_root),
        package_root,
        skill_md_digest: Digest::sha256(manifest),
    }
}

/// Reconciles every currently due Target once with the production adapters.
fn reconcile_due(
    repository: &SqliteEffectRepository<crate::test_clock::TestClock>,
    clock: &crate::test_clock::TestClock,
    worker: &WorkerIdentity,
    now: i64,
) -> Vec<Result<(EffectTargetId, ReconcileOutcome), (EffectTargetId, ReconcileError)>> {
    let claimed = repository
        .claim_due_targets(
            worker,
            LocalTimestamp::from_millis(now),
            LocalTimestamp::from_millis(now + 10_000),
            8,
        )
        .unwrap_or_else(|error| panic!("claim due Targets: {error}"));
    let mut outcomes = Vec::new();
    for (target, claim) in claimed {
        let outcome = EffectReconciler::new(
            repository,
            &SkillPlanner,
            &ReadyConsumer,
            &SkillDirectoryResourceAdapter,
            clock,
        )
        .reconcile(&target, &claim, LocalTimestamp::from_millis(now + 10_000));
        match outcome {
            Ok(outcome) => outcomes.push(Ok((target, outcome))),
            Err(error) => outcomes.push(Err((target, error))),
        }
    }
    outcomes
}

/// Runs one explicit recovery pass with the production filesystem adapter.
fn recover_target(
    repository: &SqliteEffectRepository<crate::test_clock::TestClock>,
    clock: &crate::test_clock::TestClock,
    target: &EffectTargetId,
) -> Result<RecoveryOutcome, RecoveryError> {
    EffectRecoverer::new(repository, &SkillDirectoryResourceAdapter, clock).recover(target)
}

/// Publishes one Skill, declares the Consumer, and converges once.
fn converged_target(
    directory: &std::path::Path,
    pool: &crate::RepositoryPool,
    workspace: &ora_domain::Workspace,
    manifest: &[u8],
) -> (
    SqliteEffectRepository<crate::test_clock::TestClock>,
    crate::test_clock::TestClock,
    EffectTargetId,
    WorkerIdentity,
) {
    let clock = crate::test_clock::TestClock::new(100);
    let skills = SqliteSkillRepository::with_clock(pool.clone(), clock.clone());
    skills
        .replace_plugin_skills(
            &PluginId::new("official", "skill-pack").unwrap_or_else(|e| panic!("plugin: {e}")),
            "1.0.0",
            &[skill_package(directory, "skill-review", manifest)],
            110,
        )
        .unwrap_or_else(|e| panic!("publish Skill plugin: {e}"));
    let repository = SqliteEffectRepository::with_clock(pool.clone(), clock.clone());
    repository
        .declare_consumer(
            &declaration("local/ora-space.codeagent"),
            std::slice::from_ref(workspace),
        )
        .unwrap_or_else(|e| panic!("declare Consumer: {e}"));
    let worker = WorkerIdentity::parse("worker-1").unwrap_or_else(|e| panic!("worker: {e}"));
    let outcomes = reconcile_due(&repository, &clock, &worker, 120);
    assert_eq!(outcomes.len(), 1);
    let (target, outcome) = match outcomes.into_iter().next() {
        Some(Ok((target, outcome))) => (target, outcome),
        Some(Err((target, error))) => panic!("first reconcile {target}: {error:?}"),
        None => panic!("the first Target was not claimed"),
    };
    assert!(matches!(outcome, ReconcileOutcome::Mutated { .. }));
    (repository, clock, target, worker)
}

/// Rewinds one converged attempt to the durable leftover state a pre-0.4.0 finalize
/// failure produced: an Applied operation log, an unfinished attempt, and the still-owed
/// request the failed reconcile never completed.
fn rewind_to_applied_leftover(pool: &crate::RepositoryPool, target: &EffectTargetId) {
    pool.with_connection(|connection| {
        let attempt_id: String = connection.query_row(
            "SELECT attempt_id FROM effect_operations op
             JOIN effect_reconcile_attempts attempt ON attempt.id = op.attempt_id
             WHERE attempt.target_id = ?1",
            params![target.as_str()],
            |row| row.get(0),
        )?;
        connection.execute(
            "UPDATE effect_operations SET phase = 'applied', finalized_at = NULL WHERE attempt_id = ?1",
            params![&attempt_id],
        )?;
        connection.execute(
            "UPDATE effect_reconcile_attempts SET phase = 'activated' WHERE id = ?1",
            params![&attempt_id],
        )?;
        connection.execute(
            "INSERT INTO effect_reconcile_requests (
                 target_id, requested_generation, state, wake_reasons_json, retry_count,
                 claim_token, claim_worker, lease_until, requested_at, updated_at
             ) VALUES (?1, 1, 'claimed', '[]', 3, 1, 'worker-1', 130, 121, 121)
             ON CONFLICT(target_id) DO UPDATE SET
                 state = 'claimed', claim_token = 1, claim_worker = 'worker-1',
                 lease_until = 130, updated_at = MAX(effect_reconcile_requests.updated_at, excluded.updated_at)",
            params![target.as_str()],
        )?;
        Ok(())
    })
    .unwrap_or_else(|error| panic!("rewind to applied leftover: {error}"));
}

/// Loads the durable journal phases, request state, Conditions, and ledger of one Target.
fn quarantine_state(
    pool: &crate::RepositoryPool,
    target: &EffectTargetId,
) -> (String, String, i64, i64) {
    pool.with_connection(|connection| {
        connection.query_row(
            "SELECT
                 (SELECT group_concat(DISTINCT op.phase) FROM effect_operations op
                  JOIN effect_reconcile_attempts attempt ON attempt.id = op.attempt_id
                  WHERE attempt.target_id = ?1),
                 (SELECT state FROM effect_reconcile_requests WHERE target_id = ?1),
                 (SELECT COUNT(*) FROM effect_conditions WHERE owner_kind = 'target' AND owner_id = ?1),
                 (SELECT COUNT(*) FROM effect_managed_items)",
            params![target.as_str()],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .map_err(Into::into)
    })
    .unwrap_or_else(|error| panic!("load quarantine state: {error}"))
}

/// Reproduces issue #177: a legacy Applied leftover is quarantined at restart, and the
/// explicit recovery exit finalizes the proven effect and restores scheduling.
#[test]
fn recovery_replays_a_proven_applied_leftover_and_resumes_scheduling() {
    let (directory, pool, workspace) = fixture();
    let manifest = b"---\nname: review\ndescription: Reviews changes\n---\ncontent\n";
    let (repository, clock, target, worker) =
        converged_target(directory.path(), &pool, &workspace, manifest);
    rewind_to_applied_leftover(&pool, &target);

    // A restart quarantines the leftover exactly like the reporting user's upgrade.
    let quarantined = repository
        .quarantine_unfinished_operations(LocalTimestamp::from_millis(200))
        .unwrap_or_else(|e| panic!("quarantine leftover: {e}"));
    assert_eq!(quarantined, 1);
    assert_eq!(
        quarantine_state(&pool, &target),
        ("recovery_required".to_string(), "blocked".to_string(), 1, 1)
    );

    // The materialized package proves the exact planned state, so the journal finalizes.
    let outcome = recover_target(&repository, &clock, &target)
        .unwrap_or_else(|e| panic!("recover Target: {e:?}"));
    assert_eq!(
        outcome,
        RecoveryOutcome::Recovered {
            target: target.clone(),
            finalized: 1,
            closed_without_effect: 0,
        }
    );
    let state = quarantine_state(&pool, &target);
    assert_eq!(state.0, "finalized", "the journal closed as finalized");
    assert_eq!(state.1, "pending", "the request returned to scheduling");
    assert_eq!(state.2, 0, "the manual-recovery Condition cleared");
    assert_eq!(state.3, 1, "the proven effect keeps its ledger row");
    let status = repository
        .load_target_status(&target)
        .unwrap_or_else(|e| panic!("load status: {e}"))
        .expect("status");
    assert_eq!(status.status.phase(), &TargetPhase::Pending);

    // Ordinary reconciliation converges again without another mutation.
    let outcomes = reconcile_due(&repository, &clock, &worker, 210);
    assert_eq!(outcomes.len(), 1);
    let (recovered, outcome) = match outcomes.into_iter().next() {
        Some(Ok((target, outcome))) => (target, outcome),
        Some(Err((target, error))) => panic!("post-recovery reconcile {target}: {error:?}"),
        None => panic!("the recovered Target was not claimed"),
    };
    assert_eq!(recovered, target);
    assert_eq!(
        outcome,
        ReconcileOutcome::Current {
            target: target.clone(),
            generation: Generation::new(1),
        }
    );
    assert_eq!(
        std::fs::read(
            directory
                .path()
                .join("workspace")
                .join(".agents")
                .join("skills")
                .join("review")
                .join("SKILL.md")
        )
        .unwrap_or_else(|e| panic!("read materialized Skill: {e}")),
        manifest
    );
}

/// A Prepared journal whose mutation provably never ran closes without granting ownership;
/// the next ordinary reconcile re-plans from the baseline and converges.
#[test]
fn recovery_closes_a_prepared_journal_without_effect_and_reconverges() {
    struct FailingApply;

    impl ResourceAdapter for FailingApply {
        fn prepare_operation(
            &self,
            resource: &EffectResource,
            attempt: ReconcileAttemptId,
            generation: Generation,
            sequence: u32,
            mutation: PlannedMutation,
            prepared_at: LocalTimestamp,
        ) -> Result<PreparedOperation, ResourceAdapterError> {
            ResourceAdapter::prepare_operation(
                &ADAPTER,
                resource,
                attempt,
                generation,
                sequence,
                mutation,
                prepared_at,
            )
        }

        fn observe(
            &self,
            resource: &EffectResource,
        ) -> Result<ResourceObservation, ResourceAdapterError> {
            SkillDirectoryResourceAdapter.observe(resource)
        }

        fn apply(
            &self,
            _operation: &EffectOperation,
        ) -> Result<ApplyReceipt, ResourceAdapterError> {
            Err(ResourceAdapterError::new(std::io::Error::other(
                "simulated crash before apply",
            )))
        }

        fn verify(
            &self,
            operation: &EffectOperation,
        ) -> Result<VerificationReceipt, ResourceAdapterError> {
            SkillDirectoryResourceAdapter.verify(operation)
        }

        fn verify_expected(
            &self,
            operation: &EffectOperation,
        ) -> Result<VerificationReceipt, ResourceAdapterError> {
            SkillDirectoryResourceAdapter.verify_expected(operation)
        }

        fn cleanup(
            &self,
            artifact: &OperationArtifact,
        ) -> Result<CleanupReceipt, ResourceAdapterError> {
            SkillDirectoryResourceAdapter.cleanup(artifact)
        }
    }

    let (directory, pool, workspace) = fixture();
    let manifest = b"---\nname: review\ndescription: Reviews changes\n---\ncontent\n";
    let (repository, clock, target, worker) =
        converged_target(directory.path(), &pool, &workspace, manifest);
    const ADAPTER: SkillDirectoryResourceAdapter = SkillDirectoryResourceAdapter;
    // A newer publication raises Desired; the worker crashes after journal preparation
    // but before any external mutation.
    let updated = b"---\nname: review\ndescription: Reviews changes\n---\nupdated\n";
    SqliteSkillRepository::with_clock(pool.clone(), clock.clone())
        .replace_plugin_skills(
            &PluginId::new("official", "skill-pack").unwrap_or_else(|e| panic!("plugin: {e}")),
            "1.1.0",
            &[skill_package(directory.path(), "skill-review", updated)],
            150,
        )
        .unwrap_or_else(|e| panic!("publish newer Skill: {e}"));
    let (_, claim) = repository
        .claim_due_targets(
            &worker,
            LocalTimestamp::from_millis(160),
            LocalTimestamp::from_millis(170),
            1,
        )
        .unwrap_or_else(|e| panic!("claim Target: {e}"))
        .remove(0);
    let failed = EffectReconciler::new(
        &repository,
        &SkillPlanner,
        &ReadyConsumer,
        &FailingApply,
        &clock,
    )
    .reconcile(&target, &claim, LocalTimestamp::from_millis(170));
    assert!(
        failed.is_err(),
        "the simulated crash must fail the reconcile"
    );

    // The worker lease expires before explicit recovery runs, exactly like a restart.
    clock.set(200);

    let outcome = recover_target(&repository, &clock, &target)
        .unwrap_or_else(|e| panic!("recover Target: {e:?}"));
    assert_eq!(
        outcome,
        RecoveryOutcome::Recovered {
            target: target.clone(),
            finalized: 0,
            closed_without_effect: 1,
        }
    );
    let state = quarantine_state(&pool, &target);
    assert_eq!(
        state.0, "finalized,finalized_without_effect",
        "the prepared journal closed without effect"
    );
    assert_eq!(state.1, "pending");
    assert_eq!(state.2, 0);
    assert_eq!(
        state.3, 1,
        "the closed journal grants no additional ledger row"
    );

    // The next ordinary reconcile re-plans from the unchanged baseline and converges.
    let outcomes = reconcile_due(&repository, &clock, &worker, 180);
    assert_eq!(outcomes.len(), 1);
    let (_, outcome) = match outcomes.into_iter().next() {
        Some(Ok((target, outcome))) => (target, outcome),
        Some(Err((target, error))) => panic!("post-recovery reconcile {target}: {error:?}"),
        None => panic!("the recovered Target was not claimed"),
    };
    assert!(matches!(outcome, ReconcileOutcome::Mutated { .. }));
    assert_eq!(
        std::fs::read(
            directory
                .path()
                .join("workspace")
                .join(".agents")
                .join("skills")
                .join("review")
                .join("SKILL.md")
        )
        .unwrap_or_else(|e| panic!("read materialized Skill: {e}")),
        updated
    );
}

/// External drift proves neither exact state, so recovery refuses and keeps the quarantine.
#[test]
fn recovery_refuses_drifted_state_and_keeps_the_quarantine() {
    let (directory, pool, workspace) = fixture();
    let manifest = b"---\nname: review\ndescription: Reviews changes\n---\ncontent\n";
    let (repository, clock, target, _worker) =
        converged_target(directory.path(), &pool, &workspace, manifest);
    rewind_to_applied_leftover(&pool, &target);
    repository
        .quarantine_unfinished_operations(LocalTimestamp::from_millis(200))
        .unwrap_or_else(|e| panic!("quarantine leftover: {e}"));

    // The user edits the materialized package while the journal is quarantined.
    std::fs::write(
        directory
            .path()
            .join("workspace")
            .join(".agents")
            .join("skills")
            .join("review")
            .join("SKILL.md"),
        b"---\nname: review\ndescription: Reviews changes\n---\ndrifted\n",
    )
    .unwrap_or_else(|error| panic!("drift the materialized Skill: {error}"));

    let error = recover_target(&repository, &clock, &target)
        .expect_err("drifted state must not be recoverable");
    assert!(matches!(error, RecoveryError::NotRecoverable { .. }));
    assert_eq!(
        quarantine_state(&pool, &target),
        ("recovery_required".to_string(), "blocked".to_string(), 1, 1),
        "the quarantine is preserved without guessing"
    );
    let status = repository
        .load_target_status(&target)
        .unwrap_or_else(|e| panic!("load status: {e}"))
        .expect("status");
    assert!(matches!(
        status.status.phase(),
        TargetPhase::RecoveryRequired(_)
    ));
}

/// Retiring a quarantined Target keeps its recovery phase (the status CHECK stays
/// satisfiable), and recovery then advances the Scope epoch so the retiring Target's
/// empty projection converges on a fresh identity instead of colliding (issue #6).
#[test]
fn recovery_of_a_retiring_target_advances_the_epoch_and_finishes_retirement() {
    let (directory, pool, workspace) = fixture();
    let manifest = b"---\nname: review\ndescription: Reviews changes\n---\ncontent\n";
    let (repository, clock, target, worker) =
        converged_target(directory.path(), &pool, &workspace, manifest);
    rewind_to_applied_leftover(&pool, &target);
    repository
        .quarantine_unfinished_operations(LocalTimestamp::from_millis(200))
        .unwrap_or_else(|e| panic!("quarantine leftover: {e}"));

    // Uninstalling the plugin while the Target is quarantined must not violate the
    // status CHECK and must keep the explicit recovery requirement.
    repository
        .retire_consumer(
            &ConsumerIdentity::new(ConsumerKind::agent_plugin(), "local/ora-space.codeagent")
                .unwrap_or_else(|e| panic!("consumer identity: {e}")),
        )
        .unwrap_or_else(|e| panic!("retire quarantined Consumer: {e}"));
    let retired = pool
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT target.lifecycle,
                            (SELECT phase FROM effect_target_status WHERE target_id = ?1),
                            (SELECT generation FROM effect_scopes scope
                             JOIN effect_targets target ON target.scope_id = scope.id
                             WHERE target.id = ?1)
                     FROM effect_targets target WHERE target.id = ?1",
                    params![target.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    },
                )
                .map_err(Into::into)
        })
        .unwrap_or_else(|e| panic!("load retired Target: {e}"));
    assert_eq!(
        retired,
        ("retiring".to_string(), "recovery_required".to_string(), 2),
        "retirement marks the Target and advances the epoch without clearing recovery"
    );

    let outcome = recover_target(&repository, &clock, &target)
        .unwrap_or_else(|e| panic!("recover retiring Target: {e:?}"));
    assert_eq!(
        outcome,
        RecoveryOutcome::Recovered {
            target: target.clone(),
            finalized: 1,
            closed_without_effect: 0,
        }
    );

    // The retiring Target converges its empty contribution and finishes retirement.
    let outcomes = reconcile_due(&repository, &clock, &worker, 210);
    assert_eq!(
        outcomes.len(),
        1,
        "the retiring Target is claimed: {outcomes:?}"
    );
    let (_, outcome) = match outcomes.into_iter().next() {
        Some(Ok((target, outcome))) => (target, outcome),
        Some(Err((target, error))) => panic!("retiring reconcile {target}: {error:?}"),
        None => panic!("the retiring Target was not claimed"),
    };
    assert!(matches!(outcome, ReconcileOutcome::Mutated { .. }));
    let outcomes = reconcile_due(&repository, &clock, &worker, 220);
    assert!(
        outcomes.is_empty(),
        "retirement finished; no further work remains: {outcomes:?}"
    );
    let remaining = pool
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT
                         (SELECT COUNT(*) FROM effect_targets),
                         (SELECT COUNT(*) FROM effect_reconcile_requests),
                         (SELECT COUNT(*) FROM effect_managed_items),
                         (SELECT EXISTS (SELECT 1 FROM effect_target_status WHERE phase = 'recovery_required'))
                     FROM (SELECT 1)",
                    [],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, i64>(3)?,
                        ))
                    },
                )
                .map_err(Into::into)
        })
        .unwrap_or_else(|e| panic!("load remaining rows: {e}"));
    assert_eq!(
        remaining,
        (0, 0, 0, 0),
        "retirement removed every owned row"
    );
}

/// Recovering a healthy Target changes nothing and reports that no journal needed closing.
#[test]
fn recovery_of_a_healthy_target_is_an_idempotent_no_op() {
    let (directory, pool, workspace) = fixture();
    let manifest = b"---\nname: review\ndescription: Reviews changes\n---\ncontent\n";
    let (repository, clock, target, _worker) =
        converged_target(directory.path(), &pool, &workspace, manifest);

    let before = pool
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT
                         (SELECT COUNT(*) FROM effect_operations),
                         (SELECT status_version FROM effect_target_status WHERE target_id = ?1),
                         (SELECT COUNT(*) FROM effect_conditions)
                     FROM (SELECT 1)",
                    params![target.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    },
                )
                .map_err(Into::into)
        })
        .unwrap_or_else(|e| panic!("load healthy state: {e}"));

    let outcome = recover_target(&repository, &clock, &target)
        .unwrap_or_else(|e| panic!("recover healthy Target: {e:?}"));
    assert_eq!(
        outcome,
        RecoveryOutcome::AlreadyHealthy {
            target: target.clone(),
        }
    );

    let after = pool
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT
                         (SELECT COUNT(*) FROM effect_operations),
                         (SELECT status_version FROM effect_target_status WHERE target_id = ?1),
                         (SELECT COUNT(*) FROM effect_conditions)
                     FROM (SELECT 1)",
                    params![target.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    },
                )
                .map_err(Into::into)
        })
        .unwrap_or_else(|e| panic!("reload healthy state: {e}"));
    assert_eq!(after, before, "no durable state changed");
}
