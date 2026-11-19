use super::*;
use pretty_assertions::assert_eq;
use std::fs;

/// Persists the directory blocker without claiming observation, then converges after explicit retry.
#[test]
fn linked_skill_blocks_the_bound_target_until_the_link_is_fixed() {
    let (directory, pool, workspace) = fixture();
    let vendor = directory
        .path()
        .join("workspace")
        .join(".agents")
        .join("skills")
        .join("review")
        .join("vendor");
    fs::create_dir_all(&vendor).unwrap();
    let destination = directory.path().join("external");
    fs::create_dir(&destination).unwrap();
    let link = vendor.join("linked");

    #[cfg(unix)]
    std::os::unix::fs::symlink(&destination, &link).unwrap();
    #[cfg(windows)]
    {
        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&destination)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }

    let repository = SqliteEffectRepository::with_clock(pool, crate::test_clock::TestClock::new(1));
    repository
        .declare_consumer(&declaration("official/codex"), &[workspace])
        .unwrap();
    let worker = WorkerIdentity::parse("worker").unwrap();
    let lease = LocalTimestamp::from_millis(100);
    let (target, claim) = repository
        .claim_due_targets(
            &worker,
            LocalTimestamp::from_millis(11),
            lease,
            /*limit*/ 1,
        )
        .unwrap()
        .remove(0);
    let reconciler = EffectReconciler::new(
        &repository,
        &SkillPlanner,
        &ReadyConsumer,
        &SkillDirectoryResourceAdapter,
        &FixedTimestamp,
    );
    let ReconcileOutcome::Blocked { conditions, .. } =
        reconciler.reconcile(&target, &claim, lease).unwrap()
    else {
        panic!("linked skill should block reconciliation");
    };
    let [condition] = conditions.as_slice() else {
        panic!("expected one resource blocker");
    };
    assert!(matches!(condition.owner, ConditionOwner::Resource(_)));
    assert_eq!(condition.code.as_str(), "skill_directory_link");
    assert_eq!(condition.impact, ConditionImpact::Blocking);
    let relative = std::path::Path::new("review").join("vendor").join("linked");
    assert_eq!(
        condition.safe_details.message,
        format!("Symbolic links and Windows junctions are not supported: {relative:?}")
    );
    assert!(
        !condition
            .safe_details
            .message
            .contains(directory.path().to_str().unwrap())
    );

    let persisted = repository.load_target_status(&target).unwrap().unwrap();
    assert_eq!(persisted.conditions.len(), 1);
    assert_eq!(persisted.conditions[0].safe_details, condition.safe_details);
    assert!(repository.load_unfinished_operations().unwrap().is_empty());

    #[cfg(unix)]
    fs::remove_file(&link).unwrap();
    #[cfg(windows)]
    fs::remove_dir(&link).unwrap();
    fs::create_dir(&link).unwrap();
    assert!(
        repository
            .request_reconcile(&target, LocalTimestamp::from_millis(12))
            .unwrap()
    );
    let (_, claim) = repository
        .claim_due_targets(
            &worker,
            LocalTimestamp::from_millis(13),
            lease,
            /*limit*/ 1,
        )
        .unwrap()
        .remove(0);
    assert!(matches!(
        reconciler.reconcile(&target, &claim, lease).unwrap(),
        ReconcileOutcome::Current { .. }
    ));
    assert!(
        repository
            .load_target_status(&target)
            .unwrap()
            .unwrap()
            .conditions
            .is_empty()
    );
}
