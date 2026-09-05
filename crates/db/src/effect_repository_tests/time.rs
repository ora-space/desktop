use super::{ReadyConsumer, declaration, fixture, package_fingerprint};
use crate::{
    PluginSkillProjection, SqliteEffectRepository, SqliteSkillRepository, test_clock::TestClock,
};
use ora_application::{LocalSkillSourceRevision, SkillRepository};
use ora_domain::{AuditFields, Namespace, PluginId, Skill, SkillId};
use ora_effect::*;
use ora_effect_skill::{SkillDirectoryResourceAdapter, SkillPlanner};
use pretty_assertions::assert_eq;

const MANIFEST: &[u8] = b"---\nname: review\ndescription: Reviews changes\n---\n";

/// A catalog version predating both Scope and Target must not become either row's audit clock.
/// specs/test-cases/desktop/core/effect/time.md#historical-publication-uses-the-write-transaction-clock
#[test]
fn historical_publication_uses_the_write_transaction_clock()
-> Result<(), Box<dyn std::error::Error>> {
    let (directory, pool, workspace) = fixture();
    pool.with_connection(|connection| {
        connection.execute(
            "UPDATE effect_scopes SET created_at = 100, updated_at = 100",
            [],
        )?;
        Ok(())
    })?;
    let clock = TestClock::new(200);
    let effects = SqliteEffectRepository::with_clock(pool.clone(), clock.clone());
    let skills = SqliteSkillRepository::with_clock(pool.clone(), clock.clone());
    let skill = Skill::new(
        SkillId::new("legacy"),
        Namespace::local(),
        "review",
        "Reviews changes",
        AuditFields::new(10, 10, /*is_deleted*/ false),
    )?;
    skills.create_skill(skill.clone())?;
    effects.declare_consumer(
        &declaration("official/codex"),
        std::slice::from_ref(&workspace),
    )?;
    let root = directory.path().join("legacy");
    std::fs::create_dir_all(&root)?;
    std::fs::write(root.join("SKILL.md"), MANIFEST)?;
    let revision =
        LocalSkillSourceRevision::from_package(Digest::sha256(MANIFEST), root.clone(), &root)?;

    clock.set(300);
    skills.update_skill_with_source(skill.clone(), revision.clone())?;
    let scope = EffectScopeId::Workspace(workspace.id);
    let desired = effects.load_desired_state(&scope)?;
    let timestamps = pool.with_connection(|connection| {
        connection.query_row(
            "SELECT skill.updated_at, revision.revision_key, source.created_at, source.updated_at,
                    scope.created_at, scope.updated_at, status.created_at, status.updated_at,
                    desired.created_at, desired.updated_at, request.requested_at, request.updated_at
             FROM skills skill, effect_sources source, effect_revisions revision,
                  effect_scopes scope, effect_target_status status, effect_desired_effects desired,
                  effect_reconcile_requests request",
            [], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?,
                (2..12).map(|index| row.get::<_, i64>(index)).collect::<Result<Vec<_>, _>>()?)),
        ).map_err(Into::into)
    })?;
    assert_eq!(
        timestamps,
        (
            10,
            "10".to_string(),
            vec![300, 300, 100, 300, 200, 300, 300, 300, 300, 300]
        )
    );

    clock.set(50);
    skills.update_skill_with_source(skill.clone(), revision)?;
    assert_eq!(effects.load_desired_state(&scope)?, desired);
    skills.soft_delete_skill_with_source(&skill.id, /*deleted_at*/ 11)?;
    let retired = effects.load_desired_state(&scope)?;
    assert_eq!(
        (retired.generation, retired.effects),
        (desired.generation.next()?, Default::default())
    );
    let stored = pool.with_connection(|connection| {
        connection.query_row("SELECT source.updated_at, scope.updated_at, status.updated_at, request.updated_at
            FROM effect_sources source, effect_scopes scope, effect_target_status status, effect_reconcile_requests request",
            [], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?))).map_err(Into::into)
    })?;
    assert_eq!(stored, (300, 300, 300, 300));
    Ok(())
}

/// Advances a controllable clock at real filesystem boundaries, including rollback after apply.
struct TimedResource(TestClock);

impl ResourceAdapter for TimedResource {
    /// Preparation itself takes time after the Attempt's common event timestamp was sampled.
    fn prepare_operation(
        &self,
        resource: &EffectResource,
        attempt: ReconcileAttemptId,
        generation: Generation,
        sequence: u32,
        mutation: PlannedMutation,
        prepared_at: LocalTimestamp,
    ) -> Result<PreparedOperation, ResourceAdapterError> {
        let operation = ResourceAdapter::prepare_operation(
            &SkillDirectoryResourceAdapter,
            resource,
            attempt,
            generation,
            sequence,
            mutation,
            prepared_at,
        )?;
        self.0.set(50);
        Ok(operation)
    }

    /// Observation finishes after the worker began its pass.
    fn observe(
        &self,
        resource: &EffectResource,
    ) -> Result<ResourceObservation, ResourceAdapterError> {
        let observation = SkillDirectoryResourceAdapter.observe(resource)?;
        self.0.set(40);
        Ok(observation)
    }

    /// External mutation completes substantially later than journal preparation.
    fn apply(&self, operation: &EffectOperation) -> Result<ApplyReceipt, ResourceAdapterError> {
        let receipt = SkillDirectoryResourceAdapter.apply(operation)?;
        self.0.set(200);
        Ok(receipt)
    }

    /// Simulates a wall-clock correction while exact adapter verification is in progress.
    fn verify(
        &self,
        operation: &EffectOperation,
    ) -> Result<VerificationReceipt, ResourceAdapterError> {
        let receipt = SkillDirectoryResourceAdapter.verify(operation)?;
        self.0.set(75);
        Ok(receipt)
    }

    /// Retains real artifact cleanup so the test exercises the complete mutation path.
    fn cleanup(
        &self,
        artifact: &OperationArtifact,
    ) -> Result<CleanupReceipt, ResourceAdapterError> {
        SkillDirectoryResourceAdapter.cleanup(artifact)
    }
}

/// Controls when readiness evidence arrives independently from Resource verification.
struct TimedConsumer {
    clock: TestClock,
    ready_at: i64,
}

impl ConsumerAdapter for TimedConsumer {
    /// Keeps the ordinary fixture's exact coordination proof.
    fn coordinate(
        &self,
        target: &EffectTarget,
        plan: &CoordinationPlan,
    ) -> Result<CoordinationReceipt, ConsumerAdapterError> {
        ReadyConsumer.coordinate(target, plan)
    }

    /// Keeps the ordinary fixture's exact reactivation proof.
    fn reactivate(
        &self,
        target: &EffectTarget,
        plan: &CoordinationPlan,
    ) -> Result<CoordinationReceipt, ConsumerAdapterError> {
        ReadyConsumer.reactivate(target, plan)
    }

    /// Finishes readiness at the selected time, including before the previous wall-clock sample.
    fn verify_ready(
        &self,
        target: &EffectTarget,
        projection: &TargetProjection,
    ) -> Result<ReadinessReceipt, ConsumerAdapterError> {
        self.clock.set(self.ready_at);
        ReadyConsumer.verify_ready(target, projection)
    }
}

/// Real external calls must separate phase timestamps without letting clock rollback abort finalization.
/// specs/test-cases/desktop/core/effect/time.md#business-phases-and-audit-time-survive-clock-rollback
#[test]
fn business_phases_and_audit_time_survive_clock_rollback() -> Result<(), Box<dyn std::error::Error>>
{
    for ready_at in [300, 75] {
        let (directory, pool, workspace) = fixture();
        let clock = TestClock::new(30);
        let repository = SqliteEffectRepository::with_clock(pool.clone(), clock.clone());
        let root = directory.path().join("package");
        std::fs::create_dir_all(&root)?;
        std::fs::write(root.join("SKILL.md"), MANIFEST)?;
        SqliteSkillRepository::with_clock(pool.clone(), clock.clone()).replace_plugin_skills(
            &PluginId::new("official", "review")?,
            "1",
            &[PluginSkillProjection {
                name: "review".to_string(),
                description: "Reviews changes".to_string(),
                package_fingerprint: package_fingerprint(&root),
                package_root: root,
                skill_md_digest: Digest::sha256(MANIFEST),
            }],
            /*updated_at*/ 10,
        )?;
        repository.declare_consumer(&declaration("official/codex"), &[workspace])?;
        let (target, claim) = repository
            .claim_due_targets(
                &WorkerIdentity::parse("worker")?,
                LocalTimestamp::from_millis(31),
                LocalTimestamp::from_millis(1000),
                /*limit*/ 1,
            )?
            .remove(0);
        EffectReconciler::new(
            &repository,
            &SkillPlanner,
            &TimedConsumer {
                clock: clock.clone(),
                ready_at,
            },
            &TimedResource(clock.clone()),
            &clock,
        )
        .reconcile(&target, &claim, LocalTimestamp::from_millis(1000))?;
        let final_time = ready_at.max(200);
        let stored = pool.with_connection(|connection| {
            connection.query_row("SELECT prepared_at, applied_at, finalized_at, updated_at, detected_at FROM effect_operations", [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?, row.get::<_, Option<i64>>(4)?))).map_err(Into::into)
        })?;
        assert_eq!(stored, (40, 200, final_time, final_time, None));
        let view = repository
            .load_target_status(&target)?
            .expect("persisted Target");
        assert_eq!(
            (view.status.phase(), view.updated_at),
            (&TargetPhase::Current, LocalTimestamp::from_millis(ready_at))
        );
        let requests = pool.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT count(*) FROM effect_reconcile_requests",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(Into::into)
        })?;
        assert_eq!(requests, 0);
    }
    Ok(())
}
