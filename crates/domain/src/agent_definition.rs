use crate::{AgentDefinitionId, AuditFields, DomainModelError};
use serde::{Deserialize, Serialize};

/// Represents one configurable agent type rather than a runtime agent session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDefinition {
    pub id: AgentDefinitionId,
    pub name: String,
    pub description: String,
    pub content: String,
    pub audit_fields: AuditFields,
}

impl AgentDefinition {
    /// Creates an agent definition while normalizing its user-facing name for stable lookup.
    pub fn new(
        id: AgentDefinitionId,
        name: impl Into<String>,
        description: impl Into<String>,
        content: impl Into<String>,
        audit_fields: AuditFields,
    ) -> Result<Self, DomainModelError> {
        let name = name.into().trim().to_string();

        if name.is_empty() {
            return Err(DomainModelError::EmptyAgentDefinitionName);
        }

        Ok(Self {
            id,
            name,
            description: description.into(),
            content: content.into(),
            audit_fields,
        })
    }
}
