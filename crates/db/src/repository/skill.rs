use ora_application::{LocalSkillSourceRevision, RepositoryError, SkillRepository};
use ora_domain::{AuditFields, Namespace, PluginId, Skill, SkillId};
use rusqlite::{OptionalExtension, Row, TransactionBehavior, params};
use std::path::PathBuf;
use uuid::Uuid;

use crate::repository::{RepositoryPool, connection::bool_to_sqlite};

/// One validated Skill package projected from an installed plugin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginSkillProjection {
    pub name: String,
    pub description: String,
    pub package_root: PathBuf,
    pub skill_md_digest: String,
}

/// Persists reusable skill definitions in SQLite.
#[derive(Clone, Debug)]
pub struct SqliteSkillRepository {
    pool: RepositoryPool,
}

impl SqliteSkillRepository {
    /// Builds a skill repository from the shared SQLite connection pool.
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }

    /// Replaces the catalog projection owned by one installed Skill plugin.
    pub fn replace_plugin_skills(
        &self,
        plugin_id: &PluginId,
        plugin_version: &str,
        skills: &[PluginSkillProjection],
        updated_at: i64,
    ) -> Result<(), crate::DatabaseError> {
        let plugin_id = plugin_id.canonical();
        self.pool.with_connection_mut(|connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute(
                "UPDATE skills SET is_deleted = 1, updated_at = ?2
                 WHERE namespace = ?1 COLLATE NOCASE AND is_deleted = 0",
                params![&plugin_id, updated_at],
            )?;
            let provided_names = skills
                .iter()
                .map(|skill| skill.name.to_ascii_lowercase())
                .collect::<Vec<_>>();
            retire_missing_plugin_sources(&transaction, &plugin_id, &provided_names, updated_at)?;

            for skill in skills {
                let canonical_name = skill.name.to_ascii_lowercase();
                let skill_id = format!("plugin:{plugin_id}:{canonical_name}");
                transaction.execute(
                    "INSERT INTO skills (
                         id, namespace, name, description, created_at, updated_at, is_deleted
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?5, 0)
                     ON CONFLICT(id) DO UPDATE SET
                         namespace = excluded.namespace,
                         name = excluded.name,
                         description = excluded.description,
                         updated_at = excluded.updated_at,
                         is_deleted = 0",
                    params![
                        skill_id,
                        &plugin_id,
                        &skill.name,
                        &skill.description,
                        updated_at,
                    ],
                )?;
                publish_skill_source(
                    &transaction,
                    "plugin",
                    &plugin_id,
                    &canonical_name,
                    &skill.name,
                    plugin_version,
                    &skill.skill_md_digest,
                    &skill.package_root,
                    updated_at,
                )?;
            }
            transaction.commit()?;
            Ok(())
        })
    }

    /// Makes every Skill from an uninstalled plugin disappear from the catalog.
    pub fn remove_plugin_skills(
        &self,
        plugin_id: &PluginId,
        updated_at: i64,
    ) -> Result<(), crate::DatabaseError> {
        let plugin_id = plugin_id.canonical();
        self.pool.with_connection_mut(|connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute(
                "UPDATE skills SET is_deleted = 1, updated_at = ?2
                 WHERE namespace = ?1 COLLATE NOCASE AND is_deleted = 0",
                params![&plugin_id, updated_at],
            )?;
            retire_plugin_sources(&transaction, &plugin_id, updated_at)?;
            transaction.commit()?;
            Ok(())
        })
    }
}

impl SkillRepository for SqliteSkillRepository {
    fn create_skill(&self, skill: Skill) -> Result<Skill, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO skills (id, namespace, name, description, created_at, updated_at, is_deleted) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        skill.id.to_string(),
                        skill.namespace.as_ref(),
                        &skill.name,
                        &skill.description,
                        skill.audit_fields.created_at,
                        skill.audit_fields.updated_at,
                        bool_to_sqlite(skill.audit_fields.is_deleted),
                    ],
                )?;
                Ok(skill)
            })
            .map_err(skill_repository_error_from_database)
    }

    fn create_skill_with_source(
        &self,
        skill: Skill,
        source: LocalSkillSourceRevision,
    ) -> Result<Skill, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                insert_skill(&transaction, &skill)?;
                upsert_local_source(&transaction, &skill, &source)?;
                transaction.commit()?;
                Ok(skill)
            })
            .map_err(skill_repository_error_from_database)
    }

    fn find_skill(&self, skill_id: &SkillId) -> Result<Option<Skill>, RepositoryError> {
        self.pool.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT s.id, s.namespace, s.name, s.description, s.created_at, s.updated_at, s.is_deleted,
                       e.source_kind,
                       json_extract(r.payload_json, '$.package_root') AS source_package_root
                FROM skills s
                LEFT JOIN effect_sources e
                  ON e.source_kind = 'plugin'
                 AND e.namespace = s.namespace COLLATE NOCASE
                 AND e.identifier = s.name COLLATE NOCASE
                LEFT JOIN effect_source_heads h ON h.source_id = e.id
                LEFT JOIN effect_source_revisions r ON r.id = h.revision_id
                WHERE s.id = ?1 AND s.is_deleted = 0",
            )?;
            let mut rows = statement.query(params![skill_id.to_string()])?;
            rows.next()?.map(map_skill_row).transpose()
        }).map_err(skill_repository_error_from_database)
    }

    fn find_skill_by_name(
        &self,
        namespace: &Namespace,
        name: &str,
    ) -> Result<Option<Skill>, RepositoryError> {
        self.pool.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT s.id, s.namespace, s.name, s.description, s.created_at, s.updated_at, s.is_deleted,
                       e.source_kind,
                       json_extract(r.payload_json, '$.package_root') AS source_package_root
                FROM skills s
                LEFT JOIN effect_sources e
                  ON e.source_kind = 'plugin'
                 AND e.namespace = s.namespace COLLATE NOCASE
                 AND e.identifier = s.name COLLATE NOCASE
                LEFT JOIN effect_source_heads h ON h.source_id = e.id
                LEFT JOIN effect_source_revisions r ON r.id = h.revision_id
                WHERE s.namespace = ?1 COLLATE NOCASE AND s.name = ?2 COLLATE NOCASE
                  AND s.is_deleted = 0",
            )?;
            let mut rows = statement.query(params![namespace.as_ref(), name])?;
            rows.next()?.map(map_skill_row).transpose()
        }).map_err(skill_repository_error_from_database)
    }

    fn list_skills(&self) -> Result<Vec<Skill>, RepositoryError> {
        self.pool.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT s.id, s.namespace, s.name, s.description, s.created_at, s.updated_at, s.is_deleted,
                       e.source_kind,
                       json_extract(r.payload_json, '$.package_root') AS source_package_root
                FROM skills s
                LEFT JOIN effect_sources e
                  ON e.source_kind = 'plugin'
                 AND e.namespace = s.namespace COLLATE NOCASE
                 AND e.identifier = s.name COLLATE NOCASE
                LEFT JOIN effect_source_heads h ON h.source_id = e.id
                LEFT JOIN effect_source_revisions r ON r.id = h.revision_id
                WHERE s.is_deleted = 0 ORDER BY s.created_at ASC, s.id ASC",
            )?;
            let mut rows = statement.query([])?;
            let mut skills = Vec::new();
            while let Some(row) = rows.next()? { skills.push(map_skill_row(row)?); }
            Ok(skills)
        }).map_err(skill_repository_error_from_database)
    }

    fn update_skill(&self, skill: Skill) -> Result<Skill, RepositoryError> {
        let updated = self.pool.with_connection(|connection| {
            connection.execute(
                "UPDATE skills SET namespace = ?2, name = ?3, description = ?4, updated_at = ?5 WHERE id = ?1 AND is_deleted = 0",
                params![skill.id.to_string(), skill.namespace.as_ref(), &skill.name, &skill.description, skill.audit_fields.updated_at],
            ).map(|rows| rows > 0).map_err(Into::into)
        }).map_err(skill_repository_error_from_database)?;
        if updated {
            Ok(skill)
        } else {
            Err(RepositoryError::new(std::io::Error::other(
                "skill not found during update",
            )))
        }
    }

    fn update_skill_with_source(
        &self,
        skill: Skill,
        source: LocalSkillSourceRevision,
    ) -> Result<Skill, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let previous = transaction
                    .query_row(
                        "SELECT namespace, name FROM skills WHERE id = ?1 AND is_deleted = 0",
                        params![skill.id.as_ref()],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                    )
                    .optional()?;
                let Some((previous_namespace, previous_name)) = previous else {
                    transaction.commit()?;
                    return Err(crate::DatabaseError::CorruptEffectState(
                        "Local Skill disappeared during update".to_string(),
                    ));
                };
                let selection_changed = !previous_namespace
                    .eq_ignore_ascii_case(skill.namespace.as_ref())
                    || !previous_name.eq_ignore_ascii_case(&skill.name);
                let updated = transaction.execute(
                    "UPDATE skills
                     SET namespace = ?2, name = ?3, description = ?4, updated_at = ?5
                     WHERE id = ?1 AND is_deleted = 0",
                    params![
                        skill.id.as_ref(),
                        skill.namespace.as_ref(),
                        &skill.name,
                        &skill.description,
                        skill.audit_fields.updated_at,
                    ],
                )?;
                if updated != 1 {
                    return Err(crate::DatabaseError::CorruptEffectState(
                        "Local Skill disappeared during update".to_string(),
                    ));
                }
                if selection_changed {
                    delete_local_source(&transaction, &previous_namespace, &previous_name)?;
                }
                upsert_local_source(&transaction, &skill, &source)?;
                transaction.commit()?;
                Ok(skill)
            })
            .map_err(skill_repository_error_from_database)
    }

    fn soft_delete_skill(
        &self,
        skill_id: &SkillId,
        deleted_at: i64,
    ) -> Result<bool, RepositoryError> {
        self.pool.with_connection(|connection| {
            connection.execute(
                "UPDATE skills SET updated_at = ?2, is_deleted = 1 WHERE id = ?1 AND is_deleted = 0",
                params![skill_id.to_string(), deleted_at],
            ).map(|rows| rows > 0).map_err(Into::into)
        }).map_err(skill_repository_error_from_database)
    }

    fn soft_delete_skill_with_source(
        &self,
        skill_id: &SkillId,
        deleted_at: i64,
    ) -> Result<bool, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let selection = transaction
                    .query_row(
                        "SELECT namespace, name FROM skills WHERE id = ?1 AND is_deleted = 0",
                        params![skill_id.as_ref()],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                    )
                    .optional()?;
                let Some((namespace, name)) = selection else {
                    transaction.commit()?;
                    return Ok(false);
                };
                transaction.execute(
                    "UPDATE skills SET updated_at = ?2, is_deleted = 1
                     WHERE id = ?1 AND is_deleted = 0",
                    params![skill_id.as_ref(), deleted_at],
                )?;
                delete_local_source(&transaction, &namespace, &name)?;
                transaction.commit()?;
                Ok(true)
            })
            .map_err(skill_repository_error_from_database)
    }
}

/// Inserts one Local Skill catalog row inside a larger source-publication transaction.
fn insert_skill(
    connection: &rusqlite::Connection,
    skill: &Skill,
) -> Result<(), crate::DatabaseError> {
    connection.execute(
        "INSERT INTO skills (
             id, namespace, name, description, created_at, updated_at, is_deleted
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            skill.id.as_ref(),
            skill.namespace.as_ref(),
            &skill.name,
            &skill.description,
            skill.audit_fields.created_at,
            skill.audit_fields.updated_at,
            bool_to_sqlite(skill.audit_fields.is_deleted),
        ],
    )?;
    Ok(())
}

/// Upserts the active Local source revision in the same transaction as its catalog row.
fn upsert_local_source(
    connection: &rusqlite::Connection,
    skill: &Skill,
    source: &LocalSkillSourceRevision,
) -> Result<(), crate::DatabaseError> {
    publish_skill_source(
        connection,
        "local",
        Namespace::local().as_ref(),
        &skill.name.to_ascii_lowercase(),
        &skill.name,
        &skill.audit_fields.updated_at.to_string(),
        source.skill_md_digest.as_str(),
        &source.package_root,
        skill.audit_fields.updated_at,
    )
}

/// Removes source state and a stale wakeup after a protected rename or deletion.
fn delete_local_source(
    connection: &rusqlite::Connection,
    namespace: &str,
    name: &str,
) -> Result<(), crate::DatabaseError> {
    let source_id = effect_source_id(
        connection,
        "local",
        Namespace::local().as_ref(),
        &name.to_ascii_lowercase(),
    )?;
    if let Some(source_id) = source_id {
        retire_source(connection, &source_id, 0)?;
    }
    let _ = namespace;
    Ok(())
}

/// Publishes one immutable Skill revision and installs a newly discovered source everywhere.
#[allow(clippy::too_many_arguments)]
fn publish_skill_source(
    connection: &rusqlite::Connection,
    source_kind: &str,
    namespace: &str,
    identifier: &str,
    display_name: &str,
    revision: &str,
    state_digest: &str,
    package_root: &std::path::Path,
    updated_at: i64,
) -> Result<(), crate::DatabaseError> {
    let existing_source_id = effect_source_id(connection, source_kind, namespace, identifier)?;
    let source_id = existing_source_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    connection.execute(
        "INSERT INTO effect_sources (
             id, effect_kind, source_kind, namespace, identifier, lifecycle,
             created_at, updated_at
         ) VALUES (?1, 'skill', ?2, ?3, ?4, 'active', ?5, ?5)
         ON CONFLICT(effect_kind, source_kind, namespace, identifier) DO UPDATE SET
             lifecycle = 'active', updated_at = excluded.updated_at",
        params![&source_id, source_kind, namespace, identifier, updated_at],
    )?;
    let payload = serde_json::json!({
        "display_name": display_name,
        "package_root": package_root.to_string_lossy(),
    })
    .to_string();
    let existing_revision = connection
        .query_row(
            "SELECT id, state_digest, payload_json FROM effect_source_revisions
             WHERE source_id = ?1 AND revision = ?2",
            params![&source_id, revision],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    let revision_id = match existing_revision {
        Some((revision_id, stored_digest, stored_payload)) => {
            if stored_digest != state_digest || stored_payload != payload {
                return Err(crate::DatabaseError::CorruptEffectState(
                    "an immutable Skill revision changed content".to_string(),
                ));
            }
            connection.execute(
                "UPDATE effect_source_revisions
                 SET availability = 'available', unavailable_reason = NULL, updated_at = ?2
                 WHERE id = ?1",
                params![&revision_id, updated_at],
            )?;
            revision_id
        }
        None => {
            let revision_id = Uuid::new_v4().to_string();
            connection.execute(
                "INSERT INTO effect_source_revisions (
                     id, source_id, revision, state_digest, payload_json, availability,
                     unavailable_reason, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, 'available', NULL, ?6, ?6)",
                params![
                    &revision_id,
                    &source_id,
                    revision,
                    state_digest,
                    payload,
                    updated_at,
                ],
            )?;
            revision_id
        }
    };
    let previous_head = connection
        .query_row(
            "SELECT revision_id FROM effect_source_heads WHERE source_id = ?1",
            params![&source_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    connection.execute(
        "INSERT INTO effect_source_heads (source_id, revision_id, updated_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(source_id) DO UPDATE SET
             revision_id = excluded.revision_id, updated_at = excluded.updated_at",
        params![&source_id, &revision_id, updated_at],
    )?;
    if existing_source_id.is_none() {
        install_source_in_all_workspaces(connection, &source_id, &revision_id, updated_at)?;
    } else if previous_head.as_deref() != Some(revision_id.as_str()) {
        enqueue_propagation(connection, &source_id, updated_at)?;
    }
    Ok(())
}

/// Finds the stable Effect Source identity for one catalog Skill.
fn effect_source_id(
    connection: &rusqlite::Connection,
    source_kind: &str,
    namespace: &str,
    identifier: &str,
) -> Result<Option<String>, crate::DatabaseError> {
    connection
        .query_row(
            "SELECT id FROM effect_sources
             WHERE effect_kind = 'skill' AND source_kind = ?1
               AND namespace = ?2 AND identifier = ?3",
            params![source_kind, namespace, identifier],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
}

/// Installs one new Source into every Workspace and wakes all active Surfaces atomically.
fn install_source_in_all_workspaces(
    connection: &rusqlite::Connection,
    source_id: &str,
    revision_id: &str,
    updated_at: i64,
) -> Result<(), crate::DatabaseError> {
    let mut statement = connection.prepare(
        "SELECT effects.workspace_id, effects.generation
         FROM workspace_effects effects
         WHERE NOT EXISTS (
             SELECT 1 FROM workspace_effect_desired_items desired
             WHERE desired.workspace_id = effects.workspace_id AND desired.source_id = ?1
         ) ORDER BY effects.workspace_id",
    )?;
    let workspaces = statement
        .query_map(params![source_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for (workspace_id, generation) in workspaces {
        let generation = generation.checked_add(1).ok_or_else(|| {
            crate::DatabaseError::CorruptEffectState("Workspace generation overflow".to_string())
        })?;
        connection.execute(
            "INSERT INTO workspace_effect_desired_items (
                 id, workspace_id, source_id, revision_id, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![
                Uuid::new_v4().to_string(),
                &workspace_id,
                source_id,
                revision_id,
                updated_at,
            ],
        )?;
        advance_workspace_effect(connection, &workspace_id, generation, updated_at)?;
    }
    Ok(())
}

/// Coalesces propagation work for the current Head of one Source.
fn enqueue_propagation(
    connection: &rusqlite::Connection,
    source_id: &str,
    updated_at: i64,
) -> Result<(), crate::DatabaseError> {
    connection.execute(
        "INSERT INTO effect_propagation_requests (
             source_id, head_revision_id, request_token, attempt_count,
             requested_at, not_before_at, updated_at
         ) SELECT ?1, revision_id, ?2, 0, ?3, ?3, ?3
           FROM effect_source_heads WHERE source_id = ?1
         ON CONFLICT(source_id) DO UPDATE SET
             head_revision_id = excluded.head_revision_id,
             request_token = excluded.request_token, attempt_count = 0,
             requested_at = excluded.requested_at, not_before_at = excluded.not_before_at,
             updated_at = excluded.updated_at",
        params![source_id, Uuid::new_v4().to_string(), updated_at],
    )?;
    Ok(())
}

/// Retires plugin Sources that disappeared from the package's current asset snapshot.
fn retire_missing_plugin_sources(
    connection: &rusqlite::Connection,
    namespace: &str,
    provided_names: &[String],
    updated_at: i64,
) -> Result<(), crate::DatabaseError> {
    let mut statement = connection.prepare(
        "SELECT id, identifier FROM effect_sources
         WHERE effect_kind = 'skill' AND source_kind = 'plugin'
           AND namespace = ?1 AND lifecycle = 'active'",
    )?;
    let sources = statement
        .query_map(params![namespace], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for (source_id, identifier) in sources {
        if !provided_names
            .iter()
            .any(|provided| provided.eq_ignore_ascii_case(&identifier))
        {
            retire_source(connection, &source_id, updated_at)?;
        }
    }
    Ok(())
}

/// Retires every Skill Source belonging to one removed plugin namespace.
fn retire_plugin_sources(
    connection: &rusqlite::Connection,
    namespace: &str,
    updated_at: i64,
) -> Result<(), crate::DatabaseError> {
    let mut statement = connection.prepare(
        "SELECT id FROM effect_sources
         WHERE effect_kind = 'skill' AND source_kind = 'plugin'
           AND namespace = ?1 AND lifecycle = 'active'",
    )?;
    let source_ids = statement
        .query_map(params![namespace], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for source_id in source_ids {
        retire_source(connection, &source_id, updated_at)?;
    }
    Ok(())
}

/// Removes all Desired references before making a Source unavailable for new selection.
fn retire_source(
    connection: &rusqlite::Connection,
    source_id: &str,
    updated_at: i64,
) -> Result<(), crate::DatabaseError> {
    let mut statement = connection.prepare(
        "SELECT effects.workspace_id, effects.generation, effects.updated_at
         FROM workspace_effects effects
         JOIN workspace_effect_desired_items desired
           ON desired.workspace_id = effects.workspace_id
         WHERE desired.source_id = ?1 ORDER BY effects.workspace_id",
    )?;
    let workspaces = statement
        .query_map(params![source_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for (workspace_id, generation, previous_updated_at) in workspaces {
        let generation = generation.checked_add(1).ok_or_else(|| {
            crate::DatabaseError::CorruptEffectState("Workspace generation overflow".to_string())
        })?;
        let effective_updated_at = if updated_at == 0 {
            previous_updated_at
        } else {
            updated_at
        };
        connection.execute(
            "DELETE FROM workspace_effect_desired_items
             WHERE workspace_id = ?1 AND source_id = ?2",
            params![&workspace_id, source_id],
        )?;
        advance_workspace_effect(connection, &workspace_id, generation, effective_updated_at)?;
    }
    connection.execute(
        "UPDATE effect_sources SET lifecycle = 'retired', updated_at = MAX(updated_at, ?2)
         WHERE id = ?1",
        params![source_id, updated_at],
    )?;
    connection.execute(
        "UPDATE effect_source_revisions
         SET availability = 'unavailable', unavailable_reason = 'source was removed',
             updated_at = MAX(updated_at, ?2)
         WHERE source_id = ?1",
        params![source_id, updated_at],
    )?;
    connection.execute(
        "DELETE FROM effect_propagation_requests WHERE source_id = ?1",
        params![source_id],
    )?;
    Ok(())
}

/// Advances a Workspace Desired generation and coalesces all active Surface wakeups.
fn advance_workspace_effect(
    connection: &rusqlite::Connection,
    workspace_id: &str,
    generation: i64,
    updated_at: i64,
) -> Result<(), crate::DatabaseError> {
    connection.execute(
        "UPDATE workspace_effects SET generation = ?2, updated_at = ?3 WHERE workspace_id = ?1",
        params![workspace_id, generation, updated_at],
    )?;
    connection.execute(
        "UPDATE effect_surface_status
         SET desired_generation = MAX(desired_generation, ?2),
             status_version = status_version + 1, updated_at = ?3
         WHERE surface_id IN (
             SELECT id FROM effect_surfaces
             WHERE workspace_id = ?1 AND lifecycle = 'active'
         )",
        params![workspace_id, generation, updated_at],
    )?;
    connection.execute(
        "INSERT INTO effect_reconcile_requests (
             surface_id, requested_generation, request_token, attempt_count,
             requested_at, not_before_at, updated_at
         )
         SELECT id, ?2, lower(hex(randomblob(16))), 0, ?3, ?3, ?3
         FROM effect_surfaces WHERE workspace_id = ?1 AND lifecycle = 'active'
         ON CONFLICT(surface_id) DO UPDATE SET
             requested_generation = MAX(requested_generation, excluded.requested_generation),
             request_token = excluded.request_token, attempt_count = 0,
             requested_at = excluded.requested_at, not_before_at = excluded.not_before_at,
             updated_at = excluded.updated_at",
        params![workspace_id, generation, updated_at],
    )?;
    Ok(())
}

/// Reconstructs a domain skill from a selected SQLite row.
fn map_skill_row(row: &Row<'_>) -> Result<Skill, crate::DatabaseError> {
    let id = SkillId::new(row.get::<_, String>("id")?);
    let namespace = Namespace::new(row.get::<_, String>("namespace")?)?;
    let name = row.get::<_, String>("name")?;
    let description = row.get::<_, String>("description")?;
    let audit_fields = AuditFields::new(
        row.get("created_at")?,
        row.get("updated_at")?,
        row.get::<_, i64>("is_deleted")? != 0,
    );
    let source_kind = row.get::<_, Option<String>>("source_kind")?;
    match source_kind.as_deref() {
        Some("plugin") => Skill::new_plugin(
            id,
            namespace.clone(),
            name,
            description,
            PluginId::parse(namespace.as_ref())?,
            PathBuf::from(row.get::<_, String>("source_package_root")?),
            audit_fields,
        )
        .map_err(Into::into),
        Some(other) => Err(crate::DatabaseError::CorruptEffectState(format!(
            "unexpected Skill source kind `{other}`"
        ))),
        None => Skill::new(id, namespace, name, description, audit_fields).map_err(Into::into),
    }
}

/// Converts database failures into application-port errors.
fn skill_repository_error_from_database(error: crate::DatabaseError) -> RepositoryError {
    RepositoryError::new(error)
}
