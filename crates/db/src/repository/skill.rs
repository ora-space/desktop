use ora_application::{RepositoryError, SkillRepository};
use ora_domain::{AuditFields, Namespace, Skill, SkillId};
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
