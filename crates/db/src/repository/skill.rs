use super::effect::EffectWriteContext;
use crate::{LocalTimestampSource, TimestampSource};
use ora_application::{LocalSkillSourceRevision, RepositoryError, SkillRepository};
use ora_domain::{AuditFields, Namespace, PluginId, Skill, SkillId};
use ora_effect::{
    Digest, Fingerprint, SkillName, SkillSourceKey, SkillSourceKind, SourceRevisionKey,
};
use rusqlite::{OptionalExtension, Row, TransactionBehavior, params};
use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::repository::{
    RepositoryPool,
    connection::bool_to_sqlite,
    effect::{
        PublishedSkillRevision, advance_changed_scopes, publish_skill_revision, retire_skill_source,
    },
};

/// One validated Skill package projected from an installed plugin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginSkillProjection {
    pub name: String,
    pub description: String,
    pub package_root: PathBuf,
    pub skill_md_digest: Digest,
    pub package_fingerprint: Fingerprint,
}

/// Persists reusable skill definitions in SQLite.
#[derive(Clone, Debug)]
pub struct SqliteSkillRepository<Clock = LocalTimestampSource> {
    pool: RepositoryPool,
    clock: Clock,
}

impl SqliteSkillRepository {
    /// Builds a skill repository from the shared SQLite connection pool.
    pub fn new(pool: RepositoryPool) -> Self {
        Self {
            pool,
            clock: LocalTimestampSource,
        }
    }
}

impl<Clock: TimestampSource> SqliteSkillRepository<Clock> {
    /// Injects the audit clock used after acquiring Effect write transactions.
    pub fn with_clock(pool: RepositoryPool, clock: Clock) -> Self {
        Self { pool, clock }
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
            let write = EffectWriteContext::new(&transaction, &self.clock);
            let mut changed_scopes = BTreeSet::new();
            transaction.execute(
                "UPDATE skills SET is_deleted = 1, updated_at = ?2
                 WHERE namespace = ?1 COLLATE NOCASE AND is_deleted = 0",
                params![&plugin_id, updated_at],
            )?;
            let provided_names = skills
                .iter()
                .map(|skill| skill.name.to_ascii_lowercase())
                .collect::<Vec<_>>();
            retire_missing_plugin_sources(
                &transaction,
                &plugin_id,
                &provided_names,
                write.timestamp().millis(),
                &mut changed_scopes,
            )?;

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
                let publication = PublishedSkillRevision {
                    source: skill_source_key(
                        SkillSourceKind::Plugin,
                        Namespace::new(plugin_id.clone())?,
                        &canonical_name,
                    )?,
                    revision_key: SourceRevisionKey::parse(plugin_version).map_err(|error| {
                        crate::DatabaseError::CorruptEffectState(error.to_string())
                    })?,
                    skill_md_digest: skill.skill_md_digest.clone(),
                    package_fingerprint: skill.package_fingerprint.clone(),
                    package_root: skill.package_root.clone(),
                };
                publish_skill_revision(
                    &transaction,
                    &publication,
                    write.timestamp().millis(),
                    &mut changed_scopes,
                )?;
            }
            advance_changed_scopes(&transaction, &changed_scopes, write.timestamp().millis())?;
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
            let write = EffectWriteContext::new(&transaction, &self.clock);
            let mut changed_scopes = BTreeSet::new();
            transaction.execute(
                "UPDATE skills SET is_deleted = 1, updated_at = ?2
                 WHERE namespace = ?1 COLLATE NOCASE AND is_deleted = 0",
                params![&plugin_id, updated_at],
            )?;
            retire_plugin_sources(
                &transaction,
                &plugin_id,
                write.timestamp().millis(),
                &mut changed_scopes,
            )?;
            advance_changed_scopes(&transaction, &changed_scopes, write.timestamp().millis())?;
            transaction.commit()?;
            Ok(())
        })
    }
}

impl<Clock: TimestampSource> SkillRepository for SqliteSkillRepository<Clock> {
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
                let write = EffectWriteContext::new(&transaction, &self.clock);
                let mut changed_scopes = BTreeSet::new();
                insert_skill(&transaction, &skill)?;
                upsert_local_source(
                    &transaction,
                    &skill,
                    &source,
                    write.timestamp().millis(),
                    &mut changed_scopes,
                )?;
                advance_changed_scopes(&transaction, &changed_scopes, write.timestamp().millis())?;
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
                       json_extract(r.definition_json, '$.definition.package_root') AS source_package_root
                FROM skills s
                LEFT JOIN effect_sources e
                  ON e.source_kind = 'plugin'
                 AND e.namespace = s.namespace COLLATE NOCASE
                 AND e.identifier = s.name COLLATE NOCASE
                LEFT JOIN effect_revisions r ON r.id = e.published_revision_id
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
                       json_extract(r.definition_json, '$.definition.package_root') AS source_package_root
                FROM skills s
                LEFT JOIN effect_sources e
                  ON e.source_kind = 'plugin'
                 AND e.namespace = s.namespace COLLATE NOCASE
                 AND e.identifier = s.name COLLATE NOCASE
                LEFT JOIN effect_revisions r ON r.id = e.published_revision_id
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
                       json_extract(r.definition_json, '$.definition.package_root') AS source_package_root
                FROM skills s
                LEFT JOIN effect_sources e
                  ON e.source_kind = 'plugin'
                 AND e.namespace = s.namespace COLLATE NOCASE
                 AND e.identifier = s.name COLLATE NOCASE
                LEFT JOIN effect_revisions r ON r.id = e.published_revision_id
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
                let write = EffectWriteContext::new(&transaction, &self.clock);
                let mut changed_scopes = BTreeSet::new();
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
                    retire_local_source(
                        &transaction,
                        &previous_name,
                        write.timestamp().millis(),
                        &mut changed_scopes,
                    )?;
                }
                let _ = previous_namespace;
                upsert_local_source(
                    &transaction,
                    &skill,
                    &source,
                    write.timestamp().millis(),
                    &mut changed_scopes,
                )?;
                advance_changed_scopes(&transaction, &changed_scopes, write.timestamp().millis())?;
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
                let write = EffectWriteContext::new(&transaction, &self.clock);
                let mut changed_scopes = BTreeSet::new();
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
                retire_local_source(
                    &transaction,
                    &name,
                    write.timestamp().millis(),
                    &mut changed_scopes,
                )?;
                let _ = namespace;
                advance_changed_scopes(&transaction, &changed_scopes, write.timestamp().millis())?;
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
    written_at: i64,
    changed_scopes: &mut BTreeSet<String>,
) -> Result<(), crate::DatabaseError> {
    let publication = PublishedSkillRevision {
        source: skill_source_key(
            SkillSourceKind::Local,
            Namespace::local(),
            &skill.name.to_ascii_lowercase(),
        )?,
        revision_key: SourceRevisionKey::parse(skill.audit_fields.updated_at.to_string())
            .map_err(|error| crate::DatabaseError::CorruptEffectState(error.to_string()))?,
        skill_md_digest: source.skill_md_digest.clone(),
        package_fingerprint: source.package_fingerprint.clone(),
        package_root: source.package_root.clone(),
    };
    publish_skill_revision(connection, &publication, written_at, changed_scopes).map(|_| ())
}

/// Retires one Local source after the catalog row has been renamed or deleted.
fn retire_local_source(
    connection: &rusqlite::Connection,
    name: &str,
    updated_at: i64,
    changed_scopes: &mut BTreeSet<String>,
) -> Result<(), crate::DatabaseError> {
    let source = skill_source_key(SkillSourceKind::Local, Namespace::local(), name)?;
    retire_skill_source(connection, &source, updated_at, changed_scopes)?;
    Ok(())
}

/// Constructs the canonical stable Source identity shared by publication and retirement.
fn skill_source_key(
    source_kind: SkillSourceKind,
    namespace: Namespace,
    identifier: &str,
) -> Result<SkillSourceKey, crate::DatabaseError> {
    Ok(SkillSourceKey {
        source_kind,
        namespace,
        name: SkillName::parse(identifier)
            .map_err(|error| crate::DatabaseError::CorruptEffectState(error.to_string()))?,
    })
}

/// Retires plugin Sources that disappeared from the package's current asset snapshot.
fn retire_missing_plugin_sources(
    connection: &rusqlite::Connection,
    namespace: &str,
    provided_names: &[String],
    updated_at: i64,
    changed_scopes: &mut BTreeSet<String>,
) -> Result<(), crate::DatabaseError> {
    let mut statement = connection.prepare(
        "SELECT identifier FROM effect_sources
         WHERE effect_kind = 'ora/skill' AND source_kind = 'plugin'
           AND namespace = ?1 AND lifecycle = 'active'",
    )?;
    let sources = statement
        .query_map(params![namespace], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for identifier in sources {
        if !provided_names
            .iter()
            .any(|provided| provided.eq_ignore_ascii_case(&identifier))
        {
            let source = skill_source_key(
                SkillSourceKind::Plugin,
                Namespace::new(namespace.to_string())?,
                &identifier,
            )?;
            retire_skill_source(connection, &source, updated_at, changed_scopes)?;
        }
    }
    Ok(())
}

/// Retires every Skill Source belonging to one removed plugin namespace.
fn retire_plugin_sources(
    connection: &rusqlite::Connection,
    namespace: &str,
    updated_at: i64,
    changed_scopes: &mut BTreeSet<String>,
) -> Result<(), crate::DatabaseError> {
    let mut statement = connection.prepare(
        "SELECT identifier FROM effect_sources
         WHERE effect_kind = 'ora/skill' AND source_kind = 'plugin'
           AND namespace = ?1 AND lifecycle = 'active'",
    )?;
    let source_names = statement
        .query_map(params![namespace], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for source_name in source_names {
        let source = skill_source_key(
            SkillSourceKind::Plugin,
            Namespace::new(namespace.to_string())?,
            &source_name,
        )?;
        retire_skill_source(connection, &source, updated_at, changed_scopes)?;
    }
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
