use super::*;
use pretty_assertions::assert_eq;

/// Keeps newer intent queued while the older filesystem mutation commits its proven progress.
#[test]
fn older_mutation_finalizes_and_newer_intent_converges_on_the_next_pass() {
    // Core case: specs/test-cases/desktop/core/effect/convergence.md#older-completion-preserves-newer-target-intent
    let (directory, pool, workspace) = fixture();
    let package_root = directory.path().join("plugin-skill");
    std::fs::create_dir_all(&package_root).unwrap();
    let manifest = b"---\nname: review\ndescription: Reviews changes\n---\n";
    std::fs::write(package_root.join("SKILL.md"), manifest).unwrap();
    let projection = PluginSkillProjection {
        name: "review".to_string(),
        description: "Reviews changes".to_string(),
        package_fingerprint: package_fingerprint(&package_root),
        package_root,
        skill_md_digest: Digest::sha256(manifest),
    };
    let publisher =
        SqliteSkillRepository::with_clock(pool.clone(), crate::test_clock::TestClock::new(20));
    publisher
        .replace_plugin_skills(
            &PluginId::new("official", "review").unwrap(),
            "1.0.0",
            std::slice::from_ref(&projection),
            /*updated_at*/ 10,
        )
        .unwrap();
    let repository = SqliteEffectRepository::with_clock(pool, crate::test_clock::TestClock::new(1));
    repository
        .declare_consumer(&declaration("official/codex"), &[workspace])
        .unwrap();
    let worker = WorkerIdentity::parse("worker-1").unwrap();
    let (target, claim) = repository
        .claim_due_targets(
            &worker,
            LocalTimestamp::from_millis(11),
            LocalTimestamp::from_millis(100),
            /*limit*/ 1,
        )
        .unwrap()
        .remove(0);
    let consumer = PublishSkillOnVerify {
        repository: publisher,
        projection,
        // More wakeups than local transitions also catches overwriting status_version.
        versions: vec!["2.0.0", "3.0.0", "4.0.0", "5.0.0", "6.0.0"],
        published: Mutex::new(false),
    };
    let outcome = EffectReconciler::new(
        &repository,
        &SkillPlanner,
        &consumer,
        &SkillDirectoryResourceAdapter,
        &FixedTimestamp,
    )
    .reconcile(&target, &claim, LocalTimestamp::from_millis(100))
    .unwrap();
    assert_eq!(
        outcome,
        ReconcileOutcome::Mutated {
            target: target.clone(),
            generation: Generation::new(1),
            operations: 1,
        }
    );
    assert_eq!(
        repository.load_target_status(&target).unwrap(),
        Some(TargetStatusView {
            status: TargetStatus::restore(
                target.clone(),
                TargetProgress::restore(
                    Generation::new(6),
                    Generation::new(1),
                    Generation::new(1),
                    Generation::new(1),
                )
                .unwrap(),
                TargetPhase::Pending,
                StatusVersion::new(7).unwrap(),
            ),
            updated_at: LocalTimestamp::from_millis(20),
            conditions: Vec::new(),
        })
    );
    assert_eq!(repository.load_unfinished_operations().unwrap(), Vec::new());
    let materialized = directory
        .path()
        .join("workspace")
        .join(".agents")
        .join("skills")
        .join("review")
        .join("SKILL.md");
    assert_eq!(std::fs::read(&materialized).unwrap(), manifest);

    let (next_target, next_claim) = repository
        .claim_due_targets(
            &worker,
            LocalTimestamp::from_millis(21),
            LocalTimestamp::from_millis(100),
            /*limit*/ 1,
        )
        .unwrap()
        .remove(0);
    assert_eq!(next_target, target);
    EffectReconciler::new(
        &repository,
        &SkillPlanner,
        &ReadyConsumer,
        &SkillDirectoryResourceAdapter,
        &FixedTimestamp,
    )
    .reconcile(&target, &next_claim, LocalTimestamp::from_millis(100))
    .unwrap();
    assert_eq!(
        repository.load_target_status(&target).unwrap(),
        Some(TargetStatusView {
            status: TargetStatus::restore(
                target,
                TargetProgress::restore(
                    Generation::new(6),
                    Generation::new(6),
                    Generation::new(6),
                    Generation::new(6),
                )
                .unwrap(),
                TargetPhase::Current,
                StatusVersion::new(10).unwrap(),
            ),
            updated_at: LocalTimestamp::from_millis(20),
            conditions: Vec::new(),
        })
    );
    assert_eq!(
        repository
            .claim_due_targets(
                &worker,
                LocalTimestamp::from_millis(22),
                LocalTimestamp::from_millis(100),
                /*limit*/ 1,
            )
            .unwrap(),
        Vec::new()
    );
    assert_eq!(std::fs::read(materialized).unwrap(), manifest);
}
