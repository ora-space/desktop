use ora_application::{
    LocalSkillSourceRevision, RepositoryError, SkillDeleteOutcome, SkillRepository,
    SkillUpdateOutcome,
};
use ora_domain::{AuditFields, Namespace, Skill, SkillId};
use rusqlite::{OptionalExtension, Row, TransactionBehavior, params};

use crate::repository::{RepositoryPool, connection::bool_to_sqlite};

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
                "SELECT id, namespace, name, description, created_at, updated_at, is_deleted FROM skills WHERE id = ?1 AND is_deleted = 0",
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
                "SELECT id, namespace, name, description, created_at, updated_at, is_deleted FROM skills WHERE namespace = ?1 COLLATE NOCASE AND name = ?2 COLLATE NOCASE AND is_deleted = 0",
            )?;
            let mut rows = statement.query(params![namespace.as_ref(), name])?;
            rows.next()?.map(map_skill_row).transpose()
        }).map_err(skill_repository_error_from_database)
    }

    fn list_skills(&self) -> Result<Vec<Skill>, RepositoryError> {
        self.pool.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, namespace, name, description, created_at, updated_at, is_deleted FROM skills WHERE is_deleted = 0 ORDER BY created_at ASC, id ASC",
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
    ) -> Result<SkillUpdateOutcome, RepositoryError> {
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
                    return Ok(SkillUpdateOutcome::InUse);
                };
                let selection_changed = !previous_namespace
                    .eq_ignore_ascii_case(skill.namespace.as_ref())
                    || !previous_name.eq_ignore_ascii_case(&skill.name);
                if selection_changed
                    && source_reference_exists(&transaction, &previous_namespace, &previous_name)?
                {
                    transaction.commit()?;
                    return Ok(SkillUpdateOutcome::InUse);
                }
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
                if !selection_changed {
                    upsert_local_propagation(&transaction, &skill)?;
                }
                transaction.commit()?;
                Ok(SkillUpdateOutcome::Updated(skill))
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

    fn soft_delete_skill_with_source_protection(
        &self,
        skill_id: &SkillId,
        deleted_at: i64,
    ) -> Result<SkillDeleteOutcome, RepositoryError> {
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
                    return Ok(SkillDeleteOutcome::Missing);
                };
                if source_reference_exists(&transaction, &namespace, &name)? {
                    transaction.commit()?;
                    return Ok(SkillDeleteOutcome::InUse);
                }
                transaction.execute(
                    "UPDATE skills SET updated_at = ?2, is_deleted = 1
                     WHERE id = ?1 AND is_deleted = 0",
                    params![skill_id.as_ref(), deleted_at],
                )?;
                delete_local_source(&transaction, &namespace, &name)?;
                transaction.commit()?;
                Ok(SkillDeleteOutcome::Deleted)
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
    connection.execute(
        "INSERT INTO effect_source_states (
             source_kind, namespace, skill_name, display_name, source_version,
             skill_md_digest, package_root, availability, unavailable_reason, updated_at
         ) VALUES ('local', ?1, ?2, ?3, ?4, ?5, ?6, 'available', NULL, ?7)
         ON CONFLICT(source_kind, namespace, skill_name) DO UPDATE SET
             display_name = excluded.display_name,
             source_version = excluded.source_version,
             skill_md_digest = excluded.skill_md_digest,
             package_root = excluded.package_root,
             availability = 'available', unavailable_reason = NULL,
             updated_at = excluded.updated_at",
        params![
            skill.namespace.as_ref(),
            skill.name.to_ascii_lowercase(),
            &skill.name,
            skill.audit_fields.updated_at.to_string(),
            source.skill_md_digest.as_str(),
            source.package_root.to_string_lossy(),
            skill.audit_fields.updated_at,
        ],
    )?;
    Ok(())
}

/// Coalesces Local V1-to-Vn updates by their stable selection identity.
fn upsert_local_propagation(
    connection: &rusqlite::Connection,
    skill: &Skill,
) -> Result<(), crate::DatabaseError> {
    connection.execute(
        "INSERT INTO effect_source_propagation_requests (
             source_kind, namespace, skill_name, requested_version, requested_at
         ) VALUES ('local', ?1, ?2, ?3, ?4)
         ON CONFLICT(source_kind, namespace, skill_name) DO UPDATE SET
             requested_version = excluded.requested_version,
             requested_at = excluded.requested_at",
        params![
            skill.namespace.as_ref(),
            skill.name.to_ascii_lowercase(),
            skill.audit_fields.updated_at.to_string(),
            skill.audit_fields.updated_at,
        ],
    )?;
    Ok(())
}

/// Checks Desired references under the caller's immediate write transaction.
fn source_reference_exists(
    connection: &rusqlite::Connection,
    namespace: &str,
    name: &str,
) -> Result<bool, crate::DatabaseError> {
    connection
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM workspace_effect_desired_skills
                 WHERE source_kind = 'local' AND namespace = ?1 AND skill_name = ?2
             )",
            params![namespace, name.to_ascii_lowercase()],
            |row| row.get::<_, i64>(0),
        )
        .map(|exists| exists != 0)
        .map_err(Into::into)
}

/// Removes source state and a stale wakeup after a protected rename or deletion.
fn delete_local_source(
    connection: &rusqlite::Connection,
    namespace: &str,
    name: &str,
) -> Result<(), crate::DatabaseError> {
    connection.execute(
        "DELETE FROM effect_source_propagation_requests
         WHERE source_kind = 'local' AND namespace = ?1 AND skill_name = ?2",
        params![namespace, name.to_ascii_lowercase()],
    )?;
    connection.execute(
        "DELETE FROM effect_source_states
         WHERE source_kind = 'local' AND namespace = ?1 AND skill_name = ?2",
        params![namespace, name.to_ascii_lowercase()],
    )?;
    Ok(())
}

/// Reconstructs a domain skill from a selected SQLite row.
fn map_skill_row(row: &Row<'_>) -> Result<Skill, crate::DatabaseError> {
    Skill::new(
        SkillId::new(row.get::<_, String>("id")?),
        Namespace::new(row.get::<_, String>("namespace")?)?,
        row.get::<_, String>("name")?,
        row.get::<_, String>("description")?,
        AuditFields::new(
            row.get("created_at")?,
            row.get("updated_at")?,
            row.get::<_, i64>("is_deleted")? != 0,
        ),
    )
    .map_err(Into::into)
}

/// Converts database failures into application-port errors.
fn skill_repository_error_from_database(error: crate::DatabaseError) -> RepositoryError {
    RepositoryError::new(error)
}
