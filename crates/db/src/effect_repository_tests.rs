use crate::{
    DatabaseBootstrapper, DatabaseLocation, RepositoryPool, SourceMutationOutcome,
    SourcePublication, SqliteEffectRepository, TimestampSource, default_migration_catalog,
};
use ora_domain::{Namespace, WorkspaceId};
use ora_effect::{
    ConsumerCoordination, ConsumerId, DesiredSkillState, Digest, EffectRepository,
    FilesystemSkillSurface, Generation, MaterializationFormat, ReplaceEffectOutcome, SkillName,
    SkillSelectionKey, SkillSource, SkillState, SourceKind, SourceVersion, SurfaceDescriptorSet,
    SurfacePath, WorkspaceEffectSpec,
};
use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;
use rusqlite::params;
use std::collections::BTreeMap;
use tempfile::TempDir;

#[derive(Clone, Copy, Debug)]
struct FixedTimestamp;

impl TimestampSource for FixedTimestamp {
    fn current_timestamp_millis(&self) -> i64 {
        1
    }
}

/// Creates a file-backed pool with one valid Workspace foreign-key target.
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

/// Builds one active Local source revision.
fn local_source(version: &str, manifest: &[u8]) -> (SkillSelectionKey, DesiredSkillState) {
    let name =
        SkillName::parse("review").unwrap_or_else(|error| panic!("parse Skill name: {error}"));
    let key = SkillSelectionKey::new(SourceKind::Local, Namespace::local(), name.clone());
    let state = DesiredSkillState::try_new(SkillState {
        name,
        skill_md_digest: Digest::sha256(manifest),
        source: SkillSource::Local {
            namespace: Namespace::local(),
            version: SourceVersion::parse(version)
                .unwrap_or_else(|error| panic!("parse source version: {error}")),
        },
    })
    .unwrap_or_else(|error| panic!("build source: {error}"));
    (key, state)
}

/// Registers one consumer-declared physical surface for request-upsert assertions.
fn register_surface(repository: &SqliteEffectRepository, workspace_id: &WorkspaceId, now: i64) {
    let descriptors = SurfaceDescriptorSet::merge(
        workspace_id,
        [FilesystemSkillSurface {
            workspace_relative_path: SurfacePath::parse(".agents/skills")
                .unwrap_or_else(|error| panic!("parse surface path: {error}")),
            materialization_format: MaterializationFormat::skill_directory_v1(),
            consumer: ConsumerId::new("codex"),
            coordination: ConsumerCoordination::WaitForIdleAndRestart,
        }],
    )
    .unwrap_or_else(|error| panic!("merge surface: {error}"));
    repository
        .replace_surfaces(
            workspace_id,
            std::path::Path::new("/workspace"),
            &descriptors,
            now,
        )
        .unwrap_or_else(|error| panic!("register surface: {error}"));
}

#[test]
fn desired_replace_uses_cas_and_normalized_no_op_semantics() {
    let (_directory, pool, workspace_id) = fixture();
    let repository = SqliteEffectRepository::new(pool.clone());
    let source = local_source("1", b"manifest-v1");
    repository
        .publish_source(
            &source.1,
            std::path::Path::new("/catalog/review"),
            SourcePublication::Create,
            10,
        )
        .unwrap_or_else(|error| panic!("publish source: {error}"));
    register_surface(&repository, &workspace_id, 10);
    let spec = WorkspaceEffectSpec {
        skills: BTreeMap::from([(source.0, source.1)]),
    };

    let replaced = repository
        .replace_workspace_effect(&workspace_id, Generation::default(), spec.clone(), 20)
        .unwrap_or_else(|error| panic!("replace desired: {error}"));
    assert!(matches!(
        replaced,
        ReplaceEffectOutcome::Replaced(ref effect)
            if effect.generation == Generation::new(1)
    ));
    assert_eq!(
        repository
            .replace_workspace_effect(&workspace_id, Generation::new(1), spec, 30)
            .unwrap_or_else(|error| panic!("replace no-op: {error}")),
        ReplaceEffectOutcome::Unchanged(
            repository
                .load_workspace_effect(&workspace_id)
                .unwrap_or_else(|error| panic!("load effect: {error}"))
        )
    );
    assert_eq!(
        repository
            .replace_workspace_effect(
                &workspace_id,
                Generation::default(),
                WorkspaceEffectSpec::default(),
                40,
            )
            .unwrap_or_else(|error| panic!("replace conflict: {error}")),
        ReplaceEffectOutcome::Conflict {
            expected_generation: Generation::default(),
            current_generation: Generation::new(1),
        }
    );

    let request_generation = pool
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT requested_generation FROM effect_reconcile_requests
                     WHERE workspace_id = ?1",
                    params![workspace_id.as_ref()],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(Into::into)
        })
        .unwrap_or_else(|error| panic!("load request: {error}"));
    assert_eq!(request_generation, 1);
}

#[test]
fn source_delete_is_protected_and_updates_coalesce_to_latest_revision() {
    let (_directory, pool, workspace_id) = fixture();
    let repository = SqliteEffectRepository::new(pool);
    let version_one = local_source("1", b"manifest-v1");
    repository
        .publish_source(
            &version_one.1,
            std::path::Path::new("/catalog/review"),
            SourcePublication::Create,
            10,
        )
        .unwrap_or_else(|error| panic!("publish v1: {error}"));
    repository
        .replace_workspace_effect(
            &workspace_id,
            Generation::default(),
            WorkspaceEffectSpec {
                skills: BTreeMap::from([(version_one.0.clone(), version_one.1)]),
            },
            20,
        )
        .unwrap_or_else(|error| panic!("select v1: {error}"));
    assert_eq!(
        repository
            .delete_source(&version_one.0)
            .unwrap_or_else(|error| panic!("protect delete: {error}")),
        SourceMutationOutcome::InUse {
            workspace_ids: vec![workspace_id.clone()],
        }
    );

    let version_two = local_source("2", b"manifest-v2");
    let version_three = local_source("3", b"manifest-v3");
    repository
        .publish_source(
            &version_two.1,
            std::path::Path::new("/catalog/review"),
            SourcePublication::Update,
            30,
        )
        .unwrap_or_else(|error| panic!("publish v2: {error}"));
    repository
        .publish_source(
            &version_three.1,
            std::path::Path::new("/catalog/review"),
            SourcePublication::Update,
            40,
        )
        .unwrap_or_else(|error| panic!("publish v3: {error}"));
    assert_eq!(
        repository
            .list_propagation_requests()
            .unwrap_or_else(|error| panic!("list propagation: {error}")),
        vec![version_one.0.clone()]
    );
    assert_eq!(
        repository
            .propagate_source(&version_one.0, 50)
            .unwrap_or_else(|error| panic!("propagate latest: {error}")),
        vec![(workspace_id.clone(), Generation::new(2))]
    );
    let effect = repository
        .load_workspace_effect(&workspace_id)
        .unwrap_or_else(|error| panic!("load propagated effect: {error}"));
    assert_eq!(effect.generation, Generation::new(2));
    assert_eq!(
        effect.spec.skills[&version_one.0].state().source.version(),
        version_three.1.state().source.version()
    );
    assert!(
        repository
            .list_propagation_requests()
            .unwrap_or_else(|error| panic!("list completed propagation: {error}"))
            .is_empty()
    );
}

#[test]
fn unavailable_source_cannot_enter_desired_state() {
    let (_directory, pool, workspace_id) = fixture();
    let repository = SqliteEffectRepository::new(pool);
    let source = local_source("1", b"manifest-v1");
    repository
        .publish_source(
            &source.1,
            std::path::Path::new("/catalog/review"),
            SourcePublication::Create,
            10,
        )
        .unwrap_or_else(|error| panic!("publish source: {error}"));
    repository
        .mark_source_unavailable(&source.0, "external drift", 20)
        .unwrap_or_else(|error| panic!("mark unavailable: {error}"));

    assert_eq!(
        repository
            .replace_workspace_effect(
                &workspace_id,
                Generation::default(),
                WorkspaceEffectSpec {
                    skills: BTreeMap::from([(source.0.clone(), source.1)]),
                },
                30,
            )
            .unwrap_or_else(|error| panic!("replace desired: {error}")),
        ReplaceEffectOutcome::SourceUnavailable {
            selection_key: source.0,
        }
    );
}
