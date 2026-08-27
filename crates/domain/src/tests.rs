use crate::{
    AgentDefinition, AgentDefinitionId, AgentRef, AuditFields, BACKUP_DIR_NAME, DomainModelError,
    HistoryState, JOURNAL_DIR_NAME, Namespace, Project, ProjectId, STAGING_DIR_NAME, Session,
    SessionId, SessionStatus, Skill, SkillId, Task, TaskId, WorkspaceId, Worktree,
    WorktreeActivity, WorktreeBaseline,
};
use pretty_assertions::assert_eq;

/// Verifies the domain can represent one fully populated example of each schema-backed entity.
#[test]
fn constructs_schema_backed_entities() {
    let audit_fields = AuditFields::new(1_700_000_000_000, 1_700_000_000_500, false);
    let project = Project::new(ProjectId::new("project-1"), "Ora", audit_fields.clone());
    let worktree = Worktree::new(
        WorkspaceId::new("workspace-1"),
        Some("feature/domain-models".to_string()),
        WorktreeBaseline::recorded("base-commit").unwrap(),
        WorktreeActivity::Active,
        audit_fields.clone(),
    );
    let task = Task::new(
        TaskId::new("task-1"),
        project.id.clone(),
        WorkspaceId::new("workspace-1"),
        "Implement domain models",
        audit_fields.clone(),
    );
    let session = Session::new(
        SessionId::new("session-1"),
        WorkspaceId::new("workspace-1"),
        AgentRef::parse("ora-space.nga").unwrap(),
        "agent-session-1",
        SessionStatus::Running,
        audit_fields.clone(),
    );
    let skill = Skill::new(
        SkillId::new("skill-1"),
        Namespace::local(),
        "review",
        "Reviews implementation changes",
        audit_fields.clone(),
    )
    .unwrap();
    let agent_definition = AgentDefinition::new(
        AgentDefinitionId::new("agent-definition-1"),
        Namespace::local(),
        "opencode",
        "OpenCode agent configuration",
        "",
        audit_fields.clone(),
    )
    .unwrap();

    assert_eq!(
        project,
        Project {
            id: ProjectId::new("project-1"),
            name: "Ora".to_string(),
            repository_kind: "git".to_string(),
            repository_url: None,
            default_branch: None,
            audit_fields: audit_fields.clone(),
        }
    );
    assert_eq!(
        worktree,
        Worktree {
            workspace_id: WorkspaceId::new("workspace-1"),
            branch_name: Some("feature/domain-models".to_string()),
            baseline: WorktreeBaseline::recorded("base-commit").unwrap(),
            activity: WorktreeActivity::Active,
            audit_fields: audit_fields.clone(),
        }
    );
    assert_eq!(
        task,
        Task {
            id: TaskId::new("task-1"),
            project_id: ProjectId::new("project-1"),
            workspace_id: WorkspaceId::new("workspace-1"),
            title: "Implement domain models".to_string(),
            audit_fields: audit_fields.clone(),
        }
    );
    assert_eq!(
        session,
        Session {
            id: SessionId::new("session-1"),
            workspace_id: WorkspaceId::new("workspace-1"),
            agent_ref: AgentRef::parse("ora-space.nga").unwrap(),
            agent_session_id: "agent-session-1".to_string(),
            title: None,
            status: SessionStatus::Running,
            history_state: HistoryState::Writable,
            audit_fields: audit_fields.clone(),
        }
    );
    assert_eq!(
        skill,
        Skill {
            id: SkillId::new("skill-1"),
            namespace: Namespace::local(),
            name: "review".to_string(),
            description: "Reviews implementation changes".to_string(),
            origin: crate::SkillOrigin::Local,
            audit_fields: audit_fields.clone(),
        }
    );
    assert_eq!(
        agent_definition,
        AgentDefinition {
            id: AgentDefinitionId::new("agent-definition-1"),
            namespace: Namespace::local(),
            name: "opencode".to_string(),
            description: "OpenCode agent configuration".to_string(),
            content: String::new(),
            audit_fields,
        }
    );
}

/// Verifies configurable resource constructors reject names that cannot identify a resource.
#[test]
fn rejects_blank_skill_and_agent_definition_names() {
    let audit_fields = AuditFields::new(1, 1, false);

    assert_eq!(
        Skill::new(
            SkillId::new("skill-1"),
            Namespace::local(),
            "  ",
            "",
            audit_fields.clone(),
        ),
        Err(DomainModelError::EmptySkillName)
    );
    assert_eq!(
        AgentDefinition::new(
            AgentDefinitionId::new("agent-definition-1"),
            Namespace::local(),
            "\t",
            "",
            "",
            audit_fields,
        ),
        Err(DomainModelError::EmptyAgentDefinitionName)
    );
}

/// Verifies namespace identity is non-empty and canonical across trusted resource owners.
#[test]
fn normalizes_and_validates_namespaces() {
    let deserialized = serde_json::from_str::<Namespace>(r#"" Ora.Plugin ""#).unwrap();

    assert_eq!(
        Namespace::new(" Ora.Plugin ").unwrap(),
        Namespace::new("ora.plugin").unwrap()
    );
    assert_eq!(deserialized, Namespace::new("ora.plugin").unwrap());
    assert_eq!(
        Namespace::new(" \t "),
        Err(DomainModelError::EmptyNamespace)
    );
    assert!(serde_json::from_str::<Namespace>(r#""  ""#).is_err());
}

/// Verifies skill names reject every dot-prefixed segment, including the storage layer's
/// reserved transaction directories and path-traversal segments.
#[test]
fn rejects_dot_prefixed_skill_names() {
    let audit_fields = AuditFields::new(1, 1, false);

    for name in [
        STAGING_DIR_NAME,
        BACKUP_DIR_NAME,
        JOURNAL_DIR_NAME,
        ".",
        "..",
        ".hidden",
        ".ORA-BACKUP",
    ] {
        assert_eq!(
            Skill::new(
                SkillId::new("skill-1"),
                Namespace::local(),
                name,
                "Rejected",
                audit_fields.clone()
            ),
            Err(DomainModelError::InvalidSkillName {
                name: name.to_string()
            })
        );
    }

    for accepted in ["backup.tmp", "ora-backup", "v1.2.3"] {
        assert_eq!(
            Skill::new(
                SkillId::new("skill-1"),
                Namespace::local(),
                accepted,
                "Accepted",
                audit_fields.clone()
            )
            .map(|skill| skill.name),
            Ok(accepted.to_string())
        );
    }
}

/// Verifies an agent reference accepts any installed provider id and rejects only blank text.
///
/// An identity Ora does not recognize is a provider that is not installed right now, so parsing
/// must not treat it as corrupt data the way a closed set would.
#[test]
fn parses_any_non_blank_agent_reference() {
    assert_eq!(
        ["ora-space.claude", "acme.my-agent", "  spaced.id  "].map(AgentRef::parse),
        [
            AgentRef::parse("ora-space.claude"),
            AgentRef::parse("acme.my-agent"),
            AgentRef::parse("spaced.id"),
        ]
    );
    assert_eq!(
        AgentRef::parse("   "),
        Err(DomainModelError::InvalidAgentRef("   ".to_string()))
    );
}

/// Confirms every categorical enum round-trips to the integer encoding expected by SQLite.
#[test]
fn round_trips_database_backed_enums() {
    assert_eq!(
        WorktreeActivity::from_database_value(1),
        Ok(WorktreeActivity::Active)
    );
    assert_eq!(WorktreeActivity::Inactive.database_value(), 0);

    assert_eq!(
        SessionStatus::from_database_value(1),
        Ok(SessionStatus::Stopped)
    );
    assert_eq!(SessionStatus::Running.database_value(), 0);
}

/// Ensures adapters cannot smuggle unsupported integer values into the domain layer.
#[test]
fn rejects_invalid_database_values() {
    assert_eq!(
        WorktreeBaseline::recorded("  "),
        Err(DomainModelError::EmptyWorktreeBaseline)
    );
    assert_eq!(
        WorktreeActivity::from_database_value(-1),
        Err(DomainModelError::InvalidWorktreeActivity(-1))
    );
    assert_eq!(
        SessionStatus::from_database_value(5),
        Err(DomainModelError::InvalidSessionStatus(5))
    );
}
