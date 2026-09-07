use super::{declaration, fixture};
use crate::{SqliteEffectRepository, test_clock::TestClock};
use ora_effect::*;
use pretty_assertions::assert_eq;

/// Audit time cannot accelerate retry eligibility or rewrite a Condition's observation history.
/// specs/test-cases/desktop/core/effect/time.md#scheduling-and-condition-history-use-independent-time
#[test]
fn scheduling_and_condition_history_use_independent_time() -> Result<(), Box<dyn std::error::Error>>
{
    let (_directory, pool, workspace) = fixture();
    let clock = TestClock::new(300);
    let repository = SqliteEffectRepository::with_clock(pool.clone(), clock.clone());
    let mut consumer = declaration("official/codex");
    consumer.resources.clear();
    repository.declare_consumer(&consumer, std::slice::from_ref(&workspace))?;
    let worker = WorkerIdentity::parse("worker")?;
    let lease = LocalTimestamp::from_millis(1000);
    let (target, claim) = repository
        .claim_due_targets(
            &worker,
            LocalTimestamp::from_millis(10),
            lease,
            /*limit*/ 1,
        )?
        .remove(0);
    repository.schedule_retry(
        &target,
        &claim,
        LocalTimestamp::from_millis(30),
        LocalTimestamp::from_millis(20),
    )?;
    assert_eq!(
        repository.claim_due_targets(
            &worker,
            LocalTimestamp::from_millis(25),
            lease,
            /*limit*/ 1
        )?,
        Vec::new()
    );
    let (_, claim) = repository
        .claim_due_targets(
            &worker,
            LocalTimestamp::from_millis(30),
            lease,
            /*limit*/ 1,
        )?
        .remove(0);
    let condition = ConditionProposal {
        owner: ConditionOwner::Target(target.clone()),
        subject: ConditionSubject::Target(target.clone()),
        code: StableConditionCode::from_static("test.blocked"),
        impact: ConditionImpact::Blocking,
        retry: ConditionRetry::OnChange,
        generation: ConditionGeneration::Unscoped,
        safe_details: SafeConditionDetails {
            message: "Waiting for a change".to_string(),
            parameters: Default::default(),
        },
    };
    let snapshot = repository.load_reconcile_snapshot(&target, &claim)?;
    repository.block_target(
        &target,
        &claim,
        snapshot.target_status,
        Vec::new(),
        vec![condition.clone()],
    )?;
    let before = repository
        .load_target_status(&target)?
        .expect("blocked Target");
    clock.set(200);
    repository.request_reconcile(&target, LocalTimestamp::from_millis(31))?;
    let (_, claim) = repository
        .claim_due_targets(
            &worker,
            LocalTimestamp::from_millis(31),
            lease,
            /*limit*/ 1,
        )?
        .remove(0);
    let snapshot = repository.load_reconcile_snapshot(&target, &claim)?;
    repository.block_target(
        &target,
        &claim,
        snapshot.target_status,
        Vec::new(),
        vec![condition],
    )?;
    assert_eq!(repository.load_target_status(&target)?, Some(before));

    clock.set(100);
    consumer
        .capabilities
        .coordination_contracts
        .insert("test.contract".to_string());
    repository.declare_consumer(&consumer, &[workspace])?;
    repository.retire_consumer(&consumer.consumer)?;
    let timestamps = pool.with_connection(|connection| {
        connection
            .query_row(
                "SELECT created_at, updated_at, lifecycle FROM effect_targets WHERE id = ?1",
                [target.as_str()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .map_err(Into::into)
    })?;
    assert_eq!(timestamps, (300, 300, "retiring".to_string()));
    Ok(())
}
