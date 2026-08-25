use ora_application::{
    BindWorkflowNodeSessionResult, NodeRunToStart, ProjectRepository, SessionRepository,
    SkillRepository, StartWorkflowRunResult, WorkflowRepository, WorkflowRunCreateOutcome,
    WorkflowRunEngineRepository, WorkflowRunRepository,
};
use ora_domain::{
    AgentRef, AuditFields, Namespace, PluginId, Project, ProjectId, Session, SessionId,
    SessionStatus, SkillOrigin, Workflow, WorkflowId, WorkflowNodeRunId, WorkflowRun,
    WorkflowRunId, WorkflowRunStatus, WorkflowSnapshot, WorkflowSnapshotId, Workspace,
    WorkspaceKind, WorkspaceLifecycle, WorkspaceLocation,
};
use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;
use tempfile::TempDir;

use crate::{
    DatabaseBootstrapper, DatabaseLocation, PluginSkillProjection, RepositoryPool,
    SqliteProjectRepository, SqliteSessionRepository, SqliteSkillRepository,
    SqliteWorkflowRepository, SqliteWorkflowRunEngineRepository, SqliteWorkflowRunRepository,
    SqliteWorkspaceRepository, TimestampSource, default_migration_catalog,
};

/// Supplies deterministic timestamps for repository integration fixtures.
#[derive(Clone, Copy, Debug)]
struct FixedTimestampSource;

impl TimestampSource for FixedTimestampSource {
    /// Returns the fixed timestamp used while opening the test database.
    fn current_timestamp_millis(&self) -> i64 {
        1
    }
}

/// Verifies plugin Skills use the existing Effect source table as their origin and package locator.
#[test]
fn plugin_skill_projection_round_trips_and_is_removed_with_its_plugin() {
    let (temp_dir, pool) = bootstrapped_pool();
    let repository = SqliteSkillRepository::new(pool);
    let plugin_id = PluginId::new("official", "review-pack").unwrap();
    let package_root = temp_dir.path().join("plugins/review-pack/review");
    repository
        .replace_plugin_skills(
            &plugin_id,
            "1.2.3",
            &[PluginSkillProjection {
                name: "review".to_string(),
                description: "Reviews changes".to_string(),
                package_root: package_root.clone(),
                skill_md_digest: format!("sha256:{}", "a".repeat(64)),
            }],
            10,
        )
        .unwrap();

    let skills = repository.list_skills().unwrap();
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].namespace.as_ref(), "official/review-pack");
    assert_eq!(skills[0].name, "review");
    assert_eq!(
        skills[0].origin,
        SkillOrigin::Plugin {
            plugin_id: plugin_id.clone(),
            package_root,
        }
    );

    repository.remove_plugin_skills(&plugin_id, 20).unwrap();
    assert!(repository.list_skills().unwrap().is_empty());
}
/// Verifies project creation materializes the shared main workspace used by ordinary sessions.
#[test]
fn project_creation_creates_main_workspace() {
    let (temp_dir, pool) = bootstrapped_pool();
    let workspace_path = existing_workspace_path(&temp_dir);
    let project_repository = SqliteProjectRepository::new(pool.clone());
    let workspace_repository = SqliteWorkspaceRepository::new(pool);
    let project = Project::new(
        ProjectId::new("project-1"),
        "Demo",
        AuditFields::new(10, 10, false),
    );

    assert_eq!(
        project_repository
            .create_project(
                project.clone(),
                WorkspaceLocation::local_filesystem(workspace_path.to_string_lossy()),
            )
            .unwrap(),
        project
    );
    let workspace = workspace_repository
        .list_workspaces(&ProjectId::new("project-1"))
        .unwrap();
    assert_eq!(
        workspace,
        vec![Workspace::new(
            workspace[0].id.clone(),
            ProjectId::new("project-1"),
            WorkspaceKind::Main,
            WorkspaceLocation::local_filesystem(workspace_path.to_string_lossy()),
            WorkspaceLifecycle::Active,
            AuditFields::new(10, 10, false),
        )],
    );
}

/// Verifies an absent local checkout stays in durable provisioning state instead of admitting work.
#[test]
fn project_creation_keeps_missing_main_workspace_in_provisioning() {
    let (temp_dir, pool) = bootstrapped_pool();
    let missing_path = temp_dir.path().join("missing-repository");
    let project_repository = SqliteProjectRepository::new(pool.clone());
    let workspace_repository = SqliteWorkspaceRepository::new(pool);
    project_repository
        .create_project(
            Project::new(
                ProjectId::new("project-1"),
                "Demo",
                AuditFields::new(10, 10, false),
            ),
            WorkspaceLocation::local_filesystem(missing_path.to_string_lossy()),
        )
        .unwrap();

    let workspace = workspace_repository
        .find_main_workspace(&ProjectId::new("project-1"))
        .unwrap()
        .unwrap();
    assert_eq!(workspace.lifecycle, WorkspaceLifecycle::Provisioning);
}

/// Verifies sessions can be stored and read with only their direct workspace foreign key.
#[test]
fn session_round_trip_uses_workspace_id() {
    let (temp_dir, pool) = bootstrapped_pool();
    let workspace_path = existing_workspace_path(&temp_dir);
    let project_repository = SqliteProjectRepository::new(pool.clone());
    let workspace_repository = SqliteWorkspaceRepository::new(pool.clone());
    let session_repository = SqliteSessionRepository::new(pool);
    project_repository
        .create_project(
            Project::new(
                ProjectId::new("project-1"),
                "Demo",
                AuditFields::new(10, 10, false),
            ),
            WorkspaceLocation::local_filesystem(workspace_path.to_string_lossy()),
        )
        .unwrap();
    let workspace = workspace_repository
        .find_main_workspace(&ProjectId::new("project-1"))
        .unwrap()
        .unwrap();
    let session = Session::new(
        SessionId::new("session-1"),
        workspace.id.clone(),
        AgentRef::parse("ora-space.opencode").unwrap(),
        "provider-session-1",
        // An unrelated running session shares the workspace but must not block
        // deletion of this completed workflow run.
        SessionStatus::Running,
        AuditFields::new(20, 20, false),
    );

    assert_eq!(
        session_repository.create_session(session.clone()).unwrap(),
        session
    );
    assert_eq!(
        session_repository
            .find_session(&SessionId::new("session-1"))
            .unwrap(),
        Some(session)
    );
}

/// Verifies ordinary session lists exclude sessions owned by workflow node execution.
#[test]
fn standalone_session_list_excludes_workflow_node_sessions() {
    let (temp_dir, pool) = bootstrapped_pool();
    let workspace_path = existing_workspace_path(&temp_dir);
    let project_repository = SqliteProjectRepository::new(pool.clone());
    let workspace_repository = SqliteWorkspaceRepository::new(pool.clone());
    let session_repository = SqliteSessionRepository::new(pool.clone());
    let workflow_repository = SqliteWorkflowRepository::new(pool.clone());
    let run_repository = SqliteWorkflowRunRepository::new(pool.clone());
    let engine_repository = SqliteWorkflowRunEngineRepository::new(pool);
    project_repository
        .create_project(
            Project::new(
                ProjectId::new("project-1"),
                "Demo",
                AuditFields::new(10, 10, false),
            ),
            WorkspaceLocation::local_filesystem(workspace_path.to_string_lossy()),
        )
        .unwrap();
    let workspace = workspace_repository
        .find_main_workspace(&ProjectId::new("project-1"))
        .unwrap()
        .unwrap();
    let standalone = Session::new(
        SessionId::new("session-standalone"),
        workspace.id.clone(),
        AgentRef::parse("ora-space.opencode").unwrap(),
        "provider-standalone",
        SessionStatus::Running,
        AuditFields::new(20, 20, false),
    );
    let workflow_session = Session::new(
        SessionId::new("session-workflow"),
        workspace.id.clone(),
        AgentRef::parse("ora-space.opencode").unwrap(),
        "provider-workflow",
        SessionStatus::Running,
        AuditFields::new(21, 21, false),
    );
    session_repository
        .create_session(standalone.clone())
        .unwrap();
    session_repository
        .create_session(workflow_session.clone())
        .unwrap();

    let workflow_id = WorkflowId::new("workflow-1");
    let snapshot_id = WorkflowSnapshotId::new("snapshot-1");
    workflow_repository
        .create_workflow(
            Workflow::new(
                workflow_id.clone(),
                Namespace::local(),
                "Review",
                None,
                AuditFields::new(10, 10, false),
            )
            .unwrap(),
            WorkflowSnapshot::new(
                snapshot_id.clone(),
                workflow_id.clone(),
                "draft",
                "{}",
                10,
                Some(10),
                false,
            ),
        )
        .unwrap();
    let run_id = WorkflowRunId::new("run-1");
    run_repository
        .create_run(WorkflowRun::new(
            run_id.clone(),
            workspace.id,
            workflow_id,
            snapshot_id,
            "Review run",
            WorkflowRunStatus::Pending,
            Some("{\"current_nodes\":[]}".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
            AuditFields::new(30, 30, false),
        ))
        .unwrap();
    let node_run_id = WorkflowNodeRunId::new("node-run-1");
    assert_eq!(
        engine_repository
            .start_run(
                &run_id,
                &NodeRunToStart {
                    id: node_run_id.clone(),
                    node_id: "agent-1".to_string(),
                    node_type: "agent".to_string(),
                    input: None,
                },
                40,
            )
            .unwrap(),
        StartWorkflowRunResult::Started
    );
    assert_eq!(
        engine_repository
            .bind_node_run_session(&node_run_id, &workflow_session.id, 50)
            .unwrap(),
        BindWorkflowNodeSessionResult::Bound
    );

    assert_eq!(
        session_repository.list_sessions().unwrap(),
        vec![standalone.clone(), workflow_session]
    );
    assert_eq!(
        session_repository.list_standalone_sessions().unwrap(),
        vec![standalone]
    );
}

/// Verifies workflow runs persist and project-list through their workspace without a task row.
#[test]
fn workflow_run_round_trip_uses_workspace_id() {
    let (temp_dir, pool) = bootstrapped_pool();
    let workspace_path = existing_workspace_path(&temp_dir);
    let project_repository = SqliteProjectRepository::new(pool.clone());
    let workspace_repository = SqliteWorkspaceRepository::new(pool.clone());
    let workflow_repository = SqliteWorkflowRepository::new(pool.clone());
    let run_repository = SqliteWorkflowRunRepository::new(pool);
    project_repository
        .create_project(
            Project::new(
                ProjectId::new("project-1"),
                "Demo",
                AuditFields::new(10, 10, false),
            ),
            WorkspaceLocation::local_filesystem(workspace_path.to_string_lossy()),
        )
        .unwrap();
    let workspace = workspace_repository
        .find_main_workspace(&ProjectId::new("project-1"))
        .unwrap()
        .unwrap();
    let workflow_id = WorkflowId::new("workflow-1");
    let snapshot_id = WorkflowSnapshotId::new("snapshot-1");
    workflow_repository
        .create_workflow(
            Workflow::new(
                workflow_id.clone(),
                Namespace::local(),
                "Review",
                None,
                AuditFields::new(10, 10, false),
            )
            .unwrap(),
            WorkflowSnapshot::new(
                snapshot_id.clone(),
                workflow_id.clone(),
                "draft",
                "{}",
                10,
                Some(10),
                false,
            ),
        )
        .unwrap();
    let run = WorkflowRun::new(
        WorkflowRunId::new("run-1"),
        workspace.id.clone(),
        workflow_id,
        snapshot_id,
        "Review run",
        WorkflowRunStatus::Succeeded,
        Some("done".to_string()),
        Some("{}".to_string()),
        None,
        None,
        None,
        Some(20),
        Some(30),
        ora_domain::AuditFields::new(20, 30, false),
    );

    assert_eq!(
        run_repository.create_run(run.clone()).unwrap(),
        WorkflowRunCreateOutcome::Created(Box::new(run.clone())),
    );
    assert_eq!(run_repository.find_run(&run.id).unwrap(), Some(run.clone()));
    assert_eq!(
        run_repository
            .list_runs_by_project(&ProjectId::new("project-1"))
            .unwrap(),
        vec![ora_domain::WorkflowRunSummary {
            id: run.id.clone(),
            name: run.name.clone(),
            workspace_id: workspace.id.clone(),
            project_id: ProjectId::new("project-1"),
            workflow_id: run.workflow_id.clone(),
            status: run.status,
            has_awaiting_node: false,
            started_at: run.started_at,
            finished_at: run.finished_at,
            created_at: run.audit_fields.created_at,
        }]
    );
}

/// Verifies deleting a run preserves the workspace and its independent session aggregate.
#[test]
fn deleting_workflow_run_does_not_delete_workspace_or_session() {
    let (temp_dir, pool) = bootstrapped_pool();
    let workspace_path = existing_workspace_path(&temp_dir);
    let project_repository = SqliteProjectRepository::new(pool.clone());
    let workspace_repository = SqliteWorkspaceRepository::new(pool.clone());
    let session_repository = SqliteSessionRepository::new(pool.clone());
    let workflow_repository = SqliteWorkflowRepository::new(pool.clone());
    let run_repository = SqliteWorkflowRunRepository::new(pool);
    project_repository
        .create_project(
            Project::new(
                ProjectId::new("project-1"),
                "Demo",
                AuditFields::new(10, 10, false),
            ),
            WorkspaceLocation::local_filesystem(workspace_path.to_string_lossy()),
        )
        .unwrap();
    let workspace = workspace_repository
        .find_main_workspace(&ProjectId::new("project-1"))
        .unwrap()
        .unwrap();
    let session = Session::new(
        SessionId::new("session-1"),
        workspace.id.clone(),
        AgentRef::parse("ora-space.opencode").unwrap(),
        "provider-session-1",
        SessionStatus::Stopped,
        ora_domain::AuditFields::new(20, 20, false),
    );
    session_repository.create_session(session.clone()).unwrap();
    let workflow_id = WorkflowId::new("workflow-1");
    let snapshot_id = WorkflowSnapshotId::new("snapshot-1");
    workflow_repository
        .create_workflow(
            Workflow::new(
                workflow_id.clone(),
                Namespace::local(),
                "Review",
                None,
                ora_domain::AuditFields::new(10, 10, false),
            )
            .unwrap(),
            WorkflowSnapshot::new(
                snapshot_id.clone(),
                workflow_id.clone(),
                "draft",
                "{}",
                10,
                Some(10),
                false,
            ),
        )
        .unwrap();
    run_repository
        .create_run(WorkflowRun::new(
            WorkflowRunId::new("run-1"),
            workspace.id.clone(),
            workflow_id,
            snapshot_id,
            "Review run",
            WorkflowRunStatus::Succeeded,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(30),
            ora_domain::AuditFields::new(20, 30, false),
        ))
        .unwrap();

    assert_eq!(
        run_repository
            .soft_delete_run(&WorkflowRunId::new("run-1"), 40)
            .unwrap(),
        ora_application::DeleteWorkflowRunResult::Deleted,
    );
    assert_eq!(
        workspace_repository.find_workspace(&workspace.id).unwrap(),
        Some(workspace),
    );
    assert_eq!(
        session_repository.find_session(&session.id).unwrap(),
        Some(session)
    );
}

/// Opens a file-backed repository so pooled adapters exercise the production connection path.
fn bootstrapped_pool() -> (TempDir, RepositoryPool) {
    let temp_dir = TempDir::new().expect("create temporary database directory");
    let database_path = temp_dir.path().join("repositories.sqlite3");
    let pool = with_trace_logging(|| {
        DatabaseBootstrapper::new(FixedTimestampSource)
            .bootstrap_repository_pool(
                &DatabaseLocation::path(database_path),
                &default_migration_catalog().expect("build migration catalog"),
            )
            .expect("bootstrap repository pool")
    });
    (temp_dir, pool)
}

/// Creates the existing local directory required for an admitted main Workspace fixture.
fn existing_workspace_path(temp_dir: &TempDir) -> std::path::PathBuf {
    let path = temp_dir.path().join("repository");
    std::fs::create_dir_all(&path).expect("create workspace directory");
    path
}
