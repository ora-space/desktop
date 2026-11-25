use ora_application::{RepositoryError, SkillNameClaimedError, SkillRepository};
use ora_domain::{AuditFields, Namespace, PluginId, Skill, SkillId, SkillOrigin};
use rusqlite::{Row, TransactionBehavior, params};
use std::path::PathBuf;

use crate::repository::{RepositoryPool, connection::bool_to_sqlite};

/// One validated Skill package projected from an installed plugin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginSkillProjection {
    pub name: String,
    pub description: String,
    pub package_root: PathBuf,
}

/// Persists reusable skill definitions in SQLite.
#[derive(Clone, Debug)]
pub struct SqliteSkillRepository {
    pool: RepositoryPool,
}

impl SqliteSkillRepository {
    /// Builds a skill repository from the shared repository pool.
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }

    /// Replaces the catalog projection owned by one installed Skill plugin.
    pub fn replace_plugin_skills(
        &self,
        plugin_id: &PluginId,
        _plugin_version: &str,
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

            for skill in skills {
                let canonical_name = skill.name.to_ascii_lowercase();
                let skill_id = format!("plugin:{plugin_id}:{canonical_name}");
                let package_root = skill.package_root.to_string_lossy().into_owned();
                transaction.execute(
                    "INSERT INTO skills (
                         id, namespace, name, description, package_root, created_at, updated_at, is_deleted
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, 0)
                     ON CONFLICT(id) DO UPDATE SET
                         namespace = excluded.namespace,
                         name = excluded.name,
                         description = excluded.description,
                         package_root = excluded.package_root,
                         updated_at = excluded.updated_at,
                         is_deleted = 0",
                    params![
                        skill_id,
                        &plugin_id,
                        &skill.name,
                        &skill.description,
                        package_root,
                        updated_at,
                    ],
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
            transaction.commit()?;
            Ok(())
        })
    }

    /// Looks up one skill row by exact catalog ID regardless of soft-deletion state.
    ///
    /// Callers resolving graph references use this so a deleted exact ID is recognized as a
    /// removed catalog entry rather than falling through to a same-named skill.
    pub fn find_skill_including_deleted(
        &self,
        skill_id: &SkillId,
    ) -> Result<Option<Skill>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, namespace, name, description, package_root, created_at, updated_at, is_deleted
                     FROM skills
                     WHERE id = ?1",
                )?;
                let mut rows = statement.query(params![skill_id.to_string()])?;
                rows.next()?.map(map_skill_row).transpose()
            })
            .map_err(skill_repository_error_from_database)
    }
}

impl SkillRepository for SqliteSkillRepository {
    fn create_skill(&self, skill: Skill) -> Result<Skill, RepositoryError> {
        let package_root = origin_package_root(&skill.origin);
        let claimed_name = (skill.namespace.as_ref().to_string(), skill.name.clone());
        self.pool
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO skills (
                         id, namespace, name, description, package_root, created_at, updated_at, is_deleted
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        skill.id.to_string(),
                        skill.namespace.as_ref(),
                        &skill.name,
                        &skill.description,
                        package_root,
                        skill.audit_fields.created_at,
                        skill.audit_fields.updated_at,
                        bool_to_sqlite(skill.audit_fields.is_deleted),
                    ],
                )?;
                Ok(skill)
            })
            .map_err(|error| skill_create_error_from_database(error, claimed_name))
    }

    fn find_skill(&self, skill_id: &SkillId) -> Result<Option<Skill>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, namespace, name, description, package_root, created_at, updated_at, is_deleted
                     FROM skills
                     WHERE id = ?1 AND is_deleted = 0",
                )?;
                let mut rows = statement.query(params![skill_id.to_string()])?;
                rows.next()?.map(map_skill_row).transpose()
            })
            .map_err(skill_repository_error_from_database)
    }

    fn find_skill_by_name(
        &self,
        namespace: &Namespace,
        name: &str,
    ) -> Result<Option<Skill>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, namespace, name, description, package_root, created_at, updated_at, is_deleted
                     FROM skills
                     WHERE namespace = ?1 COLLATE NOCASE AND name = ?2 COLLATE NOCASE
                       AND is_deleted = 0",
                )?;
                let mut rows = statement.query(params![namespace.as_ref(), name])?;
                rows.next()?.map(map_skill_row).transpose()
            })
            .map_err(skill_repository_error_from_database)
    }

    fn list_skills(&self) -> Result<Vec<Skill>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, namespace, name, description, package_root, created_at, updated_at, is_deleted
                     FROM skills
                     WHERE is_deleted = 0
                     ORDER BY created_at ASC, id ASC",
                )?;
                let mut rows = statement.query([])?;
                let mut skills = Vec::new();
                while let Some(row) = rows.next()? {
                    skills.push(map_skill_row(row)?);
                }
                Ok(skills)
            })
            .map_err(skill_repository_error_from_database)
    }

    fn update_skill(&self, skill: Skill) -> Result<Skill, RepositoryError> {
        let package_root = origin_package_root(&skill.origin);
        let updated = self
            .pool
            .with_connection(|connection| {
                connection
                    .execute(
                        "UPDATE skills
                         SET namespace = ?2, name = ?3, description = ?4, package_root = ?5, updated_at = ?6
                         WHERE id = ?1 AND is_deleted = 0",
                        params![
                            skill.id.to_string(),
                            skill.namespace.as_ref(),
                            &skill.name,
                            &skill.description,
                            package_root,
                            skill.audit_fields.updated_at,
                        ],
                    )
                    .map(|rows| rows > 0)
                    .map_err(Into::into)
            })
            .map_err(skill_repository_error_from_database)?;
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
        self.pool
            .with_connection(|connection| {
                connection
                    .execute(
                        "UPDATE skills SET updated_at = ?2, is_deleted = 1 WHERE id = ?1 AND is_deleted = 0",
                        params![skill_id.to_string(), deleted_at],
                    )
                    .map(|rows| rows > 0)
                    .map_err(Into::into)
            })
            .map_err(skill_repository_error_from_database)
    }
}

fn origin_package_root(origin: &SkillOrigin) -> Option<String> {
    match origin {
        SkillOrigin::Local => None,
        SkillOrigin::Plugin { package_root, .. } => {
            Some(package_root.to_string_lossy().into_owned())
        }
    }
}

/// Reconstructs a domain skill from a selected SQLite row.
fn map_skill_row(row: &Row<'_>) -> Result<Skill, crate::DatabaseError> {
    let id = SkillId::new(row.get::<_, String>("id")?);
    let namespace = Namespace::new(row.get::<_, String>("namespace")?)?;
    let name = row.get::<_, String>("name")?;
    let description = row.get::<_, String>("description")?;
    let package_root = row.get::<_, Option<String>>("package_root")?;
    let audit_fields = AuditFields::new(
        row.get("created_at")?,
        row.get("updated_at")?,
        row.get::<_, i64>("is_deleted")? != 0,
    );
    match package_root {
        Some(package_root) => Skill::new_plugin(
            id,
            namespace.clone(),
            name,
            description,
            PluginId::parse(namespace.as_ref())?,
            PathBuf::from(package_root),
            audit_fields,
        )
        .map_err(Into::into),
        None => Skill::new(id, namespace, name, description, audit_fields).map_err(Into::into),
    }
}

/// Converts database failures into application-port errors.
fn skill_repository_error_from_database(error: crate::DatabaseError) -> RepositoryError {
    RepositoryError::new(error)
}

/// Converts a rejected skill insert into a typed name claim when the active-name index fired.
///
/// `skills_active_namespace_name_unique` covers visible rows only, so a UNIQUE violation of this
/// insert is exactly a lost race for the name another visible skill now owns. A primary-key
/// collision keeps surfacing as a storage failure because it means the generated id collided.
fn skill_create_error_from_database(
    error: crate::DatabaseError,
    (namespace, name): (String, String),
) -> RepositoryError {
    if let crate::DatabaseError::Sqlite(rusqlite::Error::SqliteFailure(failure, _)) = &error
        && failure.code == rusqlite::ErrorCode::ConstraintViolation
        && failure.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
    {
        return RepositoryError::new(SkillNameClaimedError { namespace, name });
    }
    skill_repository_error_from_database(error)
}
