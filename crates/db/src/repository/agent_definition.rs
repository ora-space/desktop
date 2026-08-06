use ora_application::{AgentDefinitionRepository, RepositoryError};
use ora_domain::{AgentDefinition, AgentDefinitionId, AuditFields};
use rusqlite::{Row, params};

use crate::repository::{RepositoryPool, connection::bool_to_sqlite};

/// Persists configurable agent types in SQLite.
#[derive(Clone, Debug)]
pub struct SqliteAgentDefinitionRepository {
    pool: RepositoryPool,
}

impl SqliteAgentDefinitionRepository {
    /// Builds an agent-definition repository from the shared SQLite connection pool.
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }
}

impl AgentDefinitionRepository for SqliteAgentDefinitionRepository {
    fn create_agent_definition(
        &self,
        agent: AgentDefinition,
    ) -> Result<AgentDefinition, RepositoryError> {
        self.pool.with_connection(|connection| {
            connection.execute(
                "INSERT INTO agents (id, name, description, content, created_at, updated_at, is_deleted) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![agent.id.to_string(), &agent.name, &agent.description, &agent.content, agent.audit_fields.created_at, agent.audit_fields.updated_at, bool_to_sqlite(agent.audit_fields.is_deleted)],
            )?;
            Ok(agent)
        }).map_err(agent_repository_error_from_database)
    }

    fn find_agent_definition(
        &self,
        agent_id: &AgentDefinitionId,
    ) -> Result<Option<AgentDefinition>, RepositoryError> {
        self.pool.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, name, description, content, created_at, updated_at, is_deleted FROM agents WHERE id = ?1 AND is_deleted = 0",
            )?;
            let mut rows = statement.query(params![agent_id.to_string()])?;
            rows.next()?.map(map_agent_definition_row).transpose()
        }).map_err(agent_repository_error_from_database)
    }

    fn find_agent_definition_by_name(
        &self,
        name: &str,
    ) -> Result<Option<AgentDefinition>, RepositoryError> {
        self.pool.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, name, description, content, created_at, updated_at, is_deleted FROM agents WHERE name = ?1 COLLATE NOCASE AND is_deleted = 0 ORDER BY created_at ASC, id ASC LIMIT 1",
            )?;
            let mut rows = statement.query(params![name])?;
            rows.next()?.map(map_agent_definition_row).transpose()
        }).map_err(agent_repository_error_from_database)
    }

    fn list_agent_definitions(&self) -> Result<Vec<AgentDefinition>, RepositoryError> {
        self.pool.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, name, description, content, created_at, updated_at, is_deleted FROM agents WHERE is_deleted = 0 ORDER BY created_at ASC, id ASC",
            )?;
            let mut rows = statement.query([])?;
            let mut agents = Vec::new();
            while let Some(row) = rows.next()? { agents.push(map_agent_definition_row(row)?); }
            Ok(agents)
        }).map_err(agent_repository_error_from_database)
    }

    fn update_agent_definition(
        &self,
        agent: AgentDefinition,
    ) -> Result<AgentDefinition, RepositoryError> {
        let updated = self.pool.with_connection(|connection| {
            connection.execute(
                "UPDATE agents SET name = ?2, description = ?3, content = ?4, updated_at = ?5 WHERE id = ?1 AND is_deleted = 0",
                params![agent.id.to_string(), &agent.name, &agent.description, &agent.content, agent.audit_fields.updated_at],
            ).map(|rows| rows > 0).map_err(Into::into)
        }).map_err(agent_repository_error_from_database)?;
        if updated {
            Ok(agent)
        } else {
            Err(RepositoryError::new(std::io::Error::other(
                "agent definition not found during update",
            )))
        }
    }

    fn soft_delete_agent_definition(
        &self,
        agent_id: &AgentDefinitionId,
        deleted_at: i64,
    ) -> Result<bool, RepositoryError> {
        self.pool.with_connection(|connection| {
            connection.execute(
                "UPDATE agents SET updated_at = ?2, is_deleted = 1 WHERE id = ?1 AND is_deleted = 0",
                params![agent_id.to_string(), deleted_at],
            ).map(|rows| rows > 0).map_err(Into::into)
        }).map_err(agent_repository_error_from_database)
    }
}

/// Reconstructs a domain configurable agent type from a selected SQLite row.
fn map_agent_definition_row(row: &Row<'_>) -> Result<AgentDefinition, crate::DatabaseError> {
    AgentDefinition::new(
        AgentDefinitionId::new(row.get::<_, String>("id")?),
        row.get::<_, String>("name")?,
        row.get::<_, String>("description")?,
        row.get::<_, String>("content")?,
        AuditFields::new(
            row.get("created_at")?,
            row.get("updated_at")?,
            row.get::<_, i64>("is_deleted")? != 0,
        ),
    )
    .map_err(Into::into)
}

/// Converts database failures into application-port errors.
fn agent_repository_error_from_database(error: crate::DatabaseError) -> RepositoryError {
    RepositoryError::new(error)
}
