use super::{
    AgentDefinitionIdGenerator, AgentDefinitionRepository, CreateAgentDefinitionHandler,
    DeleteAgentDefinitionHandler, GetAgentDefinitionHandler, UpdateAgentDefinitionHandler,
};
use crate::{ApplicationError, Clock, RepositoryError};
use ora_contracts::{CreateAgentRequest, DeleteAgentRequest, GetAgentRequest, UpdateAgentRequest};
use ora_domain::{AgentDefinition, AgentDefinitionId, AuditFields};
use pretty_assertions::assert_eq;
use std::cell::RefCell;
use std::rc::Rc;

#[test]
fn creates_trimmed_agent_with_generated_id_and_updates_it_by_id() {
    let repository = Rc::new(FakeAgentRepository::default());
    let created = CreateAgentDefinitionHandler::new(
        repository.clone(),
        FixedAgentIdGenerator,
        FixedClock(10),
    )
    .handle(CreateAgentRequest {
        name: " opencode ".to_string(),
        description: "OpenCode".to_string(),
    })
    .unwrap();
    let updated = UpdateAgentDefinitionHandler::new(repository.clone(), FixedClock(20))
        .handle(UpdateAgentRequest {
            agent_id: created.agent.id.clone(),
            name: " reviewer ".to_string(),
            description: "Reviews".to_string(),
        })
        .unwrap();

    assert_eq!(updated.agent.id, "agent-1");
    assert_eq!(
        repository.agents.borrow().clone(),
        vec![agent("agent-1", "reviewer", "Reviews", 10, 20, false)]
    );
}

#[test]
fn reports_blank_name_not_found_repository_errors_and_soft_delete() {
    let blank = CreateAgentDefinitionHandler::new(
        Rc::new(FakeAgentRepository::default()),
        FixedAgentIdGenerator,
        FixedClock(1),
    )
    .handle(CreateAgentRequest {
        name: " ".to_string(),
        description: "Invalid".to_string(),
    })
    .unwrap_err();
    let missing = GetAgentDefinitionHandler::new(Rc::new(FakeAgentRepository::default()))
        .handle(GetAgentRequest {
            agent_id: "missing".to_string(),
        })
        .unwrap_err();
    let failing = Rc::new(FakeAgentRepository::default());
    failing.fail_next(RepositoryError::from_message("unavailable".to_string()));
    let repository_error = GetAgentDefinitionHandler::new(failing)
        .handle(GetAgentRequest {
            agent_id: "agent-1".to_string(),
        })
        .unwrap_err();
    let repository = Rc::new(FakeAgentRepository::with_agents(vec![agent(
        "agent-1", "opencode", "OpenCode", 1, 1, false,
    )]));
    DeleteAgentDefinitionHandler::new(repository.clone(), FixedClock(2))
        .handle(DeleteAgentRequest {
            agent_id: "agent-1".to_string(),
        })
        .unwrap();

    assert_eq!(blank, ApplicationError::AgentDefinitionNameBlank);
    assert_eq!(
        missing,
        ApplicationError::AgentDefinitionNotFound {
            agent_id: "missing".to_string()
        }
    );
    assert_eq!(
        repository_error,
        ApplicationError::AgentDefinitionRepository {
            source: RepositoryError::from_message("unavailable")
        }
    );
    assert_eq!(
        GetAgentDefinitionHandler::new(repository).handle(GetAgentRequest {
            agent_id: "agent-1".to_string()
        }),
        Err(ApplicationError::AgentDefinitionNotFound {
            agent_id: "agent-1".to_string()
        })
    );
}

#[derive(Default)]
struct FakeAgentRepository {
    agents: RefCell<Vec<AgentDefinition>>,
    next_error: RefCell<Option<RepositoryError>>,
}
impl FakeAgentRepository {
    fn with_agents(agents: Vec<AgentDefinition>) -> Self {
        Self {
            agents: RefCell::new(agents),
            next_error: RefCell::new(None),
        }
    }
    fn fail_next(&self, error: RepositoryError) {
        self.next_error.replace(Some(error));
    }
    fn take_error(&self) -> Result<(), RepositoryError> {
        self.next_error.borrow_mut().take().map_or(Ok(()), Err)
    }
}
impl AgentDefinitionRepository for Rc<FakeAgentRepository> {
    fn create_agent_definition(
        &self,
        agent: AgentDefinition,
    ) -> Result<AgentDefinition, RepositoryError> {
        self.take_error()?;
        self.agents.borrow_mut().push(agent.clone());
        Ok(agent)
    }
    fn find_agent_definition(
        &self,
        agent_id: &AgentDefinitionId,
    ) -> Result<Option<AgentDefinition>, RepositoryError> {
        self.take_error()?;
        Ok(self
            .agents
            .borrow()
            .iter()
            .find(|agent| agent.id == *agent_id && !agent.audit_fields.is_deleted)
            .cloned())
    }
    fn list_agent_definitions(&self) -> Result<Vec<AgentDefinition>, RepositoryError> {
        self.take_error()?;
        Ok(self
            .agents
            .borrow()
            .iter()
            .filter(|agent| !agent.audit_fields.is_deleted)
            .cloned()
            .collect())
    }
    fn update_agent_definition(
        &self,
        agent: AgentDefinition,
    ) -> Result<AgentDefinition, RepositoryError> {
        self.take_error()?;
        if let Some(existing) = self
            .agents
            .borrow_mut()
            .iter_mut()
            .find(|existing| existing.id == agent.id && !existing.audit_fields.is_deleted)
        {
            *existing = agent.clone();
            Ok(agent)
        } else {
            Err(RepositoryError::from_message("agent missing".to_string()))
        }
    }
    fn soft_delete_agent_definition(
        &self,
        agent_id: &AgentDefinitionId,
        deleted_at: i64,
    ) -> Result<bool, RepositoryError> {
        self.take_error()?;
        if let Some(agent) = self
            .agents
            .borrow_mut()
            .iter_mut()
            .find(|agent| agent.id == *agent_id && !agent.audit_fields.is_deleted)
        {
            agent.audit_fields.updated_at = deleted_at;
            agent.audit_fields.is_deleted = true;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
fn agent(
    id: &str,
    name: &str,
    description: &str,
    created_at: i64,
    updated_at: i64,
    is_deleted: bool,
) -> AgentDefinition {
    AgentDefinition::new(
        AgentDefinitionId::new(id),
        name,
        description,
        AuditFields::new(created_at, updated_at, is_deleted),
    )
    .unwrap()
}
struct FixedAgentIdGenerator;
impl AgentDefinitionIdGenerator for FixedAgentIdGenerator {
    fn generate_agent_definition_id(&self) -> AgentDefinitionId {
        AgentDefinitionId::new("agent-1")
    }
}
struct FixedClock(i64);
impl Clock for FixedClock {
    fn now_timestamp_millis(&self) -> i64 {
        self.0
    }
}
