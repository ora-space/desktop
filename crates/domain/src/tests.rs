use crate::{
    Artifact, ArtifactId, AuditFields, DomainModelError, Project, ProjectId, Session, SessionId,
    SessionStatus, Task, TaskId, TaskStatus, VirtualEntry, VirtualEntryId, VirtualEntryKind,
    VirtualFolder, VirtualFolderId, Worktree, WorktreeActivity, WorktreeId,
};
use pretty_assertions::assert_eq;

/// Verifies the domain can represent one fully populated example of each schema-backed entity.
#[test]
fn constructs_schema_backed_entities() {
    let audit_fields = AuditFields::new(1_700_000_000_000, 1_700_000_000_500, false);
    let project = Project::new(
        ProjectId::new("project-1"),
        "Ora",
        "/workspace/ora",
        audit_fields.clone(),
    );
    let worktree = Worktree::new(
        WorktreeId::new("worktree-1"),
        TaskId::new("task-1"),
        Some("feature/domain-models".to_string()),
        WorktreeActivity::Active,
        audit_fields.clone(),
    );
    let task = Task::new(
        TaskId::new("task-1"),
        project.id.clone(),
        "Implement domain models",
        TaskStatus::Doing,
        Some(worktree.id.clone()),
        audit_fields.clone(),
    );
    let virtual_folder = VirtualFolder::new(
        VirtualFolderId::new("folder-1"),
        project.id.clone(),
        "Context",
        ".ora/mounts/context",
        audit_fields.clone(),
    );
    let artifact = Artifact::new(
        ArtifactId::new("artifact-1"),
        task.id.clone(),
        Some("proposal".to_string()),
        audit_fields.clone(),
    );
    let entry = VirtualEntry::new(
        VirtualEntryId::new("entry-1"),
        virtual_folder.id.clone(),
        /*parent_entry_id*/ None,
        "proposal.md",
        VirtualEntryKind::File,
        Some(artifact.id.clone()),
        audit_fields.clone(),
    );
    let session = Session::new(
        SessionId::new("session-1"),
        task.id.clone(),
        "codex",
        Some("agent-session-1".to_string()),
        SessionStatus::Running,
        audit_fields.clone(),
    );

    assert_eq!(
        project,
        Project {
            id: ProjectId::new("project-1"),
            name: "Ora".to_string(),
            root_path: "/workspace/ora".to_string(),
            audit_fields: audit_fields.clone(),
        }
    );
    assert_eq!(
        worktree,
        Worktree {
            id: WorktreeId::new("worktree-1"),
            task_id: TaskId::new("task-1"),
            branch_name: Some("feature/domain-models".to_string()),
            activity: WorktreeActivity::Active,
            audit_fields: audit_fields.clone(),
        }
    );
    assert_eq!(
        task,
        Task {
            id: TaskId::new("task-1"),
            project_id: ProjectId::new("project-1"),
            title: "Implement domain models".to_string(),
            status: TaskStatus::Doing,
            worktree_id: Some(WorktreeId::new("worktree-1")),
            audit_fields: audit_fields.clone(),
        }
    );
    assert_eq!(
        virtual_folder,
        VirtualFolder {
            id: VirtualFolderId::new("folder-1"),
            project_id: ProjectId::new("project-1"),
            name: "Context".to_string(),
            mount_point: ".ora/mounts/context".to_string(),
            audit_fields: audit_fields.clone(),
        }
    );
    assert_eq!(
        artifact,
        Artifact {
            id: ArtifactId::new("artifact-1"),
            task_id: TaskId::new("task-1"),
            content: Some("proposal".to_string()),
            audit_fields: audit_fields.clone(),
        }
    );
    assert_eq!(
        entry,
        VirtualEntry {
            id: VirtualEntryId::new("entry-1"),
            virtual_folder_id: VirtualFolderId::new("folder-1"),
            parent_entry_id: None,
            name: "proposal.md".to_string(),
            kind: VirtualEntryKind::File,
            content_ref: Some(ArtifactId::new("artifact-1")),
            audit_fields: audit_fields.clone(),
        }
    );
    assert_eq!(
        session,
        Session {
            id: SessionId::new("session-1"),
            task_id: TaskId::new("task-1"),
            agent_id: "codex".to_string(),
            agent_session_id: Some("agent-session-1".to_string()),
            status: SessionStatus::Running,
            audit_fields,
        }
    );
}

/// Confirms every categorical enum round-trips to the integer encoding expected by SQLite.
#[test]
fn round_trips_database_backed_enums() {
    assert_eq!(TaskStatus::from_database_value(0), Ok(TaskStatus::Todo));
    assert_eq!(TaskStatus::Doing.database_value(), 1);
    assert_eq!(TaskStatus::Done.database_value(), 2);

    assert_eq!(
        WorktreeActivity::from_database_value(1),
        Ok(WorktreeActivity::Active)
    );
    assert_eq!(WorktreeActivity::Inactive.database_value(), 0);

    assert_eq!(
        VirtualEntryKind::from_database_value(0),
        Ok(VirtualEntryKind::File)
    );
    assert_eq!(VirtualEntryKind::Directory.database_value(), 1);

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
        TaskStatus::from_database_value(7),
        Err(DomainModelError::InvalidTaskStatus(7))
    );
    assert_eq!(
        WorktreeActivity::from_database_value(-1),
        Err(DomainModelError::InvalidWorktreeActivity(-1))
    );
    assert_eq!(
        VirtualEntryKind::from_database_value(9),
        Err(DomainModelError::InvalidVirtualEntryKind(9))
    );
    assert_eq!(
        SessionStatus::from_database_value(5),
        Err(DomainModelError::InvalidSessionStatus(5))
    );
}
