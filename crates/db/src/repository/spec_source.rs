use ora_application::{ProjectSpecSourceOverrideRepository, RepositoryError};
use ora_domain::{
    AuditFields, ProjectId, ProjectSpecSourceOverride, ProjectSpecSourceOverrideId,
    SpecSourceVisibility, SpecWorkflow,
};
use rusqlite::{Row, Transaction, TransactionBehavior, params};

use crate::repository::{RepositoryPool, connection::bool_to_sqlite};

/// Persists project specification source overrides through the shared SQLite pool.
#[derive(Clone, Debug)]
pub struct SqliteProjectSpecSourceOverrideRepository {
    pool: RepositoryPool,
}

impl SqliteProjectSpecSourceOverrideRepository {
    /// Builds the repository from Ora's configured connection pool.
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }
}

impl ProjectSpecSourceOverrideRepository for SqliteProjectSpecSourceOverrideRepository {
    fn list_spec_source_overrides(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<ProjectSpecSourceOverride>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, project_id, relative_path, workflow_kind, custom_name, visibility,
                            created_at, updated_at, is_deleted
                     FROM project_spec_source_overrides
                     WHERE project_id = ?1 AND is_deleted = 0
                     ORDER BY relative_path COLLATE NOCASE, id",
                )?;
                let mut rows = statement.query(params![project_id.as_ref()])?;
                let mut sources = Vec::new();
                while let Some(row) = rows.next()? {
                    sources.push(map_source_row(row)?);
                }
                Ok(sources)
            })
            .map_err(RepositoryError::new)
    }

    fn replace_spec_source_overrides(
        &self,
        project_id: &ProjectId,
        replacements: Vec<ProjectSpecSourceOverride>,
        replaced_at: i64,
    ) -> Result<Vec<ProjectSpecSourceOverride>, RepositoryError> {
        self.pool
            .with_connection(|connection| {
                let transaction =
                    Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
                transaction.execute(
                    "UPDATE project_spec_source_overrides
                     SET updated_at = ?2, is_deleted = 1
                     WHERE project_id = ?1 AND is_deleted = 0",
                    params![project_id.as_ref(), replaced_at],
                )?;
                for source in &replacements {
                    let (workflow_kind, custom_name) = workflow_columns(&source.workflow);
                    let visibility = visibility_value(source.visibility);
                    transaction.execute(
                        "INSERT INTO project_spec_source_overrides (
                            id, project_id, relative_path, workflow_kind, custom_name, visibility,
                            created_at, updated_at, is_deleted
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                        params![
                            source.id.as_ref(),
                            source.project_id.as_ref(),
                            &source.relative_path,
                            workflow_kind,
                            custom_name,
                            visibility,
                            source.audit_fields.created_at,
                            source.audit_fields.updated_at,
                            bool_to_sqlite(source.audit_fields.is_deleted),
                        ],
                    )?;
                }
                transaction.commit()?;
                Ok(replacements)
            })
            .map_err(RepositoryError::new)
    }
}

/// Splits the strongly typed workflow into columns constrained by migration `0007`.
fn workflow_columns(workflow: &SpecWorkflow) -> (&'static str, Option<&str>) {
    match workflow {
        SpecWorkflow::OpenSpec => ("open_spec", None),
        SpecWorkflow::Superpowers => ("superpowers", None),
        SpecWorkflow::Custom { name } => ("custom", Some(name)),
    }
}

/// Serializes source visibility into its constrained database value.
fn visibility_value(visibility: SpecSourceVisibility) -> &'static str {
    match visibility {
        SpecSourceVisibility::Enabled => "enabled",
        SpecSourceVisibility::Disabled => "disabled",
    }
}

/// Reconstructs the strongly typed workflow and visibility from constrained database columns.
fn map_source_row(row: &Row<'_>) -> Result<ProjectSpecSourceOverride, crate::DatabaseError> {
    let workflow_kind = row.get::<_, String>("workflow_kind")?;
    let custom_name = row.get::<_, Option<String>>("custom_name")?;
    let workflow = match (workflow_kind.as_str(), custom_name) {
        ("open_spec", None) => SpecWorkflow::OpenSpec,
        ("superpowers", None) => SpecWorkflow::Superpowers,
        ("custom", Some(name)) => SpecWorkflow::Custom { name },
        _ => return Err(crate::DatabaseError::Sqlite(rusqlite::Error::InvalidQuery)),
    };
    let visibility = match row.get::<_, String>("visibility")?.as_str() {
        "enabled" => SpecSourceVisibility::Enabled,
        "disabled" => SpecSourceVisibility::Disabled,
        _ => return Err(crate::DatabaseError::Sqlite(rusqlite::Error::InvalidQuery)),
    };
    Ok(ProjectSpecSourceOverride::new(
        ProjectSpecSourceOverrideId::new(row.get::<_, String>("id")?),
        ProjectId::new(row.get::<_, String>("project_id")?),
        row.get::<_, String>("relative_path")?,
        workflow,
        visibility,
        AuditFields::new(
            row.get("created_at")?,
            row.get("updated_at")?,
            row.get::<_, i64>("is_deleted")? != 0,
        ),
    ))
}
