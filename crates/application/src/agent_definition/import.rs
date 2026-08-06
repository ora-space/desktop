use super::mapper::map_agent_definition;
use super::ports::{AgentDefinitionIdGenerator, AgentDefinitionRepository};
use crate::{ApplicationError, Clock};
use gray_matter::{Matter, ParsedEntity, engine::YAML};
use ora_contracts::{
    AgentImportCandidate, AgentImportCandidateStatus, AgentImportConflictInfo, AgentImportDecision,
    AgentImportResultStatus, CommitAgentImportRequest, CommitAgentImportResponse,
    PrepareAgentImportRequest, PrepareAgentImportResponse,
};
use ora_domain::{AgentDefinition, AuditFields};
use serde::Deserialize;

const MAX_AGENT_MARKDOWN_BYTES: usize = 1024 * 1024;

#[derive(Debug, Deserialize)]
struct AgentFrontMatter {
    name: String,
    description: String,
}

struct ParsedAgent {
    name: String,
    description: String,
    content: String,
}

pub struct AgentImportService<Repository, IdGenerator, ClockSource> {
    repository: Repository,
    id_generator: IdGenerator,
    clock: ClockSource,
}

impl<Repository, IdGenerator, ClockSource>
    AgentImportService<Repository, IdGenerator, ClockSource>
{
    pub fn new(repository: Repository, id_generator: IdGenerator, clock: ClockSource) -> Self {
        Self {
            repository,
            id_generator,
            clock,
        }
    }
}

impl<Repository, IdGenerator, ClockSource> AgentImportService<Repository, IdGenerator, ClockSource>
where
    Repository: AgentDefinitionRepository,
    IdGenerator: AgentDefinitionIdGenerator,
    ClockSource: Clock,
{
    pub fn prepare(
        &self,
        request: PrepareAgentImportRequest,
    ) -> Result<PrepareAgentImportResponse, ApplicationError> {
        let parsed = parse_agent_markdown(&request.content)?;
        let existing = self.find_by_name(&parsed.name)?;
        Ok(PrepareAgentImportResponse {
            candidate: AgentImportCandidate {
                name: parsed.name,
                description: parsed.description,
                status: if existing.is_some() {
                    AgentImportCandidateStatus::Conflict
                } else {
                    AgentImportCandidateStatus::Ready
                },
                existing_agent: existing.map(|agent| AgentImportConflictInfo {
                    agent_id: agent.id.to_string(),
                    updated_at: agent.audit_fields.updated_at,
                    description: agent.description,
                }),
            },
        })
    }

    pub fn commit(
        &self,
        request: CommitAgentImportRequest,
    ) -> Result<CommitAgentImportResponse, ApplicationError> {
        let parsed = parse_agent_markdown(&request.content)?;
        let existing = self.find_by_name(&parsed.name)?;

        match existing {
            None if request.expected_agent_id.is_some() => Ok(stale()),
            None => self.import_new(parsed),
            Some(existing)
                if request.expected_agent_id.as_deref() != Some(existing.id.as_ref())
                    || request.expected_updated_at != Some(existing.audit_fields.updated_at) =>
            {
                Ok(stale())
            }
            Some(_) if request.decision == Some(AgentImportDecision::Skip) => {
                Ok(CommitAgentImportResponse {
                    status: AgentImportResultStatus::Skipped,
                    agent: None,
                })
            }
            Some(existing) if request.decision == Some(AgentImportDecision::Overwrite) => {
                self.overwrite(existing, parsed)
            }
            Some(_) => Err(ApplicationError::AgentImportDecisionMissing),
        }
    }

    fn find_by_name(&self, name: &str) -> Result<Option<AgentDefinition>, ApplicationError> {
        self.repository
            .find_agent_definition_by_name(name)
            .map_err(ApplicationError::from_agent_definition_repository_error)
    }

    fn import_new(
        &self,
        parsed: ParsedAgent,
    ) -> Result<CommitAgentImportResponse, ApplicationError> {
        let now = self.clock.now_timestamp_millis();
        let agent = AgentDefinition::new(
            self.id_generator.generate_agent_definition_id(),
            parsed.name,
            parsed.description,
            parsed.content,
            AuditFields::new(now, now, false),
        )
        .map_err(ApplicationError::from_agent_definition_domain_error)?;
        let agent = self
            .repository
            .create_agent_definition(agent)
            .map_err(ApplicationError::from_agent_definition_repository_error)?;
        Ok(CommitAgentImportResponse {
            status: AgentImportResultStatus::Imported,
            agent: Some(map_agent_definition(agent)),
        })
    }

    fn overwrite(
        &self,
        existing: AgentDefinition,
        parsed: ParsedAgent,
    ) -> Result<CommitAgentImportResponse, ApplicationError> {
        let agent = AgentDefinition::new(
            existing.id,
            parsed.name,
            parsed.description,
            parsed.content,
            AuditFields::new(
                existing.audit_fields.created_at,
                self.clock.now_timestamp_millis(),
                false,
            ),
        )
        .map_err(ApplicationError::from_agent_definition_domain_error)?;
        let agent = self
            .repository
            .update_agent_definition(agent)
            .map_err(ApplicationError::from_agent_definition_repository_error)?;
        Ok(CommitAgentImportResponse {
            status: AgentImportResultStatus::Overwritten,
            agent: Some(map_agent_definition(agent)),
        })
    }
}

fn stale() -> CommitAgentImportResponse {
    CommitAgentImportResponse {
        status: AgentImportResultStatus::StaleConflict,
        agent: None,
    }
}

fn parse_agent_markdown(content: &str) -> Result<ParsedAgent, ApplicationError> {
    if content.is_empty() || content.len() > MAX_AGENT_MARKDOWN_BYTES {
        return Err(ApplicationError::AgentImportInvalid);
    }
    let parsed: ParsedEntity<AgentFrontMatter> = Matter::<YAML>::new()
        .parse(content)
        .map_err(|_| ApplicationError::AgentImportInvalid)?;
    let front_matter = parsed.data.ok_or(ApplicationError::AgentImportInvalid)?;
    let name = front_matter.name.trim().to_string();
    let description = front_matter.description.trim().to_string();
    if name.is_empty() || description.is_empty() {
        return Err(ApplicationError::AgentImportInvalid);
    }
    Ok(ParsedAgent {
        name,
        description,
        content: parsed.content,
    })
}
