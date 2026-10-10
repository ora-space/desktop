use super::{ReadyConsumer, declaration, fixture, package_fingerprint};
use crate::{
    PluginSkillProjection, SqliteEffectRepository, SqliteSkillRepository, test_clock::TestClock,
};
use ora_domain::PluginId;
use ora_domain::{Workspace, WorkspaceLocation};
use ora_effect::*;
use ora_effect_skill::{SkillDirectoryResourceAdapter, SkillPlanner};
use pretty_assertions::assert_eq;
use rusqlite::params;

/// Builds one Consumer declaration whose Skill directory uses the quiesce coordination contract.
fn quiesce_declaration(stable_key: &str) -> ConsumerDeclaration {
    let mut consumer = declaration(stable_key);
    let contract = CoordinationContract::agent_restart_v1();
    consumer
        .capabilities
        .coordination_contracts
        .insert(contract.capability_key());
    consumer.resources[0].coordination = CoordinationRequirement::QuiesceBeforeMutation(contract);
    consumer
}

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

/// Reconciles every currently due Target once with the production planner and adapters.
fn reconcile_due(
    repository: &SqliteEffectRepository<TestClock>,
    clock: &TestClock,
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

/// Loads the lowest persisted Target projection identity for one Target.
fn persisted_target_projection(
    pool: &crate::RepositoryPool,
    target: &EffectTargetId,
) -> Option<(i64, String)> {
    use rusqlite::OptionalExtension;
    pool.with_connection(|connection| {
        connection
            .query_row(
                "SELECT generation, digest FROM effect_target_projections
                 WHERE target_id = ?1 ORDER BY generation LIMIT 1",
                params![target.as_str()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(Into::into)
    })
    .unwrap_or_else(|error| panic!("load persisted projection: {error}"))
}

/// Adds one more local-filesystem Workspace with its Effect Scope to the fixture database.
fn add_workspace(pool: &crate::RepositoryPool, workspace: &Workspace) {
    let WorkspaceLocation::LocalFilesystem { path } = &workspace.location else {
        panic!("fixture workspaces are local filesystem Workspaces");
    };
    std::fs::create_dir_all(path).unwrap_or_else(|error| panic!("create Worktree: {error}"));
    pool.with_connection_mut(|connection| {
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO workspace_locations (
                 id, location_kind, locator_version, locator_json, created_at, updated_at
             ) VALUES (?1, 'local_filesystem', 1, ?2, 1, 1)",
            params![
                format!("location-{}", workspace.id.as_ref()),
                serde_json::json!({ "path": path }).to_string()
            ],
        )?;
        transaction.execute(
            "INSERT INTO workspaces (
                 id, project_id, workspace_kind, location_id, lifecycle,
                 created_at, updated_at, is_deleted
             ) VALUES (?1, ?2, ?3, ?4, 'active', 1, 1, 0)",
            params![
                workspace.id.as_ref(),
                "project-1",
                workspace.kind.database_value(),
                format!("location-{}", workspace.id.as_ref()),
            ],
        )?;
        transaction.execute(
            "INSERT INTO effect_scopes (
                 id, scope_kind, workspace_id, lifecycle, generation, created_at, updated_at
             ) VALUES (?1, 'workspace', ?2, 'active', 0, 1, 1)",
            params![
                EffectScopeId::Workspace(workspace.id.clone()).storage_key(),
                workspace.id.as_ref()
            ],
        )?;
        transaction.commit()?;
        Ok::<_, crate::DatabaseError>(())
    })
    .unwrap_or_else(|error| panic!("insert Workspace fixture: {error}"));
}

/// A retiring Target whose Worktree directory was already deleted must finish retiring instead
/// of failing observation forever: nothing on disk remains to clean up (issue #6).
#[test]
fn retiring_target_on_a_deleted_worktree_finishes_retiring() {
    let (directory, pool, workspace) = fixture();
    let clock = TestClock::new(100);
    let skills = SqliteSkillRepository::with_clock(pool.clone(), clock.clone());
    let repository = SqliteEffectRepository::with_clock(pool.clone(), clock.clone());
    let worker = WorkerIdentity::parse("worker-1").unwrap_or_else(|e| panic!("worker: {e}"));

    // The Skill plugin owns the catalog; the local codeagent only consumes it.
    let skill_plugin =
        PluginId::new("official", "skill-pack").unwrap_or_else(|e| panic!("plugin: {e}"));
    skills
        .replace_plugin_skills(
            &skill_plugin,
            "1.0.0",
            &[skill_package(
                directory.path(),
                "skill-review",
                b"---\nname: review\ndescription: Reviews changes\n---\ncontent\n",
            )],
            110,
        )
        .unwrap_or_else(|e| panic!("publish Skill plugin: {e}"));
    let local_consumer = quiesce_declaration("local/ora-space.codeagent");
    repository
        .declare_consumer(&local_consumer, std::slice::from_ref(&workspace))
        .unwrap_or_else(|e| panic!("declare local Consumer: {e}"));
    let outcomes = reconcile_due(&repository, &clock, &worker, 120);
    assert_eq!(outcomes.len(), 1);
    assert!(matches!(
        outcomes[0],
        Ok((_, ReconcileOutcome::Mutated { .. }))
    ));
    let local_target = outcomes[0].as_ref().unwrap().0.clone();

    // The Worktree directory disappears before the worker processes the retirement.
    std::fs::remove_dir_all(directory.path().join("workspace"))
        .unwrap_or_else(|e| panic!("delete Worktree: {e}"));
    repository
        .retire_consumer(&local_consumer.consumer)
        .unwrap_or_else(|e| panic!("retire local Consumer: {e}"));

    // Observation of a deleted Worktree must not fail the retirement forever.
    let outcomes = reconcile_due(&repository, &clock, &worker, 140);
    assert_eq!(outcomes.len(), 1, "the retiring Target is claimed once");
    let (retired, outcome) = match outcomes.into_iter().next() {
        Some(Ok((target, outcome))) => (target, outcome),
        Some(Err((target, error))) => {
            panic!("retiring Target on deleted Worktree failed (issue #6): {target} {error:?}")
        }
        None => panic!("the retiring Target was not claimed"),
    };
    assert_eq!(retired, local_target);
    assert_eq!(
        outcome,
        ReconcileOutcome::Current {
            target: local_target.clone(),
            generation: Generation::new(2),
        },
        "the retiring Target forgets its ledger and finishes retiring"
    );
    let remaining = pool
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT
                        (SELECT COUNT(*) FROM effect_targets),
                        (SELECT COUNT(*) FROM effect_reconcile_requests),
                        (SELECT COUNT(*) FROM effect_managed_items)
                     FROM (SELECT 1)",
                    [],
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
        .unwrap_or_else(|e| panic!("load remaining Effect rows: {e}"));
    assert_eq!(remaining, (0, 0, 0), "retirement removed every owned row");
}

/// Replacing one Consumer's declaration (a plugin update) retires the old Target in place.
/// The Scope epoch must advance with that topology change, or the retiring Target collides
/// with the projection it persisted while active at the same generation (issue #6).
#[test]
fn replacing_a_consumer_declaration_retires_the_old_target_at_a_new_epoch() {
    let (directory, pool, workspace) = fixture();
    let clock = TestClock::new(100);
    let skills = SqliteSkillRepository::with_clock(pool.clone(), clock.clone());
    let repository = SqliteEffectRepository::with_clock(pool.clone(), clock.clone());
    let worker = WorkerIdentity::parse("worker-1").unwrap_or_else(|e| panic!("worker: {e}"));

    let skill_plugin =
        PluginId::new("official", "skill-pack").unwrap_or_else(|e| panic!("plugin: {e}"));
    skills
        .replace_plugin_skills(
            &skill_plugin,
            "1.0.0",
            &[skill_package(
                directory.path(),
                "skill-review",
                b"---\nname: review\ndescription: Reviews changes\n---\ncontent\n",
            )],
            110,
        )
        .unwrap_or_else(|e| panic!("publish Skill plugin: {e}"));
    let consumer = declaration("local/ora-space.codeagent");
    repository
        .declare_consumer(&consumer, std::slice::from_ref(&workspace))
        .unwrap_or_else(|e| panic!("declare Consumer: {e}"));
    let outcomes = reconcile_due(&repository, &clock, &worker, 120);
    assert_eq!(outcomes.len(), 1);
    let (old_target, outcome) = match outcomes.into_iter().next() {
        Some(Ok((target, outcome))) => (target, outcome),
        Some(Err((target, error))) => panic!("first reconcile {target}: {error:?}"),
        None => panic!("the first Target was not claimed"),
    };
    assert!(matches!(outcome, ReconcileOutcome::Mutated { .. }));

    // The plugin update changes its declaration digest (here: the coordination contract),
    // which retires the converged Target and creates its replacement in the same Scope.
    let updated = quiesce_declaration("local/ora-space.codeagent");
    repository
        .declare_consumer(&updated, std::slice::from_ref(&workspace))
        .unwrap_or_else(|e| panic!("declare updated Consumer: {e}"));

    let outcomes = reconcile_due(&repository, &clock, &worker, 130);
    assert_eq!(
        outcomes.len(),
        2,
        "the retiring and replacement Targets are claimed"
    );
    let mut replacement = None;
    for outcome in outcomes {
        let (target, outcome) =
            outcome.unwrap_or_else(|(target, error)| panic!("Target {target}: {error:?}"));
        if target == old_target {
            assert_eq!(
                outcome,
                ReconcileOutcome::Current {
                    target: old_target.clone(),
                    generation: Generation::new(2),
                },
                "the replaced Target finishes retiring at the new epoch"
            );
        } else {
            replacement = Some((target, outcome));
        }
    }
    let (replacement_target, replacement_outcome) =
        replacement.expect("the replacement Target was claimed");
    assert_eq!(
        replacement_outcome,
        ReconcileOutcome::Current {
            target: replacement_target.clone(),
            generation: Generation::new(2),
        },
        "the replacement Target converges at the new epoch"
    );
    let remaining = pool
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT
                        (SELECT COUNT(*) FROM effect_targets),
                        (SELECT COUNT(*) FROM effect_reconcile_requests),
                        (SELECT COUNT(*) FROM effect_target_status
                         WHERE phase = 'current' AND ready_generation = 2)
                     FROM (SELECT 1)",
                    [],
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
        .unwrap_or_else(|e| panic!("load remaining rows: {e}"));
    assert_eq!(
        remaining,
        (1, 0, 1),
        "only the replacement Target remains, current and ready"
    );
}

/// Reproduces issue #6: a pure-consumer local plugin (the Skill catalog belongs to a separate
/// Skill plugin) is uninstalled and replaced by the Marketplace build of the same agent.
///
/// The retiring Target replans its contribution at an unchanged Scope generation, so the
/// immutable projection identity `(target_id, generation, consumer_revision_id)` keeps the
/// pre-retirement content and every reconcile fails forever.
///
/// https://github.com/wanglongan587/desktop/issues/6
#[test]
fn retiring_local_consumer_handover_to_marketplace_consumer_converges() {
    let (directory, pool, workspace) = fixture();
    let clock = TestClock::new(100);
    let skills = SqliteSkillRepository::with_clock(pool.clone(), clock.clone());
    let repository = SqliteEffectRepository::with_clock(pool.clone(), clock.clone());
    let worker = WorkerIdentity::parse("worker-1").unwrap_or_else(|e| panic!("worker: {e}"));

    // 1. A separate Skill plugin publishes the Skill; the local codeagent only consumes it.
    let skill_plugin =
        PluginId::new("official", "skill-pack").unwrap_or_else(|e| panic!("plugin: {e}"));
    let skill_manifest = b"---\nname: review\ndescription: Reviews changes\n---\ncontent\n";
    skills
        .replace_plugin_skills(
            &skill_plugin,
            "1.0.0",
            &[skill_package(
                directory.path(),
                "skill-review",
                skill_manifest,
            )],
            110,
        )
        .unwrap_or_else(|e| panic!("publish Skill plugin: {e}"));
    let local_consumer = quiesce_declaration("local/ora-space.codeagent");
    repository
        .declare_consumer(&local_consumer, std::slice::from_ref(&workspace))
        .unwrap_or_else(|e| panic!("declare local Consumer: {e}"));

    // 2. The local Target reconciles to Current and materializes the Skill.
    let outcomes = reconcile_due(&repository, &clock, &worker, 120);
    assert_eq!(outcomes.len(), 1, "one local Target is claimed");
    let Ok((local_target, outcome)) = &outcomes[0] else {
        panic!("local Target reconcile failed: {:?}", outcomes[0]);
    };
    assert_eq!(
        outcome,
        &ReconcileOutcome::Mutated {
            target: local_target.clone(),
            generation: Generation::new(1),
            operations: 1,
        }
    );
    let materialized = directory
        .path()
        .join("workspace")
        .join(".agents")
        .join("skills")
        .join("review")
        .join("SKILL.md");
    assert_eq!(
        std::fs::read(&materialized).unwrap_or_else(|e| panic!("read materialized Skill: {e}")),
        skill_manifest
    );
    let persisted = persisted_target_projection(&pool, local_target);
    assert_eq!(
        persisted.as_ref().map(|(generation, _)| *generation),
        Some(1),
        "the local Target persisted one non-empty projection at generation 1"
    );

    // 3. Uninstall the local plugin and install the Marketplace build before the worker runs
    //    its next pass (the exact interleaving from the issue). The Skill plugin still owns
    //    the catalog, so the Scope generation only advances through the retirement topology
    //    change.
    repository
        .retire_consumer(&local_consumer.consumer)
        .unwrap_or_else(|e| panic!("retire local Consumer: {e}"));
    let market_consumer = quiesce_declaration("ora-space-marketplace.6905518b/ora-space.codeagent");
    repository
        .declare_consumer(&market_consumer, std::slice::from_ref(&workspace))
        .unwrap_or_else(|e| panic!("declare marketplace Consumer: {e}"));

    // 4. A restarted worker (new identity, same durable database) claims everything due: the
    //    retiring local Target and the new marketplace Target share one Skill directory
    //    Resource, and both must converge instead of failing on the shared binding.
    let restarted = WorkerIdentity::parse("worker-after-restart")
        .unwrap_or_else(|e| panic!("restarted worker: {e}"));
    let outcomes = reconcile_due(&repository, &clock, &restarted, 140);
    assert_eq!(outcomes.len(), 2, "both Targets are claimed: {outcomes:?}");
    let mut market_target = None;
    for outcome in &outcomes {
        let (target, outcome) = outcome.as_ref().unwrap_or_else(|(target, error)| {
            panic!("Target {target} reconcile failed (issue #6): {error:?}")
        });
        if target == local_target {
            assert_eq!(
                outcome,
                &ReconcileOutcome::Current {
                    target: local_target.clone(),
                    generation: Generation::new(2),
                },
                "the retiring local Target finishes retiring with the marketplace contributor"
            );
        } else {
            assert_eq!(
                outcome,
                &ReconcileOutcome::Current {
                    target: target.clone(),
                    generation: Generation::new(2),
                },
                "the marketplace Target converges onto the shared Resource"
            );
            market_target = Some(target.clone());
        }
    }
    let market_target = market_target.expect("the marketplace Target was claimed");
    let local_rows = pool
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT COUNT(*) FROM effect_targets WHERE id = ?1",
                    params![local_target.as_str()],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(Into::into)
        })
        .unwrap_or_else(|e| panic!("count local Target rows: {e}"));
    assert_eq!(local_rows, 0, "the retired local Target row is deleted");
    assert_eq!(
        std::fs::read(&materialized).unwrap_or_else(|e| panic!("read materialized Skill: {e}")),
        skill_manifest,
        "the shared Skill directory keeps its materialized content"
    );

    // 5. Control from the issue: a freshly created Worktree using the same marketplace agent
    //    converges independently.
    let fresh_workspace = Workspace::new(
        ora_domain::WorkspaceId::new("workspace-fresh"),
        ora_domain::ProjectId::new("project-1"),
        ora_domain::WorkspaceKind::Isolated,
        WorkspaceLocation::local_filesystem(
            directory
                .path()
                .join("workspace-fresh")
                .to_string_lossy()
                .into_owned(),
        ),
        ora_domain::WorkspaceLifecycle::Active,
        ora_domain::AuditFields::new(1, 1, false),
    );
    add_workspace(&pool, &fresh_workspace);
    repository
        .declare_consumer(&market_consumer, &[fresh_workspace.clone()])
        .unwrap_or_else(|e| panic!("declare marketplace Consumer for the fresh Worktree: {e}"));
    let outcomes = reconcile_due(&repository, &clock, &restarted, 150);
    assert_eq!(outcomes.len(), 1, "the fresh Worktree Target is claimed");
    let (fresh_target, fresh_outcome) = match outcomes.into_iter().next() {
        Some(Ok((target, outcome))) => (target, outcome),
        Some(Err((target, error))) => {
            panic!("fresh Worktree Target {target} failed (issue #6 control): {error:?}")
        }
        None => panic!("the fresh Worktree Target was not claimed"),
    };
    assert_eq!(
        fresh_outcome,
        ReconcileOutcome::Mutated {
            target: fresh_target.clone(),
            generation: Generation::new(1),
            operations: 1,
        }
    );

    // 6. A second pass proves convergence: no further work remains anywhere.
    let outcomes = reconcile_due(&repository, &clock, &restarted, 170);
    assert!(
        outcomes.is_empty(),
        "no Target is due after convergence: {outcomes:?}"
    );
    let final_state = pool
        .with_connection(|connection| {
            let targets: Vec<(String, String)> = connection
                .prepare("SELECT id, lifecycle FROM effect_targets ORDER BY id")
                .unwrap()
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            let status: Vec<(String, String, i64)> = connection
                .prepare(
                    "SELECT target_id, phase, ready_generation
                     FROM effect_target_status ORDER BY target_id",
                )
                .unwrap()
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                })
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            let requests: Vec<String> = connection
                .prepare("SELECT target_id FROM effect_reconcile_requests ORDER BY target_id")
                .unwrap()
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            Ok::<_, crate::DatabaseError>((targets, status, requests))
        })
        .unwrap();
    let (targets, status, requests) = final_state;
    let mut expected_targets = vec![
        (fresh_target.as_str().to_string(), "active".to_string()),
        (market_target.as_str().to_string(), "active".to_string()),
    ];
    expected_targets.sort();
    assert_eq!(
        targets, expected_targets,
        "only the two marketplace Targets remain; the local Target finished retiring"
    );
    let mut expected_status = vec![
        (fresh_target.as_str().to_string(), "current".to_string(), 1),
        (market_target.as_str().to_string(), "current".to_string(), 2),
    ];
    expected_status.sort();
    assert_eq!(
        status, expected_status,
        "both marketplace Targets are current and ready"
    );
    assert!(
        requests.is_empty(),
        "no reconcile requests remain: {requests:?}"
    );
}
