use super::{AgentDefinitionIdGenerator, AgentDefinitionRepository, AgentImportService};
use crate::{ApplicationError, Clock, RepositoryError};
use ora_contracts::{
    AgentImportCandidateStatus, AgentImportDecision, AgentImportResultStatus,
    CommitAgentImportRequest, PrepareAgentImportRequest,
};
use ora_domain::{AgentDefinition, AgentDefinitionId, AuditFields};
use pretty_assertions::assert_eq;
use std::cell::RefCell;
use std::rc::Rc;

const READY: &str =
    "---\nname: reviewer\ndescription: Reviews changes\n---\n# Instructions\nBe precise.";
const CONFLICT: &str = "---\nname: REVIEWER\ndescription: New description\n---\nNew body";

#[test]
fn prepares_and_imports_one_markdown_agent() {
    let repository = Rc::new(FakeRepository::default());
    let service = service(repository.clone());

    let preview = service
        .prepare(PrepareAgentImportRequest {
            content: READY.into(),
        })
        .unwrap();
    assert_eq!(preview.candidate.status, AgentImportCandidateStatus::Ready);
    assert_eq!(preview.candidate.existing_agent, None);

    let result = service.commit(commit_request(READY, None, None)).unwrap();
    assert_eq!(result.status, AgentImportResultStatus::Imported);
    assert_eq!(result.agent.unwrap().name, "reviewer");
    assert_eq!(
        repository.agents.borrow()[0].content,
        "# Instructions\nBe precise."
    );
}

#[test]
fn previews_conflicts_case_insensitively_and_skips_them() {
    let repository = Rc::new(FakeRepository::with_agent(agent(10, 10, "Old body")));
    let service = service(repository.clone());

    let preview = service
        .prepare(PrepareAgentImportRequest {
            content: CONFLICT.into(),
        })
        .unwrap();
    assert_eq!(
        preview.candidate.status,
        AgentImportCandidateStatus::Conflict
    );
    let existing = preview.candidate.existing_agent.unwrap();

    let mut request = commit_request(CONFLICT, Some(existing.agent_id), Some(existing.updated_at));
    request.decision = Some(AgentImportDecision::Skip);
    let result = service.commit(request).unwrap();

    assert_eq!(result.status, AgentImportResultStatus::Skipped);
    assert_eq!(repository.agents.borrow()[0].content, "Old body");
}

#[test]
fn overwrites_the_frozen_conflict_and_preserves_identity() {
    let repository = Rc::new(FakeRepository::with_agent(agent(10, 10, "Old body")));
    let service = service(repository.clone());
    let existing = service
        .prepare(PrepareAgentImportRequest {
            content: CONFLICT.into(),
        })
        .unwrap()
        .candidate
        .existing_agent
        .unwrap();
    let mut request = commit_request(CONFLICT, Some(existing.agent_id), Some(existing.updated_at));
    request.decision = Some(AgentImportDecision::Overwrite);

    let result = service.commit(request).unwrap();

    assert_eq!(result.status, AgentImportResultStatus::Overwritten);
    let stored = &repository.agents.borrow()[0];
    assert_eq!(stored.id, AgentDefinitionId::new("agent-1"));
    assert_eq!(stored.audit_fields.created_at, 10);
    assert_eq!(stored.audit_fields.updated_at, 20);
    assert_eq!(stored.content, "New body");
}

#[test]
fn marks_ready_and_conflict_previews_stale_when_storage_changes() {
    let repository = Rc::new(FakeRepository::default());
    let service = service(repository.clone());
    service
        .prepare(PrepareAgentImportRequest {
            content: READY.into(),
        })
        .unwrap();
    repository.agents.borrow_mut().push(agent(1, 1, "claimed"));
    assert_eq!(
        service
            .commit(commit_request(READY, None, None))
            .unwrap()
            .status,
        AgentImportResultStatus::StaleConflict
    );

    let existing = repository.agents.borrow()[0].clone();
    let mut request = commit_request(
        CONFLICT,
        Some(existing.id.to_string()),
        Some(existing.audit_fields.updated_at),
    );
    request.decision = Some(AgentImportDecision::Overwrite);
    repository.agents.borrow_mut()[0].audit_fields.updated_at += 1;
    assert_eq!(
        service.commit(request).unwrap().status,
        AgentImportResultStatus::StaleConflict
    );
}

#[test]
fn rejects_missing_decisions_and_invalid_markdown() {
    let repository = Rc::new(FakeRepository::with_agent(agent(1, 1, "body")));
    let service = service(repository);
    let existing = service
        .prepare(PrepareAgentImportRequest {
            content: CONFLICT.into(),
        })
        .unwrap()
        .candidate
        .existing_agent
        .unwrap();

    assert_eq!(
        service
            .commit(commit_request(
                CONFLICT,
                Some(existing.agent_id),
                Some(existing.updated_at),
            ))
            .unwrap_err(),
        ApplicationError::AgentImportDecisionMissing
    );
    for invalid in [
        "",
        "plain markdown",
        "---\nname: reviewer\n---\nbody",
        "---\nname: [broken\n---\nbody",
        "---\nname: '  '\ndescription: desc\n---\nbody",
    ] {
        assert_eq!(
            service
                .prepare(PrepareAgentImportRequest {
                    content: invalid.into(),
                })
                .unwrap_err(),
            ApplicationError::AgentImportInvalid
        );
    }
    assert_eq!(
        service
            .prepare(PrepareAgentImportRequest {
                content: "x".repeat(1024 * 1024 + 1),
            })
            .unwrap_err(),
        ApplicationError::AgentImportInvalid
    );
}

fn commit_request(
    content: &str,
    expected_agent_id: Option<String>,
    expected_updated_at: Option<i64>,
) -> CommitAgentImportRequest {
    CommitAgentImportRequest {
        content: content.into(),
        decision: None,
        expected_agent_id,
        expected_updated_at,
    }
}

fn service(
    repository: Rc<FakeRepository>,
) -> AgentImportService<Rc<FakeRepository>, FixedId, FixedClock> {
    AgentImportService::new(repository, FixedId, FixedClock)
}

#[derive(Default)]
struct FakeRepository {
    agents: RefCell<Vec<AgentDefinition>>,
}

impl FakeRepository {
    fn with_agent(agent: AgentDefinition) -> Self {
        Self {
            agents: RefCell::new(vec![agent]),
        }
    }
}

impl AgentDefinitionRepository for Rc<FakeRepository> {
    fn create_agent_definition(
        &self,
        agent: AgentDefinition,
    ) -> Result<AgentDefinition, RepositoryError> {
        self.agents.borrow_mut().push(agent.clone());
        Ok(agent)
    }

    fn find_agent_definition(
        &self,
        agent_id: &AgentDefinitionId,
    ) -> Result<Option<AgentDefinition>, RepositoryError> {
        Ok(self
            .agents
            .borrow()
            .iter()
            .find(|agent| agent.id == *agent_id && !agent.audit_fields.is_deleted)
            .cloned())
    }

    fn find_agent_definition_by_name(
        &self,
        name: &str,
    ) -> Result<Option<AgentDefinition>, RepositoryError> {
        Ok(self
            .agents
            .borrow()
            .iter()
            .find(|agent| agent.name.eq_ignore_ascii_case(name) && !agent.audit_fields.is_deleted)
            .cloned())
    }

    fn list_agent_definitions(&self) -> Result<Vec<AgentDefinition>, RepositoryError> {
        Ok(self.agents.borrow().clone())
    }

    fn update_agent_definition(
        &self,
        agent: AgentDefinition,
    ) -> Result<AgentDefinition, RepositoryError> {
        let index = self
            .agents
            .borrow()
            .iter()
            .position(|stored| stored.id == agent.id)
            .ok_or_else(|| RepositoryError::from_message("missing"))?;
        self.agents.borrow_mut()[index] = agent.clone();
        Ok(agent)
    }

    fn soft_delete_agent_definition(
        &self,
        _agent_id: &AgentDefinitionId,
        _deleted_at: i64,
    ) -> Result<bool, RepositoryError> {
        Ok(false)
    }
}

struct FixedId;
impl AgentDefinitionIdGenerator for FixedId {
    fn generate_agent_definition_id(&self) -> AgentDefinitionId {
        AgentDefinitionId::new("agent-new")
    }
}

struct FixedClock;
impl Clock for FixedClock {
    fn now_timestamp_millis(&self) -> i64 {
        20
    }
}

fn agent(created_at: i64, updated_at: i64, content: &str) -> AgentDefinition {
    AgentDefinition::new(
        AgentDefinitionId::new("agent-1"),
        "reviewer",
        "Old description",
        content,
        AuditFields::new(created_at, updated_at, false),
    )
    .unwrap()
}
