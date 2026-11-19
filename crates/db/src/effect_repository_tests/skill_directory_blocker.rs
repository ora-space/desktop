use super::*;
use pretty_assertions::assert_eq;
use std::fs;

/// Runs the full blocked-to-ready transition for each invalid tree shape.
#[test]
fn unsafe_skill_entries_block_the_bound_target_until_fixed() {
    for path in [
        ".agents/skills/review/vendor/linked",
        ".agents/skills/linked",
        ".agents",
        "",
    ] {
        assert_blocked_then_repaired(path, Entry::DirectoryLink);
    }
    #[cfg(unix)]
    for (path, entry) in [
        (".agents/skills/review/dangling", Entry::DanglingLink),
        (".agents/skills/review/file-link", Entry::FileLink),
        (".agents/skills/fifo", Entry::Fifo),
    ] {
        assert_blocked_then_repaired(path, entry);
    }
}

#[derive(Clone, Copy)]
enum Entry {
    DirectoryLink,
    #[cfg(unix)]
    DanglingLink,
    #[cfg(unix)]
    FileLink,
    #[cfg(unix)]
    Fifo,
}

/// Creates the entry without requiring Windows Developer Mode for junction coverage.
fn create_entry(entry: Entry, destination: &std::path::Path, link: &std::path::Path) {
    #[cfg(unix)]
    match entry {
        Entry::DirectoryLink => {
            fs::create_dir(destination).unwrap();
        }
        Entry::DanglingLink => {}
        Entry::FileLink => {
            fs::write(destination, b"external").unwrap();
        }
        Entry::Fifo => {
            assert!(
                std::process::Command::new("mkfifo")
                    .arg(link)
                    .status()
                    .unwrap()
                    .success()
            );
            return;
        }
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(destination, link).unwrap();
    #[cfg(windows)]
    {
        let Entry::DirectoryLink = entry;
        fs::create_dir(destination).unwrap();
        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(destination)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
}

/// Persists the exact safe path and allows an explicit retry after the entry is fixed.
fn assert_blocked_then_repaired(relative: &str, entry: Entry) {
    let (directory, pool, workspace) = fixture();
    let workspace_root = directory.path().join("workspace");
    let link = relative
        .split('/')
        .filter(|part| !part.is_empty())
        .fold(workspace_root.clone(), |path, part| path.join(part));
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    if relative.is_empty() {
        fs::remove_dir(&workspace_root).unwrap();
    }
    create_entry(entry, &directory.path().join("external"), &link);
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
    let (code, message) = match entry {
        #[cfg(unix)]
        Entry::Fifo => (
            "unsafe_skill_directory",
            "Unsupported entry in the skill directory",
        ),
        Entry::DirectoryLink => (
            "skill_directory_link",
            "Symbolic links and Windows junctions are not supported",
        ),
        #[cfg(unix)]
        Entry::DanglingLink | Entry::FileLink => (
            "skill_directory_link",
            "Symbolic links and Windows junctions are not supported",
        ),
    };
    assert_eq!(condition.code.as_str(), code);
    assert_eq!(condition.impact, ConditionImpact::Blocking);
    let display = if relative.is_empty() {
        "workspace folder"
    } else {
        relative
    };
    assert_eq!(
        condition.safe_details.message,
        format!("{message}: {display}")
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
    // A desktop metadata file must not keep the repaired Target blocked.
    let skill_root = workspace_root.join(".agents").join("skills");
    fs::create_dir_all(&skill_root).unwrap();
    fs::write(skill_root.join(".DS_Store"), b"desktop metadata").unwrap();
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

/// A regular file owning a required native name must block before mutation is journaled.
#[test]
fn occupied_skill_name_blocks_without_a_journal_and_retries_after_cleanup() {
    for name in ["review", "REVIEW"] {
        let (directory, pool, workspace) = fixture();
        let skill_root = directory
            .path()
            .join("workspace")
            .join(".agents")
            .join("skills");
        fs::create_dir_all(&skill_root).unwrap();
        let occupied = skill_root.join(name);
        fs::write(&occupied, b"user file").unwrap();
        fs::write(skill_root.join(".DS_Store"), b"desktop metadata").unwrap();
        let projection = super::consumer_handover::skill_package(
            directory.path(),
            "package",
            b"---\nname: review\ndescription: Reviews changes\n---\n",
        );
        let clock = crate::test_clock::TestClock::new(1);
        SqliteSkillRepository::with_clock(pool.clone(), clock.clone())
            .replace_plugin_skills(
                &PluginId::new("official", "review").unwrap(),
                "1.0.0",
                &[projection],
                /*updated_at*/ 1,
            )
            .unwrap();
        let repository = SqliteEffectRepository::with_clock(pool.clone(), clock);
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

        assert!(matches!(
            reconciler.reconcile(&target, &claim, lease).unwrap(),
            ReconcileOutcome::Blocked { .. }
        ));
        let status = repository.load_target_status(&target).unwrap().unwrap();
        let [condition] = status.conditions.as_slice() else {
            panic!("expected one resource blocker");
        };
        assert!(matches!(condition.owner, ConditionOwner::Resource(_)));
        assert_eq!(condition.impact, ConditionImpact::Blocking);
        assert_eq!(condition.code.as_str(), "unsafe_skill_directory");
        assert_eq!(
            condition.safe_details.message,
            format!(
                "Move the regular file that occupies the skill directory: .agents/skills/{name}"
            )
        );
        let journal_count: i64 = pool
            .with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT COUNT(*) FROM effect_reconcile_attempts",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(Into::into)
            })
            .unwrap();
        assert_eq!(journal_count, 0);
        assert!(repository.load_unfinished_operations().unwrap().is_empty());
        assert_eq!(fs::read(&occupied).unwrap(), b"user file");

        fs::remove_file(&occupied).unwrap();
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
            ReconcileOutcome::Mutated { .. }
        ));
        assert!(skill_root.join("review").join("SKILL.md").is_file());
        assert!(
            repository
                .load_target_status(&target)
                .unwrap()
                .unwrap()
                .conditions
                .is_empty()
        );
    }
}
