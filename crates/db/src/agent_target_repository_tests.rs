use crate::{
    DatabaseBootstrapper, DatabaseLocation, MigrationCatalog, RepositoryPool,
    SqliteEffectRepository, TimestampSource, default_migration_catalog,
};
use ora_domain::{Namespace, WorkspaceId};
use ora_effect::{
    AgentCapabilityRevision, AgentPluginId, AgentTargetCondition, AgentTargetConditionAttachment,
    AgentTargetConditionReason, AgentTargetConditionSubject, AgentTargetIdentity,
    AgentTargetLifecycle, AgentTargetPhase, AgentTargetReconcileRequest, AgentTargetReconcileState,
    AgentTargetRecord, AgentTargetRepository, AgentTargetStatus, AgentTargetWakeReason,
    ConditionImpact, ConsumerId, EffectRepository, Generation, ManagedIdentity, SkillName,
    SkillSelectionKey, SourceKind, SurfaceKey, WorkspaceEffect, WorkspaceEffectSpec,
    initial_agent_target_status,
};
use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;
use rusqlite::{Connection, params};
use std::collections::BTreeMap;
use tempfile::TempDir;

#[derive(Clone, Copy, Debug)]
struct FixedTimestamp;

impl TimestampSource for FixedTimestamp {
    fn current_timestamp_millis(&self) -> i64 {
        1_700_000_000_000
    }
}

/// Bootstraps a file-backed pool with one Workspace Effect aggregate.
fn fixture() -> (TempDir, RepositoryPool, WorkspaceId) {
    let directory = TempDir::new().unwrap_or_else(|error| panic!("create database dir: {error}"));
    let location = DatabaseLocation::path(directory.path().join("ora.sqlite"));
    let pool = with_trace_logging(|| {
        DatabaseBootstrapper::new(FixedTimestamp)
            .bootstrap_repository_pool(
                &location,
                &default_migration_catalog()
                    .unwrap_or_else(|error| panic!("build catalog: {error}")),
            )
            .unwrap_or_else(|error| panic!("bootstrap pool: {error}"))
    });
    let workspace_id = WorkspaceId::new("workspace-1");
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
             ) VALUES ('location-1', 'local_filesystem', 1, '{}', 1, 1)",
            [],
        )?;
        connection.execute(
            "INSERT INTO workspaces (
                 id, project_id, workspace_kind, location_id, lifecycle,
                 created_at, updated_at, is_deleted
             ) VALUES (?1, 'project-1', 'main', 'location-1', 'active', 1, 1, 0)",
            params![workspace_id.as_ref()],
        )?;
        Ok(())
    })
    .unwrap_or_else(|error| panic!("insert Workspace fixture: {error}"));
    (directory, pool, workspace_id)
}

/// Builds a catalog that stops before Agent Target Expand so upgrade backfill can be tested.
fn catalog_before_agent_targets() -> MigrationCatalog {
    let full = default_migration_catalog().unwrap_or_else(|error| panic!("build catalog: {error}"));
    MigrationCatalog::with_target_versions(
        full.migrations().to_vec(),
        vec!["0001", "0002", "0003", "0004", "0005", "0006"],
    )
    .unwrap_or_else(|error| panic!("prefix catalog: {error}"))
}

/// Inserts two surface reconcile requests that share one Agent Plugin consumer.
fn seed_surface_requests(connection: &Connection, workspace_id: &WorkspaceId) {
    connection
        .execute(
            "INSERT INTO effect_surfaces (
                 id, workspace_id, adapter_kind, locator_key, locator_json, format_kind,
                 lifecycle, created_at, updated_at
             ) VALUES
             ('surface-a', ?1, 'filesystem_skill', '.agents/skills', '{}', 'skill_directory.v1',
              'active', 10, 10),
             ('surface-b', ?1, 'filesystem_skill', '.claude/skills', '{}', 'skill_directory.v1',
              'active', 10, 10)",
            params![workspace_id.as_ref()],
        )
        .unwrap_or_else(|error| panic!("insert surfaces: {error}"));
    connection
        .execute(
            "INSERT INTO effect_surface_consumers (
                 surface_id, consumer_id, coordination_kind, created_at, updated_at
             ) VALUES
             ('surface-a', 'opencode', 'wait_for_idle_and_restart', 10, 10),
             ('surface-b', 'opencode', 'wait_for_idle_and_restart', 11, 11),
             ('surface-b', 'codex', 'wait_for_idle_and_restart', 12, 12)",
            [],
        )
        .unwrap_or_else(|error| panic!("insert consumers: {error}"));
    connection
        .execute(
            "INSERT INTO effect_surface_status (
                 surface_id, desired_generation, observed_generation, applied_generation,
                 phase, status_version, created_at, updated_at
             ) VALUES
             ('surface-a', 5, 4, 3, 'pending', 1, 10, 10),
             ('surface-b', 7, 6, 5, 'pending', 1, 10, 10)",
            [],
        )
        .unwrap_or_else(|error| panic!("insert surface status: {error}"));
    connection
        .execute(
            "INSERT INTO effect_consumer_status (
                 surface_id, consumer_id, ready_generation, phase, status_version,
                 created_at, updated_at
             ) VALUES
             ('surface-a', 'opencode', 2, 'current', 1, 10, 10),
             ('surface-b', 'opencode', 4, 'current', 1, 10, 10),
             ('surface-b', 'codex', 1, 'current', 1, 10, 10)",
            [],
        )
        .unwrap_or_else(|error| panic!("insert consumer status: {error}"));
    connection
        .execute(
            "INSERT INTO effect_reconcile_requests (
                 surface_id, requested_generation, request_token, state, wake_reason,
                 attempt_count, requested_at, not_before_at, updated_at
             ) VALUES
             ('surface-a', 4, 'token-a', 'pending', 'desired_changed', 0, 100, 250, 100),
             ('surface-b', 9, 'token-b', 'pending', 'desired_changed', 0, 110, 200, 110)",
            [],
        )
        .unwrap_or_else(|error| panic!("insert surface requests: {error}"));
    connection
        .execute(
            "INSERT INTO effect_conditions (
                 id, surface_id, consumer_id, subject_kind, subject_id, reason,
                 failed_generation, message, first_observed_at, last_observed_at
             ) VALUES
             ('cond-surface', 'surface-a', NULL, 'surface',
              '{\"kind\":\"surface\",\"surface_key\":\"surface-a\"}',
              'path_unsafe', 3, 'unsafe path', 10, 20),
             ('cond-desired', 'surface-a', NULL, 'desired_item',
              '{\"kind\":\"desired_skill\",\"selection_key\":{\"source_kind\":\"plugin\",\"namespace\":\"ora\",\"name\":\"demo\"}}',
              'desired_collision', 4, 'collision', 11, 21),
             ('cond-consumer', 'surface-b', 'opencode', 'consumer',
              '{\"kind\":\"consumer\",\"consumer_id\":\"opencode\"}',
              'waiting_for_idle', 5, 'waiting', 30, 40),
             ('cond-managed', 'surface-b', 'opencode', 'managed_item',
              '{\"kind\":\"managed_skill\",\"managed_identity\":\"managed-1\"}',
              'ownership_conflict', 6, 'owned', 31, 41)",
            [],
        )
        .unwrap_or_else(|error| panic!("insert conditions: {error}"));
}

#[test]
fn empty_database_bootstraps_agent_target_schema() {
    let catalog = default_migration_catalog().expect("build migration catalog");
    assert_eq!(
        catalog.target_versions(),
        ["0001", "0002", "0003", "0004", "0005", "0006", "0007"]
    );

    let database = with_trace_logging(|| {
        DatabaseBootstrapper::new(FixedTimestamp)
            .bootstrap(&DatabaseLocation::in_memory(), &catalog)
            .expect("bootstrap database")
    });

    let tables = load_table_names(database.connection());
    assert!(tables.iter().any(|name| name == "effect_agent_targets"));
    assert!(
        tables
            .iter()
            .any(|name| name == "effect_agent_target_status")
    );
    assert!(
        tables
            .iter()
            .any(|name| name == "effect_agent_target_reconcile_requests")
    );
    assert!(
        tables
            .iter()
            .any(|name| name == "effect_agent_target_conditions")
    );
    assert!(
        tables
            .iter()
            .any(|name| name == "effect_reconcile_requests")
    );
}

#[test]
fn upgrades_old_schema_and_backfills_target_requests_deterministically() {
    let directory = TempDir::new().unwrap_or_else(|error| panic!("create database dir: {error}"));
    let location = DatabaseLocation::path(directory.path().join("ora.sqlite"));
    let prefix = catalog_before_agent_targets();
    let database = with_trace_logging(|| {
        DatabaseBootstrapper::new(FixedTimestamp)
            .bootstrap(&location, &prefix)
            .unwrap_or_else(|error| panic!("bootstrap prefix: {error}"))
    });
    let workspace_id = WorkspaceId::new("workspace-1");
    database
        .connection()
        .execute(
            "INSERT INTO projects (
                 id, name, repository_kind, created_at, updated_at, is_deleted
             ) VALUES ('project-1', 'Demo', 'git', 1, 1, 0)",
            [],
        )
        .unwrap();
    database
        .connection()
        .execute(
            "INSERT INTO workspace_locations (
                 id, location_kind, locator_version, locator_json, created_at, updated_at
             ) VALUES ('location-1', 'local_filesystem', 1, '{}', 1, 1)",
            [],
        )
        .unwrap();
    database
        .connection()
        .execute(
            "INSERT INTO workspaces (
                 id, project_id, workspace_kind, location_id, lifecycle,
                 created_at, updated_at, is_deleted
             ) VALUES (?1, 'project-1', 'main', 'location-1', 'active', 1, 1, 0)",
            params![workspace_id.as_ref()],
        )
        .unwrap();
    seed_surface_requests(database.connection(), &workspace_id);
    drop(database);

    let full = default_migration_catalog().expect("full catalog");
    let upgraded = with_trace_logging(|| {
        DatabaseBootstrapper::new(FixedTimestamp)
            .bootstrap(&location, &full)
            .unwrap_or_else(|error| panic!("upgrade to 0007: {error}"))
    });

    let surface_request_count: i64 = upgraded
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM effect_reconcile_requests",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(surface_request_count, 2);

    let target_count: i64 = upgraded
        .connection()
        .query_row("SELECT COUNT(*) FROM effect_agent_targets", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(target_count, 2);

    let opencode = upgraded
        .connection()
        .query_row(
            "SELECT requests.requested_generation, requests.not_before_at, requests.requested_at,
                    status.desired_generation, status.ready_generation, status.phase
             FROM effect_agent_targets targets
             JOIN effect_agent_target_reconcile_requests requests
               ON requests.agent_target_id = targets.id
             JOIN effect_agent_target_status status
               ON status.agent_target_id = targets.id
             WHERE targets.workspace_id = ?1 AND targets.agent_plugin_id = 'opencode'",
            params![workspace_id.as_ref()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .unwrap_or_else(|error| panic!("load opencode backfill: {error}"));
    assert_eq!(opencode, (9, 200, 100, 7, 4, "pending".to_string()));

    let condition_count: i64 = upgraded
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM effect_agent_target_conditions
             WHERE impact = 'blocking'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    // surface-scoped fans out to opencode on surface-a (surface + desired_item);
    // consumer-scoped keeps opencode on surface-b (consumer + managed_item).
    assert_eq!(condition_count, 4);
    drop(upgraded);

    let pool = RepositoryPool::new(&location)
        .unwrap_or_else(|error| panic!("open upgraded pool: {error}"));
    let repository = SqliteEffectRepository::new(pool);
    let identity = AgentTargetIdentity::new(workspace_id.clone(), AgentPluginId::new("opencode"));
    let status = repository
        .load_agent_target_status(&identity)
        .unwrap_or_else(|error| panic!("load backfilled status: {error}"))
        .expect("opencode target status");
    let mut actual = status;
    actual
        .conditions
        .sort_by_key(|condition| format!("{:?}", condition.subject));
    for condition in &mut actual.conditions {
        condition.id.clear();
    }
    let surface_a = AgentTargetConditionAttachment {
        surface_key: SurfaceKey::new("surface-a"),
        consumer_id: Some(ConsumerId::new("opencode")),
    };
    let surface_b = AgentTargetConditionAttachment {
        surface_key: SurfaceKey::new("surface-b"),
        consumer_id: Some(ConsumerId::new("opencode")),
    };
    let mut expected = AgentTargetStatus {
        agent_target_id: actual.agent_target_id.clone(),
        identity: identity.clone(),
        desired_generation: Generation::new(7),
        observed_generation: Generation::new(6),
        applied_generation: Generation::new(5),
        ready_generation: Generation::new(4),
        phase: AgentTargetPhase::Pending,
        status_version: 1,
        created_at: 10,
        updated_at: 11,
        conditions: vec![
            AgentTargetCondition {
                id: String::new(),
                subject: AgentTargetConditionSubject::Consumer {
                    consumer_id: ConsumerId::new("opencode"),
                },
                reason: AgentTargetConditionReason::WaitingForIdle,
                impact: ConditionImpact::Blocking,
                message: "waiting".to_string(),
                first_observed_at: 30,
                last_observed_at: 40,
                failed_generation: Some(Generation::new(5)),
                attachment: Some(surface_b.clone()),
            },
            AgentTargetCondition {
                id: String::new(),
                subject: AgentTargetConditionSubject::DesiredSkill {
                    selection_key: SkillSelectionKey::new(
                        SourceKind::Plugin,
                        Namespace::new("ora").expect("namespace"),
                        SkillName::parse("demo").expect("skill name"),
                    ),
                },
                reason: AgentTargetConditionReason::DesiredCollision,
                impact: ConditionImpact::Blocking,
                message: "collision".to_string(),
                first_observed_at: 11,
                last_observed_at: 21,
                failed_generation: Some(Generation::new(4)),
                attachment: Some(surface_a.clone()),
            },
            AgentTargetCondition {
                id: String::new(),
                subject: AgentTargetConditionSubject::ManagedSkill {
                    managed_identity: ManagedIdentity::new("managed-1"),
                },
                reason: AgentTargetConditionReason::OwnershipConflict,
                impact: ConditionImpact::Blocking,
                message: "owned".to_string(),
                first_observed_at: 31,
                last_observed_at: 41,
                failed_generation: Some(Generation::new(6)),
                attachment: Some(surface_b),
            },
            AgentTargetCondition {
                id: String::new(),
                subject: AgentTargetConditionSubject::Surface {
                    surface_key: SurfaceKey::new("surface-a"),
                },
                reason: AgentTargetConditionReason::PathUnsafe,
                impact: ConditionImpact::Blocking,
                message: "unsafe path".to_string(),
                first_observed_at: 10,
                last_observed_at: 20,
                failed_generation: Some(Generation::new(3)),
                attachment: Some(surface_a),
            },
        ],
    };
    expected
        .conditions
        .sort_by_key(|condition| format!("{:?}", condition.subject));
    assert_eq!(actual, expected);
}

#[test]
fn agent_target_uniqueness_and_generation_constraints() {
    let (_directory, pool, workspace_id) = fixture();
    pool.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO effect_agent_targets (
                     id, workspace_id, agent_plugin_id, capability_revision, lifecycle,
                     created_at, updated_at
                 ) VALUES ('target-1', ?1, 'opencode', '', 'active', 1, 1)",
                params![workspace_id.as_ref()],
            )
            .unwrap();
        let duplicate = connection.execute(
            "INSERT INTO effect_agent_targets (
                 id, workspace_id, agent_plugin_id, capability_revision, lifecycle,
                 created_at, updated_at
             ) VALUES ('target-2', ?1, 'opencode', '', 'active', 1, 1)",
            params![workspace_id.as_ref()],
        );
        assert!(duplicate.is_err(), "duplicate Agent Target must fail");

        connection
            .execute(
                "INSERT INTO effect_agent_target_status (
                     agent_target_id, desired_generation, observed_generation,
                     applied_generation, ready_generation, phase, status_version,
                     created_at, updated_at
                 ) VALUES ('target-1', 1, 1, 1, 1, 'current', 1, 1, 1)",
                [],
            )
            .unwrap();
        let illegal_order = connection.execute(
            "UPDATE effect_agent_target_status
             SET ready_generation = 2, applied_generation = 1
             WHERE agent_target_id = 'target-1'",
            [],
        );
        assert!(illegal_order.is_err(), "ready > applied must fail");

        let illegal_phase = connection.execute(
            "UPDATE effect_agent_target_status SET phase = 'unknown' WHERE agent_target_id = 'target-1'",
            [],
        );
        assert!(illegal_phase.is_err(), "illegal phase must fail");

        let illegal_impact = connection.execute(
            "INSERT INTO effect_agent_target_conditions (
                 id, agent_target_id, surface_id, consumer_id, subject_kind, subject_id, reason,
                 impact, failed_generation, message, first_observed_at, last_observed_at
             ) VALUES ('c1', 'target-1', NULL, NULL, 'agent_target', '{}', 'path_unsafe',
                       'maybe', NULL, 'bad', 1, 1)",
            [],
        );
        assert!(illegal_impact.is_err(), "illegal impact must fail");

        let illegal_ownership = connection.execute(
            "INSERT INTO effect_agent_target_conditions (
                 id, agent_target_id, surface_id, consumer_id, subject_kind, subject_id, reason,
                 impact, failed_generation, message, first_observed_at, last_observed_at
             ) VALUES ('c2', 'target-1', NULL, 'opencode', 'consumer', '{}', 'waiting_for_idle',
                       'blocking', NULL, 'bad ownership', 1, 1)",
            [],
        );
        assert!(
            illegal_ownership.is_err(),
            "consumer without surface must fail"
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn typed_repository_round_trips_complete_agent_target_record() {
    let (_directory, pool, workspace_id) = fixture();
    let repository = SqliteEffectRepository::new(pool);
    let identity = AgentTargetIdentity::new(workspace_id.clone(), AgentPluginId::new("opencode"));
    let target = repository
        .upsert_agent_target(
            &identity,
            &AgentCapabilityRevision::new("cap-1"),
            AgentTargetLifecycle::Active,
            /*updated_at*/ 50,
        )
        .unwrap_or_else(|error| panic!("upsert target: {error}"));

    let mut status =
        initial_agent_target_status(target.id.clone(), identity.clone(), /*now*/ 50);
    status.desired_generation = Generation::new(4);
    status.observed_generation = Generation::new(3);
    status.applied_generation = Generation::new(2);
    status.ready_generation = Generation::new(1);
    status.phase = AgentTargetPhase::ReadyWithIssues;
    status.status_version = 2;
    status.updated_at = 60;
    status.conditions = vec![AgentTargetCondition {
        id: "condition-1".to_string(),
        subject: AgentTargetConditionSubject::Mcp {
            managed_identity: ManagedIdentity::new("mcp-1"),
        },
        reason: AgentTargetConditionReason::UnsupportedByAgent,
        impact: ConditionImpact::NonBlocking,
        message: "stdio unsupported".to_string(),
        first_observed_at: 55,
        last_observed_at: 60,
        failed_generation: Some(Generation::new(4)),
        attachment: None,
    }];
    repository
        .save_agent_target_status(&status)
        .unwrap_or_else(|error| panic!("save status: {error}"));

    let request = repository
        .upsert_agent_target_reconcile_request(
            &identity,
            Generation::new(4),
            AgentTargetWakeReason::DesiredChanged,
            /*not_before_at*/ 80,
            /*updated_at*/ 70,
        )
        .unwrap_or_else(|error| panic!("upsert request: {error}"));
    let expected_request = AgentTargetReconcileRequest {
        agent_target_id: target.id.clone(),
        identity: identity.clone(),
        requested_generation: Generation::new(6),
        request_token: request.request_token.clone(),
        state: AgentTargetReconcileState::Pending,
        wake_reason: AgentTargetWakeReason::CapabilityChanged,
        attempt_count: 0,
        requested_at: 65,
        not_before_at: 65,
        updated_at: 90,
    };
    let coalesced = repository
        .upsert_agent_target_reconcile_request(
            &identity,
            Generation::new(6),
            AgentTargetWakeReason::CapabilityChanged,
            /*not_before_at*/ 65,
            /*updated_at*/ 90,
        )
        .unwrap_or_else(|error| panic!("coalesce request: {error}"));
    assert_eq!(coalesced, expected_request);

    let loaded = repository
        .load_agent_target_record(&identity)
        .unwrap_or_else(|error| panic!("load record: {error}"))
        .expect("record present");
    assert_eq!(
        loaded,
        AgentTargetRecord {
            target,
            status,
            reconcile_request: Some(expected_request),
        }
    );
}

#[test]
fn surface_keyed_skill_effect_apis_remain_available() {
    let (_directory, pool, workspace_id) = fixture();
    let repository = SqliteEffectRepository::new(pool.clone());
    let effect = repository
        .load_workspace_effect(&workspace_id)
        .unwrap_or_else(|error| panic!("load workspace effect: {error}"));
    assert_eq!(
        effect,
        WorkspaceEffect {
            workspace_id,
            generation: Generation::default(),
            spec: WorkspaceEffectSpec {
                skills: BTreeMap::new(),
            },
        }
    );

    let surface_table_exists: i64 = pool
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT EXISTS(
                     SELECT 1 FROM sqlite_master
                     WHERE type = 'table' AND name = 'effect_reconcile_requests'
                 )",
                    [],
                    |row| row.get(0),
                )
                .map_err(Into::into)
        })
        .unwrap();
    assert_eq!(surface_table_exists, 1);
}

fn load_table_names(connection: &Connection) -> Vec<String> {
    let mut statement = connection
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}
