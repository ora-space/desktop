use crate::{
    DatabaseBootstrapper, DatabaseLocation, PluginSkillProjection, RepositoryPool,
    SqliteEffectRepository, SqliteSkillRepository, TimestampSource, default_migration_catalog,
};
use ora_domain::{
    AuditFields, PluginId, ProjectId, Workspace, WorkspaceId, WorkspaceKind, WorkspaceLifecycle,
    WorkspaceLocation,
};
use ora_effect::*;
use ora_effect_skill::{SkillDirectoryResourceAdapter, SkillPlanner};
use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;
use rusqlite::params;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, PoisonError};
use tempfile::TempDir;

#[derive(Clone, Copy, Debug)]
struct FixedTimestamp;

impl TimestampSource for FixedTimestamp {
    fn current_timestamp_millis(&self) -> i64 {
        1
    }
}

#[derive(Clone, Copy, Debug)]
struct ReadyConsumer;

impl ConsumerAdapter for ReadyConsumer {
    /// Returns a matching barrier receipt if a test declaration opts into coordination.
    fn coordinate(
        &self,
        target: &EffectTarget,
        plan: &CoordinationPlan,
    ) -> Result<CoordinationReceipt, ConsumerAdapterError> {
        coordination_receipt(target, plan, CoordinationReceiptState::SafeToMutate)
    }

    /// Returns a matching reactivation receipt for the same immutable coordination plan.
    fn reactivate(
        &self,
        target: &EffectTarget,
        plan: &CoordinationPlan,
    ) -> Result<CoordinationReceipt, ConsumerAdapterError> {
        coordination_receipt(target, plan, CoordinationReceiptState::Reactivated)
    }

    /// Proves readiness only for the exact Target projection supplied by the reconciler.
    fn verify_ready(
        &self,
        target: &EffectTarget,
        projection: &TargetProjection,
    ) -> Result<ReadinessReceipt, ConsumerAdapterError> {
        Ok(ReadinessReceipt {
            target: target.identity.clone(),
            generation: projection.generation,
            consumer_revision: target.consumer_revision.clone(),
            projection: projection.digest.clone(),
            proof: AdapterReceipt {
                version: 1,
                payload: serde_json::json!({ "ready": true }),
            },
        })
    }
}

/// Publishes a newer Desired generation while an older projection is being verified.
struct PublishSkillOnVerify {
    repository: SqliteSkillRepository,
    projection: PluginSkillProjection,
    published: Mutex<bool>,
}

impl ConsumerAdapter for PublishSkillOnVerify {
    /// Delegates coordination because this probe exercises a no-mutation projection.
    fn coordinate(
        &self,
        target: &EffectTarget,
        plan: &CoordinationPlan,
    ) -> Result<CoordinationReceipt, ConsumerAdapterError> {
        ReadyConsumer.coordinate(target, plan)
    }

    /// Delegates reactivation because this probe exercises a no-mutation projection.
    fn reactivate(
        &self,
        target: &EffectTarget,
        plan: &CoordinationPlan,
    ) -> Result<CoordinationReceipt, ConsumerAdapterError> {
        ReadyConsumer.reactivate(target, plan)
    }

    /// Advances Desired State after the old snapshot is planned but before it is committed.
    fn verify_ready(
        &self,
        target: &EffectTarget,
        projection: &TargetProjection,
    ) -> Result<ReadinessReceipt, ConsumerAdapterError> {
        let mut published = self
            .published
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !*published {
            self.repository
                .replace_plugin_skills(
                    &PluginId::new("official", "review")
                        .unwrap_or_else(|error| panic!("plugin id: {error}")),
                    "1.0.0",
                    std::slice::from_ref(&self.projection),
                    /*updated_at*/ 20,
                )
                .unwrap_or_else(|error| panic!("publish newer Skill generation: {error}"));
            *published = true;
        }
        ReadyConsumer.verify_ready(target, projection)
    }
}

/// Builds an exact coordination receipt or rejects an Uninterrupted Target call as a test bug.
fn coordination_receipt(
    target: &EffectTarget,
    plan: &CoordinationPlan,
    state: CoordinationReceiptState,
) -> Result<CoordinationReceipt, ConsumerAdapterError> {
    let Some(CoordinationRequirement::QuiesceBeforeMutation(contract)) =
        plan.participants.get(&target.identity)
    else {
        return Err(ConsumerAdapterError::new(std::io::Error::other(
            "Uninterrupted Target should not receive a coordination call",
        )));
    };
    Ok(CoordinationReceipt {
        target: target.identity.clone(),
        contract: contract.clone(),
        state,
        proof: AdapterReceipt {
            version: 1,
            payload: serde_json::json!({ "acknowledged": true }),
        },
    })
}

/// Creates one file-backed database whose Workspace insert also seeds an Effect Scope.
fn fixture() -> (TempDir, RepositoryPool, Workspace) {
    let directory = TempDir::new().unwrap_or_else(|error| panic!("create database dir: {error}"));
    let workspace_root = directory.path().join("workspace");
    std::fs::create_dir_all(&workspace_root)
        .unwrap_or_else(|error| panic!("create workspace: {error}"));
    let pool = with_trace_logging(|| {
        DatabaseBootstrapper::new(FixedTimestamp)
            .bootstrap_repository_pool(
                &DatabaseLocation::path(directory.path().join("ora.sqlite")),
                &default_migration_catalog()
                    .unwrap_or_else(|error| panic!("build catalog: {error}")),
            )
            .unwrap_or_else(|error| panic!("bootstrap pool: {error}"))
    });
    let workspace = Workspace::new(
        WorkspaceId::new("workspace-1"),
        ProjectId::new("project-1"),
        WorkspaceKind::Main,
        WorkspaceLocation::local_filesystem(workspace_root.to_string_lossy()),
        WorkspaceLifecycle::Active,
        AuditFields::new(1, 1, false),
    );
    pool.with_connection(|connection| {
        connection.execute(
            "INSERT INTO projects (
                 id, name, repository_kind, created_at, updated_at, is_deleted
             ) VALUES ('project-1', 'Demo', 'git', 1, 1, 0)",
            [],
        )?;
        connection.execute(
            "INSERT INTO workspace_locations (
                 id, location_kind, locator_version, locator_json, created_at, updated_at
             ) VALUES ('location-1', 'local_filesystem', 1, ?1, 1, 1)",
            params![serde_json::json!({ "path": workspace_root }).to_string()],
        )?;
        connection.execute(
            "INSERT INTO workspaces (
                 id, project_id, workspace_kind, location_id, lifecycle,
                 created_at, updated_at, is_deleted
             ) VALUES (?1, 'project-1', 'main', 'location-1', 'active', 1, 1, 0)",
            params![workspace.id.as_ref()],
        )?;
        Ok(())
    })
    .unwrap_or_else(|error| panic!("insert Workspace fixture: {error}"));
    (directory, pool, workspace)
}

/// Computes the immutable package identity before repository publication.
fn package_fingerprint(package_root: &std::path::Path) -> Fingerprint {
    Fingerprint::from(
        ora_utils::directory::fingerprint_directory(package_root, &[])
            .unwrap_or_else(|error| panic!("fingerprint package: {error}")),
    )
}

/// Builds one valid Agent Consumer declaration for a shared Skill directory Resource.
fn declaration(stable_key: &str) -> ConsumerDeclaration {
    let materialization = MaterializationContract::skill_directory_v1();
    ConsumerDeclaration {
        consumer: ConsumerIdentity::new(ConsumerKind::agent_plugin(), stable_key)
            .unwrap_or_else(|error| panic!("consumer identity: {error}")),
        adapter: ConsumerAdapterIdentity::parse("ora/agent-plugin")
            .unwrap_or_else(|error| panic!("consumer adapter: {error}")),
        capabilities: CapabilitySet {
            effect_protocols: BTreeMap::from([(EffectKind::skill(), 1)]),
            materialization_contracts: BTreeSet::from([materialization.capability_key()]),
            coordination_contracts: BTreeSet::new(),
            readiness_contracts: BTreeSet::new(),
        },
        resources: vec![FilesystemResourceTemplate {
            ownership_relative_path: None,
            relative_path: ResourcePath::parse(".agents/skills")
                .unwrap_or_else(|error| panic!("resource path: {error}")),
            materialization_format: MaterializationFormat::skill_directory_v1(),
            materialization_contract: MaterializationContract::skill_directory_v1(),
            accepts: CapabilityRequirement::default(),
            coordination: CoordinationRequirement::Uninterrupted,
        }],
    }
}

#[test]
fn consumer_targets_share_one_resource_without_sharing_target_state() {
    let (_directory, pool, workspace) = fixture();
    let repository = SqliteEffectRepository::new(pool.clone());
    repository
        .declare_consumer(
            &declaration("official/codex"),
            std::slice::from_ref(&workspace),
            LocalTimestamp::from_millis(10),
        )
        .unwrap_or_else(|error| panic!("declare first Consumer: {error}"));
    repository
        .declare_consumer(
            &declaration("official/opencode"),
            std::slice::from_ref(&workspace),
            LocalTimestamp::from_millis(11),
        )
        .unwrap_or_else(|error| panic!("declare second Consumer: {error}"));

    let counts = pool
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT
                         (SELECT COUNT(*) FROM effect_targets),
                         (SELECT COUNT(*) FROM effect_resources),
                         (SELECT COUNT(*) FROM effect_target_resource_bindings),
                         (SELECT COUNT(*) FROM effect_reconcile_requests)",
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
        .unwrap_or_else(|error| panic!("load Effect counts: {error}"));
    assert_eq!(counts, (2, 1, 2, 2));
}

#[test]
fn unchanged_consumer_declaration_does_not_requeue_its_target() {
    let (_directory, pool, workspace) = fixture();
    let repository = SqliteEffectRepository::new(pool.clone());
    let consumer = declaration("official/codex");
    repository
        .declare_consumer(
            &consumer,
            std::slice::from_ref(&workspace),
            LocalTimestamp::from_millis(10),
        )
        .unwrap_or_else(|error| panic!("declare Consumer: {error}"));
    let before = pool
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT status.status_version, status.updated_at, request.requested_at,
                            request.updated_at
                     FROM effect_target_status status
                     JOIN effect_reconcile_requests request ON request.target_id = status.target_id",
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
        .unwrap_or_else(|error| panic!("load Target state: {error}"));

    repository
        .declare_consumer(&consumer, &[workspace], LocalTimestamp::from_millis(20))
        .unwrap_or_else(|error| panic!("redeclare Consumer: {error}"));
    let after = pool
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT status.status_version, status.updated_at, request.requested_at,
                            request.updated_at
                     FROM effect_target_status status
                     JOIN effect_reconcile_requests request ON request.target_id = status.target_id",
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
        .unwrap_or_else(|error| panic!("reload Target state: {error}"));

    assert_eq!(after, before);
}

#[test]
fn newer_wakeup_during_projection_commit_preserves_request_timestamp() {
    let (directory, pool, workspace) = fixture();
    let repository = SqliteEffectRepository::new(pool.clone());
    repository
        .declare_consumer(
            &declaration("official/codex"),
            &[workspace],
            LocalTimestamp::from_millis(10),
        )
        .unwrap_or_else(|error| panic!("declare Consumer: {error}"));
    let worker = WorkerIdentity::parse("worker-1")
        .unwrap_or_else(|error| panic!("worker identity: {error}"));
    let (target, claim) = repository
        .claim_due_targets(
            &worker,
            LocalTimestamp::from_millis(11),
            LocalTimestamp::from_millis(100),
            /*limit*/ 1,
        )
        .unwrap_or_else(|error| panic!("claim Target: {error}"))
        .remove(0);
    let package_root = directory.path().join("plugin-skill");
    std::fs::create_dir_all(&package_root)
        .unwrap_or_else(|error| panic!("create package: {error}"));
    let manifest = b"---\nname: review\ndescription: Reviews changes\n---\n";
    std::fs::write(package_root.join("SKILL.md"), manifest)
        .unwrap_or_else(|error| panic!("write package: {error}"));
    let consumer = PublishSkillOnVerify {
        repository: SqliteSkillRepository::new(pool.clone()),
        projection: PluginSkillProjection {
            name: "review".to_string(),
            description: "Reviews changes".to_string(),
            package_fingerprint: package_fingerprint(&package_root),
            package_root,
            skill_md_digest: Digest::sha256(manifest),
        },
        published: Mutex::new(false),
    };

    let outcome = EffectReconciler::new(
        &repository,
        &SkillPlanner,
        &consumer,
        &SkillDirectoryResourceAdapter,
    )
    .reconcile(
        &target,
        &claim,
        LocalTimestamp::from_millis(11),
        LocalTimestamp::from_millis(100),
    )
    .unwrap_or_else(|error| panic!("commit older projection: {error:?}"));
    let request = pool
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT state, requested_generation, requested_at, updated_at
                     FROM effect_reconcile_requests WHERE target_id = ?1",
                    params![target.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, i64>(3)?,
                        ))
                    },
                )
                .map_err(Into::into)
        })
        .unwrap_or_else(|error| panic!("load preserved request: {error}"));

    assert_eq!(
        (outcome, request),
        (
            ReconcileOutcome::Current {
                target,
                generation: Generation::default(),
            },
            ("pending".to_string(), 1, 20, 20),
        )
    );
}

#[test]
fn source_publication_changes_each_complete_scope_generation_once() {
    let (directory, pool, workspace) = fixture();
    let package_root = directory.path().join("plugin-skill");
    std::fs::create_dir_all(&package_root)
        .unwrap_or_else(|error| panic!("create package: {error}"));
    std::fs::write(package_root.join("SKILL.md"), b"manifest")
        .unwrap_or_else(|error| panic!("write package: {error}"));
    let repository = SqliteSkillRepository::new(pool.clone());
    let plugin_id =
        PluginId::new("official", "review").unwrap_or_else(|error| panic!("plugin id: {error}"));
    repository
        .replace_plugin_skills(
            &plugin_id,
            "1.0.0",
            &[PluginSkillProjection {
                name: "review".to_string(),
                description: "Reviews changes".to_string(),
                package_fingerprint: package_fingerprint(&package_root),
                package_root,
                skill_md_digest: ora_effect::Digest::sha256(b"manifest"),
            }],
            10,
        )
        .unwrap_or_else(|error| panic!("publish plugin Skill: {error}"));

    let state = SqliteEffectRepository::new(pool)
        .load_desired_state(&ora_effect::EffectScopeId::Workspace(workspace.id))
        .unwrap_or_else(|error| panic!("load Desired State: {error}"));
    assert_eq!(state.generation, ora_effect::Generation::new(1));
    assert_eq!(state.effects.len(), 1);
}

#[test]
fn reconciler_materializes_and_finalizes_one_complete_target_generation() {
    let (directory, pool, workspace) = fixture();
    let package_root = directory.path().join("plugin-skill");
    std::fs::create_dir_all(&package_root)
        .unwrap_or_else(|error| panic!("create package: {error}"));
    let manifest = b"---\nname: review\ndescription: Reviews changes\n---\n";
    std::fs::write(package_root.join("SKILL.md"), manifest)
        .unwrap_or_else(|error| panic!("write package: {error}"));
    SqliteSkillRepository::new(pool.clone())
        .replace_plugin_skills(
            &PluginId::new("official", "review")
                .unwrap_or_else(|error| panic!("plugin id: {error}")),
            "1.0.0",
            &[PluginSkillProjection {
                name: "review".to_string(),
                description: "Reviews changes".to_string(),
                package_fingerprint: package_fingerprint(&package_root),
                package_root,
                skill_md_digest: Digest::sha256(manifest),
            }],
            10,
        )
        .unwrap_or_else(|error| panic!("publish plugin Skill: {error}"));
    let repository = SqliteEffectRepository::new(pool.clone());
    repository
        .declare_consumer(
            &declaration("official/codex"),
            std::slice::from_ref(&workspace),
            LocalTimestamp::from_millis(11),
        )
        .unwrap_or_else(|error| panic!("declare Consumer: {error}"));
    let worker = WorkerIdentity::parse("worker-1")
        .unwrap_or_else(|error| panic!("worker identity: {error}"));
    let (target, claim) = repository
        .claim_due_targets(
            &worker,
            LocalTimestamp::from_millis(12),
            LocalTimestamp::from_millis(100),
            1,
        )
        .unwrap_or_else(|error| panic!("claim Target: {error}"))
        .remove(0);
    let planner = SkillPlanner;
    let filesystem = SkillDirectoryResourceAdapter;
    let outcome = EffectReconciler::new(&repository, &planner, &ReadyConsumer, &filesystem)
        .reconcile(
            &target,
            &claim,
            LocalTimestamp::from_millis(13),
            LocalTimestamp::from_millis(100),
        )
        .unwrap_or_else(|error| panic!("reconcile Target: {error}"));
    let persisted = pool
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT status.phase, status.ready_generation,
                            (SELECT COUNT(*) FROM effect_managed_items),
                            (SELECT COUNT(*) FROM effect_operations WHERE phase = 'finalized'),
                            (SELECT COUNT(*) FROM effect_operation_artifacts)
                     FROM effect_target_status status WHERE status.target_id = ?1",
                    params![target.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, i64>(3)?,
                            row.get::<_, i64>(4)?,
                        ))
                    },
                )
                .map_err(Into::into)
        })
        .unwrap_or_else(|error| panic!("load finalized Effect state: {error}"));
    let materialized_manifest = directory
        .path()
        .join("workspace")
        .join(".agents")
        .join("skills")
        .join("review")
        .join("SKILL.md");

    assert_eq!(
        outcome,
        ReconcileOutcome::Mutated {
            target: target.clone(),
            generation: ora_effect::Generation::new(1),
            operations: 1,
        }
    );
    assert_eq!(persisted, ("current".to_string(), 1, 1, 1, 0));
    assert_eq!(
        std::fs::read(materialized_manifest)
            .unwrap_or_else(|error| panic!("read materialized Skill: {error}")),
        manifest
    );

    assert_eq!(
        repository
            .request_reconcile(&target, LocalTimestamp::from_millis(14))
            .unwrap_or_else(|error| panic!("request idempotent reconcile: {error}")),
        true
    );
    let (_, second_claim) = repository
        .claim_due_targets(
            &worker,
            LocalTimestamp::from_millis(14),
            LocalTimestamp::from_millis(110),
            1,
        )
        .unwrap_or_else(|error| panic!("reclaim Target: {error}"))
        .remove(0);
    let replay = EffectReconciler::new(&repository, &planner, &ReadyConsumer, &filesystem)
        .reconcile(
            &target,
            &second_claim,
            LocalTimestamp::from_millis(15),
            LocalTimestamp::from_millis(110),
        )
        .unwrap_or_else(|error| panic!("reconcile current Target: {error}"));
    let operation_count = pool
        .with_connection(|connection| {
            connection
                .query_row("SELECT COUNT(*) FROM effect_operations", [], |row| {
                    row.get::<_, i64>(0)
                })
                .map_err(Into::into)
        })
        .unwrap_or_else(|error| panic!("count replayed Operations: {error}"));

    assert_eq!(
        (replay, operation_count),
        (
            ReconcileOutcome::Current {
                target,
                generation: ora_effect::Generation::new(1),
            },
            1,
        )
    );
}

#[test]
fn desired_replacement_uses_generation_cas_and_exact_no_op_semantics() {
    let (directory, pool, workspace) = fixture();
    let package_root = directory.path().join("plugin-skill");
    std::fs::create_dir_all(&package_root)
        .unwrap_or_else(|error| panic!("create package: {error}"));
    std::fs::write(package_root.join("SKILL.md"), b"manifest")
        .unwrap_or_else(|error| panic!("write package: {error}"));
    SqliteSkillRepository::new(pool.clone())
        .replace_plugin_skills(
            &PluginId::new("official", "review")
                .unwrap_or_else(|error| panic!("plugin id: {error}")),
            "1.0.0",
            &[PluginSkillProjection {
                name: "review".to_string(),
                description: "Reviews changes".to_string(),
                package_fingerprint: package_fingerprint(&package_root),
                package_root,
                skill_md_digest: ora_effect::Digest::sha256(b"manifest"),
            }],
            10,
        )
        .unwrap_or_else(|error| panic!("publish plugin Skill: {error}"));
    let repository = SqliteEffectRepository::new(pool);
    let scope = ora_effect::EffectScopeId::Workspace(workspace.id);
    let current = repository
        .load_desired_state(&scope)
        .unwrap_or_else(|error| panic!("load Desired State: {error}"));

    assert_eq!(
        repository
            .replace_desired_state(
                &scope,
                current.generation,
                current.effects.values().cloned().collect(),
                LocalTimestamp::from_millis(20),
            )
            .unwrap_or_else(|error| panic!("replace no-op: {error}")),
        ReplaceDesiredStateOutcome::Unchanged(current.clone())
    );
    assert_eq!(
        repository
            .replace_desired_state(
                &scope,
                ora_effect::Generation::default(),
                Vec::new(),
                LocalTimestamp::from_millis(21),
            )
            .unwrap_or_else(|error| panic!("replace conflict: {error}")),
        ReplaceDesiredStateOutcome::Conflict {
            expected_generation: ora_effect::Generation::default(),
            current_generation: current.generation,
        }
    );
}

#[test]
fn target_fencing_remains_monotonic_after_request_row_recreation() {
    let (_directory, pool, workspace) = fixture();
    let repository = SqliteEffectRepository::new(pool.clone());
    repository
        .declare_consumer(
            &declaration("official/codex"),
            &[workspace],
            LocalTimestamp::from_millis(10),
        )
        .unwrap_or_else(|error| panic!("declare Consumer: {error}"));
    let worker = WorkerIdentity::parse("worker-1")
        .unwrap_or_else(|error| panic!("worker identity: {error}"));
    let first = repository
        .claim_due_targets(
            &worker,
            LocalTimestamp::from_millis(10),
            LocalTimestamp::from_millis(100),
            1,
        )
        .unwrap_or_else(|error| panic!("claim first request: {error}"))
        .remove(0);
    pool.with_connection(|connection| {
        connection.execute(
            "DELETE FROM effect_reconcile_requests WHERE target_id = ?1",
            params![first.0.as_str()],
        )?;
        connection.execute(
            "INSERT INTO effect_reconcile_requests (
                 target_id, requested_generation, state, wake_reasons_json,
                 requested_at, updated_at
             ) VALUES (?1, 0, 'pending', '[]', 20, 20)",
            params![first.0.as_str()],
        )?;
        Ok(())
    })
    .unwrap_or_else(|error| panic!("recreate request: {error}"));
    let second = repository
        .claim_due_targets(
            &worker,
            LocalTimestamp::from_millis(20),
            LocalTimestamp::from_millis(110),
            1,
        )
        .unwrap_or_else(|error| panic!("claim recreated request: {error}"))
        .remove(0);

    assert_eq!(first.1.token.value(), 1);
    assert_eq!(second.1.token.value(), 2);
}

#[test]
fn transient_failures_keep_a_counted_durable_retry_schedule() {
    let (_directory, pool, workspace) = fixture();
    let repository = SqliteEffectRepository::new(pool.clone());
    repository
        .declare_consumer(
            &declaration("official/codex"),
            &[workspace],
            LocalTimestamp::from_millis(10),
        )
        .unwrap_or_else(|error| panic!("declare Consumer: {error}"));
    let worker = WorkerIdentity::parse("worker-1")
        .unwrap_or_else(|error| panic!("worker identity: {error}"));
    let (target, first_claim) = repository
        .claim_due_targets(
            &worker,
            LocalTimestamp::from_millis(10),
            LocalTimestamp::from_millis(100),
            1,
        )
        .unwrap_or_else(|error| panic!("claim first request: {error}"))
        .remove(0);
    let first_retry = repository
        .schedule_retry(
            &target,
            &first_claim,
            LocalTimestamp::from_millis(20),
            LocalTimestamp::from_millis(11),
        )
        .unwrap_or_else(|error| panic!("schedule first retry: {error}"));
    let (_, second_claim) = repository
        .claim_due_targets(
            &worker,
            LocalTimestamp::from_millis(20),
            LocalTimestamp::from_millis(110),
            1,
        )
        .unwrap_or_else(|error| panic!("claim second request: {error}"))
        .remove(0);
    let second_retry = repository
        .schedule_retry(
            &target,
            &second_claim,
            LocalTimestamp::from_millis(30),
            LocalTimestamp::from_millis(21),
        )
        .unwrap_or_else(|error| panic!("schedule second retry: {error}"));
    let stored = pool
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT state, retry_count, retry_attempt, not_before,
                            claim_token, claim_worker, lease_until
                     FROM effect_reconcile_requests WHERE target_id = ?1",
                    params![target.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, Option<i64>>(2)?,
                            row.get::<_, Option<i64>>(3)?,
                            row.get::<_, Option<i64>>(4)?,
                            row.get::<_, Option<String>>(5)?,
                            row.get::<_, Option<i64>>(6)?,
                        ))
                    },
                )
                .map_err(Into::into)
        })
        .unwrap_or_else(|error| panic!("load retry request: {error}"));

    assert_eq!(first_retry.map(|attempt| attempt.value()), Some(1));
    assert_eq!(second_retry.map(|attempt| attempt.value()), Some(2));
    assert_eq!(
        stored,
        (
            "retry_scheduled".to_string(),
            2,
            Some(2),
            Some(30),
            None,
            None,
            None,
        )
    );
}

#[test]
fn unfinished_operation_is_quarantined_with_target_and_resource_conditions() {
    let (directory, pool, workspace) = fixture();
    let repository = SqliteEffectRepository::new(pool.clone());
    repository
        .declare_consumer(
            &declaration("official/codex"),
            &[workspace],
            LocalTimestamp::from_millis(10),
        )
        .unwrap_or_else(|error| panic!("declare Consumer: {error}"));
    let worker = WorkerIdentity::parse("worker-1")
        .unwrap_or_else(|error| panic!("worker identity: {error}"));
    let (target, _claim) = repository
        .claim_due_targets(
            &worker,
            LocalTimestamp::from_millis(10),
            LocalTimestamp::from_millis(100),
            1,
        )
        .unwrap_or_else(|error| panic!("claim request: {error}"))
        .remove(0);
    let (consumer_revision, resource) = pool
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT target.consumer_revision_id, binding.resource_id
                     FROM effect_targets target
                     JOIN effect_target_resource_bindings binding ON binding.target_id = target.id
                     WHERE target.id = ?1",
                    params![target.as_str()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .map_err(Into::into)
        })
        .unwrap_or_else(|error| panic!("load Target journal identities: {error}"));
    let target_projection = Digest::sha256(b"target-projection").to_string();
    let resource_projection = Digest::sha256(b"resource-projection").to_string();
    let target_identity = EffectTargetId::new(target.as_str());
    let resource_identity = EffectResourceId::new(&resource);
    let coordination = CoordinationPlan::new(
        BTreeSet::from([resource_identity]),
        BTreeMap::from([(target_identity, CoordinationRequirement::Uninterrupted)]),
    )
    .unwrap_or_else(|error| panic!("build coordination plan: {error}"));
    let workspace_root = directory.path().join("workspace");
    let resource_root = workspace_root.join(".agents").join("skills");
    let payload = VersionedAdapterPlan::FilesystemDirectoryV1(FilesystemOperationPlan {
        workspace_root: workspace_root.clone(),
        resource_relative_path: ResourcePath::parse(".agents/skills")
            .unwrap_or_else(|error| panic!("resource path: {error}")),
        resource_root,
        source_root: None,
        staging_path: workspace_root.join(".agents").join("staging"),
        backup_path: workspace_root.join(".agents").join("backup"),
    });
    let planned = ExactPlannedState::Present {
        native_identity: NativeResourceIdentity::parse("review")
            .unwrap_or_else(|error| panic!("native identity: {error}")),
        fingerprint: Fingerprint::sha256(b"planned"),
        managed_identity: ManagedIdentity::new("managed-1"),
    };
    pool.with_connection(|connection| {
        connection.execute(
            "INSERT INTO effect_target_projections (
                 target_id, generation, consumer_revision_id, digest, created_at
             ) VALUES (?1, 0, ?2, ?3, 11)",
            params![target.as_str(), &consumer_revision, &target_projection],
        )?;
        connection.execute(
            "INSERT INTO effect_resource_projections (
                 resource_id, generation, digest, created_at
             ) VALUES (?1, 0, ?2, 11)",
            params![&resource, &resource_projection],
        )?;
        connection.execute(
            "INSERT INTO effect_reconcile_attempts (
                 id, target_id, generation, consumer_revision_id, target_projection_digest,
                 coordination_plan_version, coordination_plan_json, phase, prepared_at, updated_at
             ) VALUES ('attempt-1', ?1, 0, ?2, ?3, 1, ?4, 'prepared', 11, 11)",
            params![
                target.as_str(),
                &consumer_revision,
                &target_projection,
                serde_json::to_string(&coordination)
                    .unwrap_or_else(|error| panic!("serialize coordination: {error}")),
            ],
        )?;
        connection.execute(
            "INSERT INTO effect_attempt_resource_projections (
                 attempt_id, resource_projection_digest, sequence
             ) VALUES ('attempt-1', ?1, 0)",
            params![&resource_projection],
        )?;
        connection.execute(
            "INSERT INTO effect_operations (
                 id, attempt_id, resource_id, generation, sequence, mutation,
                 expected_version, expected_json, planned_version, planned_json,
                 payload_version, payload_json, phase, prepared_at, updated_at
             ) VALUES ('operation-1', 'attempt-1', ?1, 0, 0, 'create',
                       1, ?2, 1, ?3, 1, ?4, 'prepared', 11, 11)",
            params![
                &resource,
                serde_json::to_string(&ExactPreviousState::Missing)
                    .unwrap_or_else(|error| panic!("serialize expected state: {error}")),
                serde_json::to_string(&planned)
                    .unwrap_or_else(|error| panic!("serialize planned state: {error}")),
                serde_json::to_string(&payload)
                    .unwrap_or_else(|error| panic!("serialize operation payload: {error}")),
            ],
        )?;
        Ok(())
    })
    .unwrap_or_else(|error| panic!("insert unfinished journal: {error}"));

    let active_lease_quarantined = repository
        .quarantine_unfinished_operations(LocalTimestamp::from_millis(20))
        .unwrap_or_else(|error| panic!("check active journal: {error}"));
    let quarantined = repository
        .quarantine_unfinished_operations(LocalTimestamp::from_millis(101))
        .unwrap_or_else(|error| panic!("quarantine journal: {error}"));
    let recovery_wakeup_recorded = repository
        .request_reconcile(&target, LocalTimestamp::from_millis(102))
        .unwrap_or_else(|error| panic!("record recovery wakeup: {error}"));
    let stored = pool
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT
                         (SELECT phase FROM effect_operations WHERE id = 'operation-1'),
                         (SELECT phase FROM effect_reconcile_attempts WHERE id = 'attempt-1'),
                         (SELECT phase FROM effect_target_status WHERE target_id = ?1),
                         (SELECT phase FROM effect_resource_status WHERE resource_id = ?2),
                         (SELECT state FROM effect_reconcile_requests WHERE target_id = ?1),
                         (SELECT COUNT(*) FROM effect_conditions
                          WHERE owner_kind = 'target' AND owner_id = ?1),
                         (SELECT COUNT(*) FROM effect_conditions
                          WHERE owner_kind = 'resource' AND owner_id = ?2)",
                    params![target.as_str(), &resource],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, i64>(5)?,
                            row.get::<_, i64>(6)?,
                        ))
                    },
                )
                .map_err(Into::into)
        })
        .unwrap_or_else(|error| panic!("load recovery state: {error}"));

    assert_eq!(
        (
            active_lease_quarantined,
            quarantined,
            recovery_wakeup_recorded
        ),
        (0, 1, true)
    );
    assert_eq!(
        stored,
        (
            "recovery_required".to_string(),
            "recovery_required".to_string(),
            "recovery_required".to_string(),
            "recovery_required".to_string(),
            "blocked".to_string(),
            1,
            1,
        )
    );
}
