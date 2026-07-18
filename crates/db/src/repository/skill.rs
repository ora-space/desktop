use ora_application::{SkillRepository, SkillRepositoryError};
use ora_domain::{AuditFields, Skill, SkillId};
use rusqlite::{Row, params};

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
    fn create_skill(&self, skill: Skill) -> Result<Skill, SkillRepositoryError> {
        self.pool
            .with_connection(|connection| {
                insert_skill_row(connection, &skill)?;
                Ok(skill)
            })
            .map_err(skill_repository_error_from_database)
    }

    fn find_skill(&self, skill_id: &SkillId) -> Result<Option<Skill>, SkillRepositoryError> {
        self.pool.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, name, description, created_at, updated_at, is_deleted FROM skills WHERE id = ?1 AND is_deleted = 0",
            )?;
            let mut rows = statement.query(params![skill_id.to_string()])?;
            rows.next()?.map(map_skill_row).transpose()
        }).map_err(skill_repository_error_from_database)
    }

    fn list_skills(&self) -> Result<Vec<Skill>, SkillRepositoryError> {
        self.pool.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, name, description, created_at, updated_at, is_deleted FROM skills WHERE is_deleted = 0 ORDER BY created_at ASC, id ASC",
            )?;
            let mut rows = statement.query([])?;
            let mut skills = Vec::new();
            while let Some(row) = rows.next()? { skills.push(map_skill_row(row)?); }
            Ok(skills)
        }).map_err(skill_repository_error_from_database)
    }

    fn update_skill(&self, skill: Skill) -> Result<Skill, SkillRepositoryError> {
        let updated = self.pool.with_connection(|connection| {
            connection.execute(
                "UPDATE skills SET name = ?2, description = ?3, updated_at = ?4 WHERE id = ?1 AND is_deleted = 0",
                params![skill.id.to_string(), &skill.name, &skill.description, skill.audit_fields.updated_at],
            ).map(|rows| rows > 0).map_err(Into::into)
        }).map_err(skill_repository_error_from_database)?;
        if updated {
            Ok(skill)
        } else {
            Err(SkillRepositoryError::OperationFailed(
                "skill not found during update".to_string(),
            ))
        }
    }

    fn soft_delete_skill(
        &self,
        skill_id: &SkillId,
        deleted_at: i64,
    ) -> Result<bool, SkillRepositoryError> {
        self.pool.with_connection(|connection| {
            connection.execute(
                "UPDATE skills SET updated_at = ?2, is_deleted = 1 WHERE id = ?1 AND is_deleted = 0",
                params![skill_id.to_string(), deleted_at],
            ).map(|rows| rows > 0).map_err(Into::into)
        }).map_err(skill_repository_error_from_database)
    }
}

/// Inserts one skill row on the given connection, shared by the repository and the import unit of work.
///
/// Centralizing the statement keeps the per-statement create path and the transactional import path
/// writing identical columns, so the two insertion routes cannot drift apart.
pub(crate) fn insert_skill_row(
    connection: &rusqlite::Connection,
    skill: &Skill,
) -> Result<(), rusqlite::Error> {
    connection.execute(
        "INSERT INTO skills (id, name, description, created_at, updated_at, is_deleted) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            skill.id.to_string(),
            &skill.name,
            &skill.description,
            skill.audit_fields.created_at,
            skill.audit_fields.updated_at,
            bool_to_sqlite(skill.audit_fields.is_deleted),
        ],
    )?;

    Ok(())
}

/// Reconstructs a domain skill from a selected SQLite row.
fn map_skill_row(row: &Row<'_>) -> Result<Skill, crate::DatabaseError> {
    Skill::new(
        SkillId::new(row.get::<_, String>("id")?),
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
fn skill_repository_error_from_database(error: crate::DatabaseError) -> SkillRepositoryError {
    SkillRepositoryError::OperationFailed(error.to_string())
}
