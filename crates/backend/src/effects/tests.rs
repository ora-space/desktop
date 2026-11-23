use super::Effects;
use ora_contracts::{
    EmptyErrorParams, GetEffectTargetStatusRequest, GetEffectTargetStatusResponse, PublicError,
    RecoverEffectTargetRequest, RetryEffectTargetRequest, RetryEffectTargetResponse,
};
use ora_db::{DatabaseBootstrapper, DatabaseLocation, default_migration_catalog};
use pretty_assertions::assert_eq;

/// Missing targets are an empty status, while actual SQLite read failures remain public errors;
/// neither path needs a plugin host, worker, or supervisor to exercise the consumer interface.
#[test]
fn missing_targets_and_storage_failures_stay_distinct_without_runtime_composition() {
    ora_logging::with_trace_logging(|| {
        let temporary = tempfile::tempdir().expect("Effect status fixture");
        let path = temporary.path().join("ora.sqlite3");
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(&path),
                &default_migration_catalog().expect("catalog"),
            )
            .expect("database");
        let effects = Effects::new(pool, crate::effect_worker::EffectWorkerHandle::detached());
        let requests = [
            GetEffectTargetStatusRequest::Target {
                target_id: "missing-target".to_string(),
            },
            GetEffectTargetStatusRequest::WorkspaceAgent {
                workspace_id: "missing-workspace".to_string(),
                agent_plugin_id: "official/ora-space.fixture".to_string(),
            },
        ];
        for request in &requests {
            assert_eq!(
                effects
                    .target_status(request.clone())
                    .expect("missing status"),
                GetEffectTargetStatusResponse { status: None }
            );
        }
        rusqlite::Connection::open(&path)
            .expect("fault injection connection")
            .execute_batch("DROP TABLE effect_target_status;")
            .expect("inject storage read failure");
        let error = effects
            .target_status(requests[0].clone())
            .expect_err("missing table is not a missing target");
        assert_eq!(
            error.public_error(),
            &PublicError::InternalError(EmptyErrorParams {})
        );
        assert!(std::error::Error::source(&error).is_some());
    });
}

/// Retrying a missing Target is a no-op instead of inventing a request or surfacing storage failure.
#[test]
fn retrying_a_missing_target_reports_no_durable_request() {
    ora_logging::initialize_test_clock();
    ora_logging::with_trace_logging(|| {
        let temporary = tempfile::tempdir().expect("Effect retry fixture");
        let path = temporary.path().join("ora.sqlite3");
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(&path),
                &default_migration_catalog().expect("catalog"),
            )
            .expect("database");
        let effects = Effects::new(pool, crate::effect_worker::EffectWorkerHandle::detached());

        let response = effects
            .retry_target(RetryEffectTargetRequest {
                target_id: "missing-target".to_string(),
            })
            .expect("missing Target retry is a no-op");

        assert_eq!(response, RetryEffectTargetResponse { requested: false });
    });
}

/// Recovery of a missing Target is a typed public refusal, not an internal failure.
#[test]
fn recovering_a_missing_target_maps_to_a_public_not_found_error() {
    ora_logging::initialize_test_clock();
    ora_logging::with_trace_logging(|| {
        let temporary = tempfile::tempdir().expect("Effect recovery fixture");
        let path = temporary.path().join("ora.sqlite3");
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(&path),
                &default_migration_catalog().expect("catalog"),
            )
            .expect("database");
        let effects = Effects::new(pool, crate::effect_worker::EffectWorkerHandle::detached());
        let error = effects
            .recover_target(RecoverEffectTargetRequest {
                target_id: "missing-target".to_string(),
            })
            .expect_err("missing Target is not recoverable");
        assert_eq!(
            error.public_error(),
            &PublicError::EffectTargetNotFound(EmptyErrorParams {})
        );
    });
}

/// Target status projection preserves structured Condition parameters for frontend localization.
#[test]
fn target_status_projects_condition_parameters_for_preserved_conflicts() {
    use ora_db::{PluginSkillProjection, SqliteEffectRepository, SqliteSkillRepository};
    use ora_domain::{
        AuditFields, PluginId, ProjectId, Workspace, WorkspaceId, WorkspaceKind,
        WorkspaceLifecycle, WorkspaceLocation,
    };
    use ora_effect::{
        AdapterReceipt, CapabilityRequirement, CapabilitySet, ConsumerAdapter,
        ConsumerAdapterError, ConsumerAdapterIdentity, ConsumerDeclaration, ConsumerIdentity,
        ConsumerKind, CoordinationPlan, CoordinationReceipt, CoordinationRequirement, Digest,
        EffectKind, EffectReconciler, EffectRepository, EffectScopeId, EffectTarget,
        FilesystemResourceTemplate, Fingerprint, LocalTimestamp, MaterializationContract,
        MaterializationFormat, ReadinessReceipt, ReconcileOutcome, ResourcePath, TargetProjection,
        WorkerIdentity,
    };
    use ora_effect_skill::{SkillDirectoryResourceAdapter, SkillPlanner};
    use std::collections::{BTreeMap, BTreeSet};

    #[derive(Clone, Copy)]
    struct FixedTimestamp;

    impl ora_db::TimestampSource for FixedTimestamp {
        fn current_timestamp_millis(&self) -> i64 {
            1
        }
    }

    #[derive(Clone, Copy)]
    struct NoopConsumer;

    impl ConsumerAdapter for NoopConsumer {
        fn reserve_coordination(&self, _target: &EffectTarget) -> Result<(), ConsumerAdapterError> {
            Ok(())
        }

        fn coordinate(
            &self,
            _target: &EffectTarget,
            _plan: &CoordinationPlan,
        ) -> Result<CoordinationReceipt, ConsumerAdapterError> {
            unreachable!("blocked plan does not coordinate")
        }

        fn reactivate(
            &self,
            _target: &EffectTarget,
            _plan: &CoordinationPlan,
        ) -> Result<CoordinationReceipt, ConsumerAdapterError> {
            unreachable!("blocked plan does not reactivate")
        }

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

    ora_logging::with_trace_logging(|| {
        let temporary = tempfile::tempdir().expect("Effect status fixture");
        let workspace_root = temporary.path().join("workspace");
        for root in [".agents/skills", ".opencode/skills"] {
            let conflicting = workspace_root.join(root).join("foo");
            std::fs::create_dir_all(&conflicting).expect("conflict dir");
            std::fs::write(conflicting.join("notes.txt"), b"user content").expect("user file");
        }
        let package_root = temporary.path().join("package");
        std::fs::create_dir_all(&package_root).expect("package dir");
        let manifest = b"---\nname: foo\ndescription: Demo skill\n---\n";
        std::fs::write(package_root.join("SKILL.md"), manifest).expect("skill manifest");
        let fingerprint =
            ora_utils::directory::fingerprint_directory(&package_root, &[]).expect("fingerprint");
        let projection = PluginSkillProjection {
            name: "foo".to_string(),
            description: "Demo skill".to_string(),
            package_root,
            skill_md_digest: Digest::sha256(manifest),
            package_fingerprint: Fingerprint::from(fingerprint),
        };

        let db_path = temporary.path().join("ora.sqlite3");
        let pool = DatabaseBootstrapper::new(FixedTimestamp)
            .bootstrap_repository_pool(
                &DatabaseLocation::path(&db_path),
                &default_migration_catalog().expect("catalog"),
            )
            .expect("database");
        let workspace = Workspace::new(
            WorkspaceId::new("workspace-1"),
            ProjectId::new("project-1"),
            WorkspaceKind::Main,
            WorkspaceLocation::local_filesystem(workspace_root.to_string_lossy()),
            WorkspaceLifecycle::Active,
            AuditFields::new(1, 1, false),
        );
        let mut connection =
            rusqlite::Connection::open(&db_path).expect("workspace fixture connection");
        let transaction = connection.transaction().expect("workspace fixture tx");
        transaction
            .execute(
                "INSERT INTO projects (
                     id, name, repository_kind, created_at, updated_at, is_deleted
                 ) VALUES ('project-1', 'Demo', 'git', 1, 1, 0)",
                [],
            )
            .expect("insert project");
        transaction
            .execute(
                "INSERT INTO workspace_locations (
                     id, location_kind, locator_version, locator_json, created_at, updated_at
                 ) VALUES ('location-1', 'local_filesystem', 1, ?1, 1, 1)",
                rusqlite::params![serde_json::json!({ "path": workspace_root }).to_string()],
            )
            .expect("insert workspace location");
        transaction
            .execute(
                "INSERT INTO workspaces (
                     id, project_id, workspace_kind, location_id, lifecycle,
                     created_at, updated_at, is_deleted
                 ) VALUES (?1, 'project-1', 'main', 'location-1', 'active', 1, 1, 0)",
                rusqlite::params![workspace.id.as_ref()],
            )
            .expect("insert workspace");
        transaction
            .execute(
                "INSERT INTO effect_scopes (
                     id, scope_kind, workspace_id, lifecycle, generation, created_at, updated_at
                 ) VALUES (?1, 'workspace', ?2, 'active', 0, 1, 1)",
                rusqlite::params![
                    EffectScopeId::Workspace(workspace.id.clone()).storage_key(),
                    workspace.id.as_ref()
                ],
            )
            .expect("insert effect scope");
        transaction.commit().expect("commit workspace fixture");
        drop(connection);

        SqliteSkillRepository::new(pool.clone())
            .replace_plugin_skills(
                &PluginId::new("official", "foo").expect("plugin id"),
                "1.0.0",
                &[projection],
                /*updated_at*/ 1,
            )
            .expect("publish skill");
        let declaration = ConsumerDeclaration {
            consumer: ConsumerIdentity::new(ConsumerKind::agent_plugin(), "official/opencode")
                .expect("consumer"),
            adapter: ConsumerAdapterIdentity::parse("ora/agent-plugin").expect("adapter"),
            capabilities: CapabilitySet {
                effect_protocols: BTreeMap::from([(EffectKind::skill(), 1)]),
                materialization_contracts: BTreeSet::from([
                    MaterializationContract::skill_directory_v1().capability_key(),
                ]),
                coordination_contracts: BTreeSet::new(),
                readiness_contracts: BTreeSet::new(),
            },
            resources: [".agents/skills", ".opencode/skills"]
                .into_iter()
                .map(|root| FilesystemResourceTemplate {
                    ownership_relative_path: None,
                    relative_path: ResourcePath::parse(root).expect("resource path"),
                    materialization_format: MaterializationFormat::skill_directory_v1(),
                    materialization_contract: MaterializationContract::skill_directory_v1(),
                    accepts: CapabilityRequirement::default(),
                    coordination: CoordinationRequirement::Uninterrupted,
                })
                .collect(),
        };
        let repository = SqliteEffectRepository::new(pool.clone());
        repository
            .declare_consumer(&declaration, &[workspace])
            .expect("declare consumer");
        let worker = WorkerIdentity::parse("worker").expect("worker");
        let lease = LocalTimestamp::from_millis(100);
        let (target, claim) = repository
            .claim_due_targets(
                &worker,
                LocalTimestamp::from_millis(11),
                lease,
                /*limit*/ 1,
            )
            .expect("claim")
            .remove(0);
        let reconciler = EffectReconciler::new(
            &repository,
            &SkillPlanner,
            &NoopConsumer,
            &SkillDirectoryResourceAdapter,
            &FixedTimestamp,
        );
        assert!(matches!(
            reconciler
                .reconcile(&target, &claim, lease)
                .expect("reconcile"),
            ReconcileOutcome::Blocked { .. }
        ));

        let effects = Effects::new(pool, crate::effect_worker::EffectWorkerHandle::detached());
        let response = effects
            .target_status(GetEffectTargetStatusRequest::WorkspaceAgent {
                workspace_id: "workspace-1".to_string(),
                agent_plugin_id: "official/opencode".to_string(),
            })
            .expect("load target status");
        let status = response.status.expect("persisted target status");
        let mut relative_paths = status
            .conditions
            .iter()
            .map(|condition| {
                assert_eq!(condition.code, "preserved_item_conflict");
                condition
                    .parameters
                    .get("relativePath")
                    .cloned()
                    .expect("relativePath DTO parameter")
            })
            .collect::<Vec<_>>();
        relative_paths.sort();
        assert_eq!(
            relative_paths,
            [".agents/skills/foo", ".opencode/skills/foo"]
        );
    });
}
