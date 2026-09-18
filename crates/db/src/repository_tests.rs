use ora_application::{
    BindWorkflowNodeSessionResult, CancelWorkflowRunResult, FileChange, NodeFailure,
    NodeFailureKind, NodeRunToStart, ProjectRepository, RestartWorkflowRunResult,
    ResumeWorkflowRunResult, SessionRepository, SkillRepository, StartWorkflowRunResult,
    WorkflowRepository, WorkflowRunCreateOutcome, WorkflowRunEngineRepository, WorkflowRunPayload,
    WorkflowRunRepository, WorkflowVariablePool,
};
use ora_contracts::WorkflowRunLocale;
use ora_domain::{
    AgentRef, AuditFields, Namespace, PluginId, Project, ProjectId, Session, SessionId,
    SessionStatus, SkillOrigin, Workflow, WorkflowId, WorkflowNodeRunId, WorkflowNodeStatus,
    WorkflowRun, WorkflowRunId, WorkflowRunStatus, WorkflowSnapshot, WorkflowSnapshotId, Workspace,
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
    let repository =
        SqliteSkillRepository::with_clock(pool.clone(), crate::test_clock::TestClock::new(1));
    let plugin_id = PluginId::new("official", "review-pack").unwrap();
    let package_root = temp_dir.path().join("plugins/review-pack/review");
    std::fs::create_dir_all(&package_root).unwrap();
    std::fs::write(package_root.join("SKILL.md"), b"manifest").unwrap();
    repository
        .replace_plugin_skills(
            &plugin_id,
            "1.2.3",
            &[PluginSkillProjection {
                name: "review".to_string(),
                description: "Reviews changes".to_string(),
                package_root: package_root.clone(),
                skill_md_digest: ora_effect::Digest::sha256(b"manifest"),
                package_fingerprint: ora_effect::Fingerprint::from(
                    ora_utils::directory::fingerprint_directory(&package_root, &[])
                        .unwrap_or_else(|error| panic!("fingerprint package: {error}")),
                ),
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

    let workspace_path = existing_workspace_path(&temp_dir);
    let project_repository =
        SqliteProjectRepository::with_clock(pool.clone(), crate::test_clock::TestClock::new(1));
    project_repository
        .create_project(
            Project::new(
                ProjectId::new("project-with-plugin-skill"),
                "Plugin Skill Project",
                AuditFields::new(15, 15, false),
            ),
            WorkspaceLocation::local_filesystem(workspace_path.to_string_lossy()),
        )
        .unwrap();
    let (workspace_id, generation, namespace, identifier) = pool
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT scopes.workspace_id, scopes.generation,
                            sources.namespace, sources.identifier
                     FROM effect_desired_effects desired
                     JOIN effect_scopes scopes ON scopes.id = desired.scope_id
                     JOIN effect_revisions revisions ON revisions.id = desired.revision_id
                     JOIN effect_sources sources ON sources.id = revisions.source_id",
                    [],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    },
                )
                .map_err(Into::into)
        })
        .unwrap();
    assert_eq!(
        (generation, namespace, identifier),
        (1, "official/review-pack".to_string(), "review".to_string())
    );

    repository.remove_plugin_skills(&plugin_id, 20).unwrap();
    assert!(repository.list_skills().unwrap().is_empty());
    let (generation, desired_count) = pool
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT scopes.generation, COUNT(desired.id)
                     FROM effect_scopes scopes
                     LEFT JOIN effect_desired_effects desired
                       ON desired.scope_id = scopes.id
                     WHERE scopes.workspace_id = ?1
                     GROUP BY scopes.workspace_id",
                    [workspace_id],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                )
                .map_err(Into::into)
        })
        .unwrap();
    assert_eq!((generation, desired_count), (2, 0));
}
/// Verifies project creation materializes the shared main workspace used by ordinary sessions.
#[test]
fn project_creation_creates_main_workspace() {
    let (temp_dir, pool) = bootstrapped_pool();
    let workspace_path = existing_workspace_path(&temp_dir);
    let project_repository =
        SqliteProjectRepository::with_clock(pool.clone(), crate::test_clock::TestClock::new(1));
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
    let project_repository =
        SqliteProjectRepository::with_clock(pool.clone(), crate::test_clock::TestClock::new(1));
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
fn session_round_trip_persists_workspace_and_mcp_selection() {
    let (temp_dir, pool) = bootstrapped_pool();
    let workspace_path = existing_workspace_path(&temp_dir);
    let project_repository =
        SqliteProjectRepository::with_clock(pool.clone(), crate::test_clock::TestClock::new(1));
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
        ora_domain::SessionMcpSelection::Explicit(std::collections::BTreeSet::from([
            ora_domain::PluginId::parse("official/github").unwrap(),
        ])),
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

/// Verifies workflow session ownership survives completion, restart, and database reopening.
#[test]
fn standalone_session_list_excludes_workflow_node_sessions() {
    let (temp_dir, pool) = bootstrapped_pool();
    let workspace_path = existing_workspace_path(&temp_dir);
    let project_repository =
        SqliteProjectRepository::with_clock(pool.clone(), crate::test_clock::TestClock::new(1));
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
        ora_domain::SessionMcpSelection::Automatic,
        AuditFields::new(20, 20, false),
    );
    let workflow_session = Session::new(
        SessionId::new("session-workflow"),
        workspace.id.clone(),
        AgentRef::parse("ora-space.opencode").unwrap(),
        "provider-workflow",
        SessionStatus::Running,
        ora_domain::SessionMcpSelection::Automatic,
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
                    scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                    node_id: "agent-1".to_string(),
                    node_type: "agent".to_string(),
                    input: None,
                    iteration: None,
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
        vec![standalone.clone(), workflow_session.clone()]
    );
    assert_eq!(
        session_repository.list_standalone_sessions().unwrap(),
        vec![standalone.clone()]
    );

    assert_eq!(
        engine_repository
            .complete_node(
                &node_run_id,
                Some("Agent result".to_string()),
                /*structured_output*/ None,
                /*stop_reason*/ None,
                Vec::new(),
                /*now*/ 60,
            )
            .unwrap(),
        ora_application::AdvanceWorkflowRunResult::Advanced
    );
    engine_repository
        .finish_run(&run_id, Some("Agent result".to_string()), /*now*/ 70)
        .unwrap();
    assert_eq!(
        session_repository.list_standalone_sessions().unwrap(),
        vec![standalone.clone()]
    );
    assert_eq!(
        engine_repository.restart_run(&run_id, /*now*/ 80).unwrap(),
        RestartWorkflowRunResult::Restarted
    );
    // Restart retires the node row, but must not turn its retained conversation into a chat.
    assert_eq!(
        session_repository.list_standalone_sessions().unwrap(),
        vec![standalone.clone()]
    );
    assert_eq!(
        session_repository
            .find_session(&workflow_session.id)
            .unwrap(),
        Some(workflow_session.clone())
    );

    let reopened_pool = with_trace_logging(|| {
        DatabaseBootstrapper::new(FixedTimestampSource)
            .bootstrap_repository_pool(
                &DatabaseLocation::path(temp_dir.path().join("repositories.sqlite3")),
                &default_migration_catalog().unwrap(),
            )
            .unwrap()
    });
    let reopened_sessions = SqliteSessionRepository::new(reopened_pool);
    assert_eq!(
        reopened_sessions.list_standalone_sessions().unwrap(),
        vec![standalone.clone()]
    );
    assert_eq!(
        reopened_sessions.list_sessions().unwrap(),
        vec![standalone, workflow_session]
    );
}

/// D2: a Failed run still binds an in-flight sibling whose node-run status is Running.
#[test]
fn bind_node_run_session_accepts_running_node_after_sibling_fails_the_run() {
    let (temp_dir, pool) = bootstrapped_pool();
    let engine_repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let run_repository = SqliteWorkflowRunRepository::new(pool.clone());
    let run_id = seed_pending_run(&temp_dir, &pool);
    assert_eq!(
        engine_repository
            .start_run(
                &run_id,
                &NodeRunToStart {
                    id: WorkflowNodeRunId::new("nr-start"),
                    scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                    node_id: "start".to_string(),
                    node_type: "start".to_string(),
                    input: None,
                    iteration: None,
                },
                40,
            )
            .unwrap(),
        StartWorkflowRunResult::Started
    );
    engine_repository
        .start_ready_nodes(
            &run_id,
            &[
                NodeRunToStart {
                    id: WorkflowNodeRunId::new("nr-a"),
                    scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                    node_id: "a".to_string(),
                    node_type: "agent".to_string(),
                    input: None,
                    iteration: None,
                },
                NodeRunToStart {
                    id: WorkflowNodeRunId::new("nr-b"),
                    scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                    node_id: "b".to_string(),
                    node_type: "agent".to_string(),
                    input: None,
                    iteration: None,
                },
            ],
            50,
        )
        .unwrap();
    let session_id = create_run_workspace_session(&pool, &run_id);
    engine_repository
        .fail_node(
            &WorkflowNodeRunId::new("nr-b"),
            NodeFailure::new(NodeFailureKind::PromptTemplate, "b prompt failed"),
            ora_application::FailurePropagation::Run,
            60,
        )
        .unwrap();
    let run = run_repository.find_run(&run_id).unwrap().unwrap();
    assert_eq!(run.status, WorkflowRunStatus::Failed);

    assert_eq!(
        engine_repository
            .bind_node_run_session(&WorkflowNodeRunId::new("nr-a"), &session_id, 70)
            .unwrap(),
        BindWorkflowNodeSessionResult::Bound
    );
    let nodes = engine_repository.list_node_runs(&run_id).unwrap();
    let node_a = nodes.iter().find(|node| node.node_id == "a").unwrap();
    assert_eq!(node_a.session_id.as_ref(), Some(&session_id));
    assert_eq!(node_a.status, WorkflowNodeStatus::Running);
}

/// Cancellation rejects bind through the node-run status, not the run status.
#[test]
fn bind_node_run_session_rejects_cancelled_node_run() {
    let (temp_dir, pool) = bootstrapped_pool();
    let engine_repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let run_id = seed_pending_run(&temp_dir, &pool);
    engine_repository
        .start_run(
            &run_id,
            &NodeRunToStart {
                id: WorkflowNodeRunId::new("nr-start"),
                scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                node_id: "start".to_string(),
                node_type: "start".to_string(),
                input: None,
                iteration: None,
            },
            40,
        )
        .unwrap();
    engine_repository
        .start_ready_nodes(
            &run_id,
            &[NodeRunToStart {
                id: WorkflowNodeRunId::new("nr-a"),
                scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                node_id: "a".to_string(),
                node_type: "agent".to_string(),
                input: None,
                iteration: None,
            }],
            50,
        )
        .unwrap();
    let session_id = create_run_workspace_session(&pool, &run_id);
    assert_eq!(
        engine_repository.cancel_run(&run_id, 60).unwrap(),
        CancelWorkflowRunResult::Cancelled
    );

    assert_eq!(
        engine_repository
            .bind_node_run_session(&WorkflowNodeRunId::new("nr-a"), &session_id, 70)
            .unwrap(),
        BindWorkflowNodeSessionResult::NotRunning
    );
}

/// Verifies workflow runs persist and project-list through their workspace without a task row.
#[test]
fn workflow_run_round_trip_uses_workspace_id() {
    let (temp_dir, pool) = bootstrapped_pool();
    let workspace_path = existing_workspace_path(&temp_dir);
    let project_repository =
        SqliteProjectRepository::with_clock(pool.clone(), crate::test_clock::TestClock::new(1));
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
            version: "draft".to_string(),
            status: run.status,
            has_awaiting_node: false,
            started_at: run.started_at,
            finished_at: run.finished_at,
            created_at: run.audit_fields.created_at,
        }]
    );
}

/// Verifies a restart resets variable values but keeps the catalog, re-seeding the task input.
#[test]
fn restart_resets_variable_values_and_keeps_the_catalog() {
    let (temp_dir, pool) = bootstrapped_pool();
    let workspace_path = existing_workspace_path(&temp_dir);
    let project_repository =
        SqliteProjectRepository::with_clock(pool.clone(), crate::test_clock::TestClock::new(1));
    let workspace_repository = SqliteWorkspaceRepository::new(pool.clone());
    let workflow_repository = SqliteWorkflowRepository::new(pool.clone());
    let run_repository = SqliteWorkflowRunRepository::new(pool.clone());
    let engine_repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
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

    let mut seeded = WorkflowVariablePool::default();
    seeded.declare("sys.task", "string", "system");
    seeded.declare("start.request", "string", "start");
    seeded.declare("start.count", "integer", "start");
    seeded.declare("review.text", "string", "review");
    seeded
        .set("sys.task", "system", serde_json::json!("旧任务"))
        .unwrap();
    seeded
        .set("start.request", "start", serde_json::json!("旧任务"))
        .unwrap();
    seeded
        .set("start.count", "start", serde_json::json!(2))
        .unwrap();
    seeded
        .set("review.text", "review", serde_json::json!("旧输出"))
        .unwrap();
    let mut run_payload = WorkflowRunPayload::with_variable_pool(
        WorkflowRunLocale::EnUs,
        Default::default(),
        Some("start".to_string()),
        seeded,
    );
    run_payload
        .condition_decisions
        .insert("condition-1".to_string(), "case-1".to_string());
    let payload = serde_json::to_string(&run_payload).unwrap();

    let run_id = WorkflowRunId::new("run-1");
    run_repository
        .create_run(WorkflowRun::new(
            run_id.clone(),
            workspace.id,
            workflow_id,
            snapshot_id,
            "Review run",
            WorkflowRunStatus::Succeeded,
            Some(r#"{"current_nodes":[]}"#.to_string()),
            Some("新任务".to_string()),
            Some("旧输出".to_string()),
            None,
            Some(payload),
            Some(20),
            Some(30),
            AuditFields::new(20, 30, false),
        ))
        .unwrap();

    assert_eq!(
        engine_repository.restart_run(&run_id, 40).unwrap(),
        RestartWorkflowRunResult::Restarted
    );

    let restarted = run_repository.find_run(&run_id).unwrap().unwrap();
    let parsed: WorkflowRunPayload =
        serde_json::from_str(restarted.payload.as_deref().unwrap()).unwrap();
    // Legacy instruction aliases are removed while real declarations remain available.
    assert!(!parsed.variable_pool.catalog.contains_key("sys.task"));
    assert!(!parsed.variable_pool.catalog.contains_key("start.request"));
    assert!(parsed.variable_pool.catalog.contains_key("start.count"));
    assert!(parsed.variable_pool.catalog.contains_key("review.text"));
    assert_eq!(
        parsed.variable_pool.values.get("start.count"),
        Some(&serde_json::json!(2))
    );
    assert!(!parsed.variable_pool.values.contains_key("review.text"));
    assert_eq!(
        parsed.condition_decisions,
        std::collections::BTreeMap::new()
    );
    // The mutation revision advanced so readers observe the reset.
    assert!(parsed.variable_pool.revision >= 1);
}

/// Resuming from failure clears only the listed writers' values and branch decisions.
#[test]
fn resume_from_failure_clears_listed_writers_and_keeps_other_values() {
    let (temp_dir, pool) = bootstrapped_pool();
    let engine_repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let run_repository = SqliteWorkflowRunRepository::new(pool.clone());
    let mut seeded = WorkflowVariablePool::default();
    seeded.declare("start.count", "integer", "start");
    seeded.declare("review.text", "string", "review");
    seeded
        .set("start.count", "start", serde_json::json!(2))
        .unwrap();
    seeded
        .set("review.text", "review", serde_json::json!("旧输出"))
        .unwrap();
    let mut run_payload = WorkflowRunPayload::with_variable_pool(
        WorkflowRunLocale::EnUs,
        Default::default(),
        Some("start".to_string()),
        seeded,
    );
    run_payload
        .condition_decisions
        .insert("condition-1".to_string(), "case-1".to_string());
    run_payload
        .condition_decisions
        .insert("other".to_string(), "kept".to_string());
    let revision_before = run_payload.variable_pool.revision;
    let run_id = seed_run(
        &temp_dir,
        &pool,
        WorkflowRunStatus::Failed,
        Some(serde_json::to_string(&run_payload).unwrap()),
        Some("review failed".to_string()),
        Some(20),
        Some(30),
    );
    engine_repository
        .start_ready_nodes(
            &run_id,
            &[NodeRunToStart {
                id: WorkflowNodeRunId::new("nr-review"),
                scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                node_id: "review".to_string(),
                node_type: "agent".to_string(),
                input: None,
                iteration: None,
            }],
            40,
        )
        .unwrap();
    engine_repository
        .fail_node(
            &WorkflowNodeRunId::new("nr-review"),
            NodeFailure::new(NodeFailureKind::Session, "review failed"),
            ora_application::FailurePropagation::Run,
            50,
        )
        .unwrap();

    assert_eq!(
        engine_repository
            .resume_from_failure(
                &run_id,
                &["review".to_string(), "condition-1".to_string()],
                70
            )
            .unwrap(),
        ResumeWorkflowRunResult::Resumed
    );

    let resumed = run_repository.find_run(&run_id).unwrap().unwrap();
    assert_eq!(resumed.status, WorkflowRunStatus::Running);
    assert_eq!(resumed.error, None);
    assert_eq!(resumed.finished_at, None);
    assert_eq!(resumed.started_at, Some(20));
    let parsed: WorkflowRunPayload =
        serde_json::from_str(resumed.payload.as_deref().unwrap()).unwrap();
    assert_eq!(
        parsed.variable_pool.values.get("start.count"),
        Some(&serde_json::json!(2))
    );
    assert!(!parsed.variable_pool.values.contains_key("review.text"));
    assert!(parsed.variable_pool.catalog.contains_key("review.text"));
    assert_eq!(
        parsed.variable_pool.revision,
        revision_before.saturating_add(1)
    );
    assert_eq!(
        parsed.condition_decisions,
        std::collections::BTreeMap::from([("other".to_string(), "kept".to_string())])
    );
    let live = engine_repository.list_node_runs(&run_id).unwrap();
    assert!(live.iter().all(|node| node.node_id != "review"));
    let deleted: i64 = pool
        .with_connection(|connection| {
            Ok(connection.query_row(
                "SELECT is_deleted FROM workflow_node_runs WHERE id = ?1",
                rusqlite::params!["nr-review"],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(deleted, 1);
}

/// A Failed run still holding a Running node cannot be resumed.
#[test]
fn resume_from_failure_rejects_a_run_with_a_running_node() {
    let (temp_dir, pool) = bootstrapped_pool();
    let engine_repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let run_id = seed_pending_run(&temp_dir, &pool);
    engine_repository
        .start_run(
            &run_id,
            &NodeRunToStart {
                id: WorkflowNodeRunId::new("nr-start"),
                scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                node_id: "start".to_string(),
                node_type: "start".to_string(),
                input: None,
                iteration: None,
            },
            40,
        )
        .unwrap();
    engine_repository
        .start_ready_nodes(
            &run_id,
            &[
                NodeRunToStart {
                    id: WorkflowNodeRunId::new("nr-a"),
                    scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                    node_id: "a".to_string(),
                    node_type: "agent".to_string(),
                    input: None,
                    iteration: None,
                },
                NodeRunToStart {
                    id: WorkflowNodeRunId::new("nr-b"),
                    scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                    node_id: "b".to_string(),
                    node_type: "agent".to_string(),
                    input: None,
                    iteration: None,
                },
            ],
            50,
        )
        .unwrap();
    engine_repository
        .fail_node(
            &WorkflowNodeRunId::new("nr-a"),
            NodeFailure::new(NodeFailureKind::Session, "a failed"),
            ora_application::FailurePropagation::Run,
            60,
        )
        .unwrap();

    assert_eq!(
        engine_repository
            .resume_from_failure(&run_id, &["a".to_string()], 70)
            .unwrap(),
        ResumeWorkflowRunResult::NotResumable
    );
}

/// A still-Running run cannot be resumed from failure.
#[test]
fn resume_from_failure_rejects_a_running_run() {
    let (temp_dir, pool) = bootstrapped_pool();
    let engine_repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let run_id = seed_pending_run(&temp_dir, &pool);
    engine_repository
        .start_run(
            &run_id,
            &NodeRunToStart {
                id: WorkflowNodeRunId::new("nr-start"),
                scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                node_id: "start".to_string(),
                node_type: "start".to_string(),
                input: None,
                iteration: None,
            },
            40,
        )
        .unwrap();

    assert_eq!(
        engine_repository
            .resume_from_failure(&run_id, &["start".to_string()], 50)
            .unwrap(),
        ResumeWorkflowRunResult::NotResumable
    );
}

/// Switching snapshot and payload is allowed only for Failed/Cancelled runs.
#[test]
fn switch_run_snapshot_updates_snapshot_and_payload_only_for_terminal_runs() {
    let (temp_dir, pool) = bootstrapped_pool();
    let engine_repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let run_repository = SqliteWorkflowRunRepository::new(pool.clone());
    let original = r#"{"locale":"zh-CN","skillMaterialization":{"bindings":[]}}"#;
    let migrated =
        r#"{"locale":"zh-CN","skillMaterialization":{"bindings":[]},"startNodeId":"start"}"#;
    let failed_id = seed_run(
        &temp_dir,
        &pool,
        WorkflowRunStatus::Failed,
        Some(original.to_string()),
        Some("err".to_string()),
        Some(20),
        Some(30),
    );
    pool.with_connection_mut(|connection| {
        connection.execute(
            "INSERT INTO workflow_snapshots (id, workflow_id, version, graph, created_at, updated_at, is_deleted)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params!["snapshot-2", "workflow-1", "v2", "{}", 40, None::<i64>, 0],
        )?;
        Ok(())
    })
    .unwrap();
    assert!(
        engine_repository
            .switch_run_snapshot(
                &failed_id,
                &WorkflowSnapshotId::new("snapshot-2"),
                migrated,
                80,
            )
            .unwrap()
    );
    let failed = run_repository.find_run(&failed_id).unwrap().unwrap();
    assert_eq!(failed.snapshot_id, WorkflowSnapshotId::new("snapshot-2"));
    assert_eq!(failed.payload.as_deref(), Some(migrated));

    let (running_dir, running_pool) = bootstrapped_pool();
    let running_engine = SqliteWorkflowRunEngineRepository::new(running_pool.clone());
    let running_runs = SqliteWorkflowRunRepository::new(running_pool.clone());
    let running_id = seed_pending_run(&running_dir, &running_pool);
    running_engine
        .start_run(
            &running_id,
            &NodeRunToStart {
                id: WorkflowNodeRunId::new("nr-start"),
                scope_id: ora_domain::WorkflowScopeId::new(format!("root:{running_id}")),
                node_id: "start".to_string(),
                node_type: "start".to_string(),
                input: None,
                iteration: None,
            },
            40,
        )
        .unwrap();
    let before = running_runs.find_run(&running_id).unwrap().unwrap();
    assert!(
        !running_engine
            .switch_run_snapshot(
                &running_id,
                &WorkflowSnapshotId::new("snapshot-2"),
                migrated,
                80,
            )
            .unwrap()
    );
    let after = running_runs.find_run(&running_id).unwrap().unwrap();
    assert_eq!(after.snapshot_id, before.snapshot_id);
    assert_eq!(after.payload, before.payload);
    assert_eq!(after.status, WorkflowRunStatus::Running);
}

/// The second failing node keeps the first run-level error and finished_at.
#[test]
fn second_fail_node_does_not_overwrite_run_level_error() {
    let (temp_dir, pool) = bootstrapped_pool();
    let engine_repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let run_repository = SqliteWorkflowRunRepository::new(pool.clone());
    let run_id = seed_pending_run(&temp_dir, &pool);
    engine_repository
        .start_run(
            &run_id,
            &NodeRunToStart {
                id: WorkflowNodeRunId::new("nr-start"),
                scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                node_id: "start".to_string(),
                node_type: "start".to_string(),
                input: None,
                iteration: None,
            },
            40,
        )
        .unwrap();
    engine_repository
        .start_ready_nodes(
            &run_id,
            &[
                NodeRunToStart {
                    id: WorkflowNodeRunId::new("nr-a"),
                    scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                    node_id: "a".to_string(),
                    node_type: "agent".to_string(),
                    input: None,
                    iteration: None,
                },
                NodeRunToStart {
                    id: WorkflowNodeRunId::new("nr-b"),
                    scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                    node_id: "b".to_string(),
                    node_type: "agent".to_string(),
                    input: None,
                    iteration: None,
                },
            ],
            50,
        )
        .unwrap();

    engine_repository
        .fail_node(
            &WorkflowNodeRunId::new("nr-a"),
            NodeFailure::new(NodeFailureKind::Session, "error-a"),
            ora_application::FailurePropagation::Run,
            60,
        )
        .unwrap();
    let after_a = run_repository.find_run(&run_id).unwrap().unwrap();
    assert_eq!(after_a.status, WorkflowRunStatus::Failed);
    assert_eq!(after_a.error.as_deref(), Some("error-a"));
    assert_eq!(after_a.finished_at, Some(60));

    engine_repository
        .fail_node(
            &WorkflowNodeRunId::new("nr-b"),
            NodeFailure::new(NodeFailureKind::Session, "error-b"),
            ora_application::FailurePropagation::Run,
            80,
        )
        .unwrap();
    let after_b = run_repository.find_run(&run_id).unwrap().unwrap();
    assert_eq!(after_b.status, WorkflowRunStatus::Failed);
    assert_eq!(after_b.error.as_deref(), Some("error-a"));
    assert_eq!(after_b.finished_at, Some(60));
    let nodes = engine_repository.list_node_runs(&run_id).unwrap();
    let node_b = nodes.iter().find(|node| node.node_id == "b").unwrap();
    assert_eq!(node_b.status, WorkflowNodeStatus::Failed);
    assert_eq!(node_b.error.as_deref(), Some("error-b"));
}

/// `fail_node` writes `payload.error_detail` on the first attempt and keeps other payload keys.
#[test]
fn fail_node_writes_error_detail_and_preserves_existing_payload_keys() {
    let (temp_dir, pool) = bootstrapped_pool();
    let engine_repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let run_id = seed_run(
        &temp_dir,
        &pool,
        WorkflowRunStatus::Failed,
        None,
        Some("review failed".to_string()),
        Some(20),
        Some(30),
    );
    engine_repository
        .start_ready_nodes(
            &run_id,
            &[NodeRunToStart {
                id: WorkflowNodeRunId::new("nr-review"),
                scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                node_id: "review".to_string(),
                node_type: "agent".to_string(),
                input: None,
                iteration: None,
            }],
            40,
        )
        .unwrap();
    pool.with_connection(|connection| {
        connection.execute(
            "UPDATE workflow_node_runs SET payload = ?2 WHERE id = ?1",
            rusqlite::params!["nr-review", r#"{"stop_reason":"end_turn"}"#],
        )?;
        Ok(())
    })
    .unwrap();
    engine_repository
        .fail_node(
            &WorkflowNodeRunId::new("nr-review"),
            NodeFailure::new(NodeFailureKind::Session, "review failed"),
            ora_application::FailurePropagation::Run,
            50,
        )
        .unwrap();

    let nodes = engine_repository.list_node_runs(&run_id).unwrap();
    let node = nodes.iter().find(|node| node.node_id == "review").unwrap();
    let payload = node.payload.as_deref().unwrap();
    println!("payload={payload}");
    let parsed: serde_json::Value = serde_json::from_str(payload).unwrap();
    assert_eq!(parsed["stop_reason"], "end_turn");
    assert_eq!(parsed["error_detail"]["kind"], "session");
    assert_eq!(parsed["error_detail"]["attempt"], 1);
    assert_eq!(parsed["error_detail"]["resumable"], true);
    assert_eq!(parsed["error_detail"]["injects_previous_failure"], false);
    assert_eq!(parsed["error_detail"]["recorded_at"], 50);
    assert_eq!(parsed["error_detail"]["message"], "review failed");
}

/// A second failure of the same node after resume increments `error_detail.attempt`.
#[test]
fn fail_node_increments_attempt_after_resume_from_failure() {
    let (temp_dir, pool) = bootstrapped_pool();
    let engine_repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let run_id = seed_run(
        &temp_dir,
        &pool,
        WorkflowRunStatus::Failed,
        None,
        Some("review failed".to_string()),
        Some(20),
        Some(30),
    );
    engine_repository
        .start_ready_nodes(
            &run_id,
            &[NodeRunToStart {
                id: WorkflowNodeRunId::new("nr-review"),
                scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                node_id: "review".to_string(),
                node_type: "agent".to_string(),
                input: None,
                iteration: None,
            }],
            40,
        )
        .unwrap();
    engine_repository
        .fail_node(
            &WorkflowNodeRunId::new("nr-review"),
            NodeFailure::new(NodeFailureKind::Session, "review failed"),
            ora_application::FailurePropagation::Run,
            50,
        )
        .unwrap();
    assert_eq!(
        engine_repository
            .resume_from_failure(&run_id, &["review".to_string()], 60)
            .unwrap(),
        ResumeWorkflowRunResult::Resumed
    );
    engine_repository
        .start_ready_nodes(
            &run_id,
            &[NodeRunToStart {
                id: WorkflowNodeRunId::new("nr-review-2"),
                scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                node_id: "review".to_string(),
                node_type: "agent".to_string(),
                input: None,
                iteration: None,
            }],
            70,
        )
        .unwrap();
    engine_repository
        .fail_node(
            &WorkflowNodeRunId::new("nr-review-2"),
            NodeFailure::new(NodeFailureKind::Session, "review failed again"),
            ora_application::FailurePropagation::Run,
            80,
        )
        .unwrap();

    let nodes = engine_repository.list_node_runs(&run_id).unwrap();
    let node = nodes.iter().find(|node| node.node_id == "review").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(node.payload.as_deref().unwrap()).unwrap();
    assert_eq!(parsed["error_detail"]["attempt"], 2);
    assert_eq!(parsed["error_detail"]["kind"], "session");
}

/// Soft-deleted failures are queryable; live rows and successes are not.
#[test]
fn find_last_failed_attempt_returns_the_latest_soft_deleted_failure() {
    let (temp_dir, pool) = bootstrapped_pool();
    let engine_repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let run_id = seed_run(
        &temp_dir,
        &pool,
        WorkflowRunStatus::Failed,
        None,
        Some("review failed".to_string()),
        Some(20),
        Some(30),
    );
    engine_repository
        .start_ready_nodes(
            &run_id,
            &[NodeRunToStart {
                id: WorkflowNodeRunId::new("nr-review"),
                scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                node_id: "review".to_string(),
                node_type: "agent".to_string(),
                input: None,
                iteration: None,
            }],
            40,
        )
        .unwrap();
    engine_repository
        .fail_node(
            &WorkflowNodeRunId::new("nr-review"),
            NodeFailure::new(NodeFailureKind::Session, "review failed"),
            ora_application::FailurePropagation::Run,
            50,
        )
        .unwrap();
    assert_eq!(
        engine_repository
            .resume_from_failure(&run_id, &["review".to_string()], 60)
            .unwrap(),
        ResumeWorkflowRunResult::Resumed
    );
    engine_repository
        .start_ready_nodes(
            &run_id,
            &[NodeRunToStart {
                id: WorkflowNodeRunId::new("nr-review-2"),
                scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                node_id: "review".to_string(),
                node_type: "agent".to_string(),
                input: None,
                iteration: None,
            }],
            70,
        )
        .unwrap();
    engine_repository
        .fail_node(
            &WorkflowNodeRunId::new("nr-review-2"),
            NodeFailure::new(NodeFailureKind::Session, "review failed again"),
            ora_application::FailurePropagation::Run,
            80,
        )
        .unwrap();
    assert_eq!(
        engine_repository
            .resume_from_failure(&run_id, &["review".to_string()], 90)
            .unwrap(),
        ResumeWorkflowRunResult::Resumed
    );

    let latest = engine_repository
        .find_last_failed_attempt(&run_id, "review", None)
        .unwrap()
        .expect("soft-deleted failure");
    let parsed: serde_json::Value =
        serde_json::from_str(latest.payload.as_deref().unwrap()).unwrap();
    assert_eq!(parsed["error_detail"]["attempt"], 2);
    assert_eq!(latest.audit_fields.is_deleted, true);

    engine_repository
        .start_ready_nodes(
            &run_id,
            &[
                NodeRunToStart {
                    id: WorkflowNodeRunId::new("nr-ok"),
                    scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                    node_id: "ok".to_string(),
                    node_type: "agent".to_string(),
                    input: None,
                    iteration: None,
                },
                NodeRunToStart {
                    id: WorkflowNodeRunId::new("nr-live"),
                    scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                    node_id: "live".to_string(),
                    node_type: "agent".to_string(),
                    input: None,
                    iteration: None,
                },
            ],
            100,
        )
        .unwrap();
    engine_repository
        .complete_node(
            &WorkflowNodeRunId::new("nr-ok"),
            Some("done".to_string()),
            None,
            Some("end_turn".to_string()),
            Vec::new(),
            110,
        )
        .unwrap();
    assert_eq!(
        engine_repository
            .find_last_failed_attempt(&run_id, "ok", None)
            .unwrap(),
        None
    );

    engine_repository
        .fail_node(
            &WorkflowNodeRunId::new("nr-live"),
            NodeFailure::new(NodeFailureKind::Session, "still live"),
            ora_application::FailurePropagation::Run,
            120,
        )
        .unwrap();
    assert_eq!(
        engine_repository
            .find_last_failed_attempt(&run_id, "live", None)
            .unwrap(),
        None
    );
}

/// Attempt numbers are scoped by `(run_id, node_id, iteration)`, so two rounds of the same
/// member node count independently.
#[test]
fn deleted_attempt_count_is_scoped_by_node_id_and_iteration() {
    let (temp_dir, pool) = bootstrapped_pool();
    let engine_repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let run_id = seed_run(
        &temp_dir,
        &pool,
        WorkflowRunStatus::Failed,
        None,
        Some("round failed".to_string()),
        Some(20),
        Some(30),
    );
    engine_repository
        .start_ready_nodes(
            &run_id,
            &[
                NodeRunToStart {
                    id: WorkflowNodeRunId::new("nr-fix-0"),
                    scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                    node_id: "fix".to_string(),
                    node_type: "agent".to_string(),
                    input: None,
                    iteration: Some(0),
                },
                NodeRunToStart {
                    id: WorkflowNodeRunId::new("nr-fix-1"),
                    scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                    node_id: "fix".to_string(),
                    node_type: "agent".to_string(),
                    input: None,
                    iteration: Some(1),
                },
            ],
            40,
        )
        .unwrap();
    engine_repository
        .fail_node(
            &WorkflowNodeRunId::new("nr-fix-0"),
            NodeFailure::new(NodeFailureKind::Session, "round 0 failed"),
            ora_application::FailurePropagation::Composite,
            50,
        )
        .unwrap();
    engine_repository
        .fail_node(
            &WorkflowNodeRunId::new("nr-fix-1"),
            NodeFailure::new(NodeFailureKind::Session, "round 1 failed"),
            ora_application::FailurePropagation::Composite,
            60,
        )
        .unwrap();

    let attempt = |node_run_id: &str| {
        let payload = engine_repository
            .list_node_runs(&run_id)
            .unwrap()
            .into_iter()
            .find(|row| row.id.as_ref() == node_run_id)
            .and_then(|row| row.payload)
            .expect("payload");
        serde_json::from_str::<serde_json::Value>(&payload).unwrap()["error_detail"]["attempt"]
            .clone()
    };
    assert_eq!(attempt("nr-fix-0"), serde_json::json!(1));
    assert_eq!(attempt("nr-fix-1"), serde_json::json!(1));

    // Soft-delete both rounds so the next live rows see them as prior attempts.
    engine_repository
        .resume_from_failure(&run_id, &["fix".to_string()], 70)
        .unwrap();
    engine_repository
        .start_ready_nodes(
            &run_id,
            &[
                NodeRunToStart {
                    id: WorkflowNodeRunId::new("nr-fix-0b"),
                    scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                    node_id: "fix".to_string(),
                    node_type: "agent".to_string(),
                    input: None,
                    iteration: Some(0),
                },
                NodeRunToStart {
                    id: WorkflowNodeRunId::new("nr-fix-1b"),
                    scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                    node_id: "fix".to_string(),
                    node_type: "agent".to_string(),
                    input: None,
                    iteration: Some(1),
                },
            ],
            80,
        )
        .unwrap();
    engine_repository
        .fail_node(
            &WorkflowNodeRunId::new("nr-fix-0b"),
            NodeFailure::new(NodeFailureKind::Session, "round 0 failed again"),
            ora_application::FailurePropagation::Composite,
            90,
        )
        .unwrap();
    engine_repository
        .fail_node(
            &WorkflowNodeRunId::new("nr-fix-1b"),
            NodeFailure::new(NodeFailureKind::Session, "round 1 failed again"),
            ora_application::FailurePropagation::Run,
            100,
        )
        .unwrap();

    let live = engine_repository.list_node_runs(&run_id).unwrap();
    let attempt_of = |id: &str| {
        let payload = live
            .iter()
            .find(|row| row.id.as_ref() == id)
            .and_then(|row| row.payload.as_deref())
            .expect("payload");
        serde_json::from_str::<serde_json::Value>(payload).unwrap()["error_detail"]["attempt"]
            .clone()
    };
    assert_eq!(attempt_of("nr-fix-0b"), serde_json::json!(2));
    assert_eq!(attempt_of("nr-fix-1b"), serde_json::json!(2));

    engine_repository
        .resume_from_failure(&run_id, &["fix".to_string()], 110)
        .unwrap();
    let round0 = engine_repository
        .find_last_failed_attempt(&run_id, "fix", Some(0))
        .unwrap()
        .expect("round 0");
    let round1 = engine_repository
        .find_last_failed_attempt(&run_id, "fix", Some(1))
        .unwrap()
        .expect("round 1");
    let parsed0: serde_json::Value =
        serde_json::from_str(round0.payload.as_deref().unwrap()).unwrap();
    let parsed1: serde_json::Value =
        serde_json::from_str(round1.payload.as_deref().unwrap()).unwrap();
    assert_eq!(parsed0["error_detail"]["attempt"], 2);
    assert_eq!(parsed1["error_detail"]["attempt"], 2);
    assert_eq!(round0.iteration, Some(0));
    assert_eq!(round1.iteration, Some(1));
    assert_eq!(
        engine_repository
            .find_last_failed_attempt(&run_id, "fix", None)
            .unwrap(),
        None
    );
}

/// `record_node_checkpoint` merges `checkpoint` into an existing payload and leaves other keys.
#[test]
fn record_node_checkpoint_merges_into_existing_payload() {
    let (temp_dir, pool) = bootstrapped_pool();
    let engine_repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let run_id = seed_pending_run(&temp_dir, &pool);
    engine_repository
        .start_ready_nodes(
            &run_id,
            &[NodeRunToStart {
                id: WorkflowNodeRunId::new("nr-review"),
                scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                node_id: "review".to_string(),
                node_type: "agent".to_string(),
                input: None,
                iteration: None,
            }],
            40,
        )
        .unwrap();
    pool.with_connection(|connection| {
        connection.execute(
            "UPDATE workflow_node_runs SET payload = ?2 WHERE id = ?1",
            rusqlite::params!["nr-review", r#"{"stop_reason":"end_turn"}"#],
        )?;
        Ok(())
    })
    .unwrap();
    engine_repository
        .record_node_checkpoint(
            &WorkflowNodeRunId::new("nr-review"),
            "snapshot-1",
            Some("abc123"),
            None,
            50,
        )
        .unwrap();

    let nodes = engine_repository.list_node_runs(&run_id).unwrap();
    let node = nodes.iter().find(|node| node.node_id == "review").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(node.payload.as_deref().unwrap()).unwrap();
    assert_eq!(parsed["stop_reason"], "end_turn");
    assert_eq!(parsed["checkpoint"], "abc123");
    assert_eq!(parsed["snapshot_id"], "snapshot-1");
    assert!(parsed.get("checkpoint_error").is_none());
}

/// `record_node_injected_failure` merges the rendered prompt block and keeps existing keys.
#[test]
fn record_node_injected_failure_merges_into_existing_payload() {
    let (temp_dir, pool) = bootstrapped_pool();
    let engine_repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let run_id = seed_pending_run(&temp_dir, &pool);
    engine_repository
        .start_ready_nodes(
            &run_id,
            &[NodeRunToStart {
                id: WorkflowNodeRunId::new("nr-review"),
                scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                node_id: "review".to_string(),
                node_type: "agent".to_string(),
                input: None,
                iteration: None,
            }],
            40,
        )
        .unwrap();
    pool.with_connection(|connection| {
        connection.execute(
            "UPDATE workflow_node_runs SET payload = ?2 WHERE id = ?1",
            rusqlite::params![
                "nr-review",
                r#"{"checkpoint":"abc123","snapshot_id":"snapshot-1"}"#
            ],
        )?;
        Ok(())
    })
    .unwrap();
    engine_repository
        .record_node_injected_failure(
            &WorkflowNodeRunId::new("nr-review"),
            "## 上一次尝试（第 1 次）失败信息\n...",
        )
        .unwrap();

    let nodes = engine_repository.list_node_runs(&run_id).unwrap();
    let node = nodes.iter().find(|node| node.node_id == "review").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(node.payload.as_deref().unwrap()).unwrap();
    assert_eq!(
        parsed,
        serde_json::json!({
            "checkpoint": "abc123",
            "snapshot_id": "snapshot-1",
            "injected_failure_context": "## 上一次尝试（第 1 次）失败信息\n...",
        })
    );
}

/// `record_node_ai_diagnosis` merges `ai_diagnosis` into an existing payload and overwrites only that key.
#[test]
fn record_node_ai_diagnosis_merges_into_existing_payload() {
    let (temp_dir, pool) = bootstrapped_pool();
    let engine_repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let run_id = seed_pending_run(&temp_dir, &pool);
    engine_repository
        .start_ready_nodes(
            &run_id,
            &[NodeRunToStart {
                id: WorkflowNodeRunId::new("nr-review"),
                scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                node_id: "review".to_string(),
                node_type: "agent".to_string(),
                input: None,
                iteration: None,
            }],
            40,
        )
        .unwrap();
    pool.with_connection(|connection| {
        connection.execute(
            "UPDATE workflow_node_runs SET payload = ?2 WHERE id = ?1",
            rusqlite::params![
                "nr-review",
                r#"{"error_detail":{"kind":"session","message":"boom"}}"#
            ],
        )?;
        Ok(())
    })
    .unwrap();
    engine_repository
        .record_node_ai_diagnosis(
            &WorkflowNodeRunId::new("nr-review"),
            r#"{"text":"first","agent_cli":"open_code","model":"m","generated_at":1}"#,
            50,
        )
        .unwrap();
    let nodes = engine_repository.list_node_runs(&run_id).unwrap();
    let node = nodes.iter().find(|node| node.node_id == "review").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(node.payload.as_deref().unwrap()).unwrap();
    assert_eq!(parsed["error_detail"]["kind"], "session");
    assert_eq!(parsed["ai_diagnosis"]["text"], "first");
    engine_repository
        .record_node_ai_diagnosis(
            &WorkflowNodeRunId::new("nr-review"),
            r#"{"text":"second","agent_cli":"open_code","model":"m","generated_at":2}"#,
            60,
        )
        .unwrap();
    let nodes = engine_repository.list_node_runs(&run_id).unwrap();
    let node = nodes.iter().find(|node| node.node_id == "review").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(node.payload.as_deref().unwrap()).unwrap();
    assert_eq!(parsed["error_detail"]["kind"], "session");
    assert_eq!(
        parsed["ai_diagnosis"],
        serde_json::json!({
            "text": "second",
            "agent_cli": "open_code",
            "model": "m",
            "generated_at": 2,
        })
    );
}

/// A failed snapshot writes `checkpoint: null` and `checkpoint_error` without dropping other keys.
#[test]
fn record_node_checkpoint_writes_null_and_error_when_snapshot_fails() {
    let (temp_dir, pool) = bootstrapped_pool();
    let engine_repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let run_id = seed_pending_run(&temp_dir, &pool);
    engine_repository
        .start_ready_nodes(
            &run_id,
            &[NodeRunToStart {
                id: WorkflowNodeRunId::new("nr-review"),
                scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                node_id: "review".to_string(),
                node_type: "agent".to_string(),
                input: None,
                iteration: None,
            }],
            40,
        )
        .unwrap();
    pool.with_connection(|connection| {
        connection.execute(
            "UPDATE workflow_node_runs SET payload = ?2 WHERE id = ?1",
            rusqlite::params!["nr-review", r#"{"stop_reason":"end_turn"}"#],
        )?;
        Ok(())
    })
    .unwrap();
    engine_repository
        .record_node_checkpoint(
            &WorkflowNodeRunId::new("nr-review"),
            "snapshot-1",
            None,
            Some("not a git repository"),
            50,
        )
        .unwrap();

    let nodes = engine_repository.list_node_runs(&run_id).unwrap();
    let node = nodes.iter().find(|node| node.node_id == "review").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(node.payload.as_deref().unwrap()).unwrap();
    assert_eq!(parsed["stop_reason"], "end_turn");
    assert_eq!(parsed["checkpoint"], serde_json::Value::Null);
    assert_eq!(parsed["checkpoint_error"], "not a git repository");
}

/// `fail_node` writes `payload.file_changes` in the same shape as a succeeded node.
#[test]
fn fail_node_writes_file_changes_in_the_same_shape_as_complete_node() {
    let (temp_dir, pool) = bootstrapped_pool();
    let engine_repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let run_id = seed_pending_run(&temp_dir, &pool);
    engine_repository
        .start_ready_nodes(
            &run_id,
            &[
                NodeRunToStart {
                    id: WorkflowNodeRunId::new("nr-ok"),
                    scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                    node_id: "ok".to_string(),
                    node_type: "agent".to_string(),
                    input: None,
                    iteration: None,
                },
                NodeRunToStart {
                    id: WorkflowNodeRunId::new("nr-fail"),
                    scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                    node_id: "fail".to_string(),
                    node_type: "agent".to_string(),
                    input: None,
                    iteration: None,
                },
            ],
            40,
        )
        .unwrap();
    engine_repository
        .record_node_checkpoint(
            &WorkflowNodeRunId::new("nr-fail"),
            "snapshot-1",
            Some("def456"),
            None,
            45,
        )
        .unwrap();
    let file_changes = vec![
        FileChange {
            path: "src/a.ts".to_string(),
            additions: 3,
            deletions: 1,
        },
        FileChange {
            path: "src/b.ts".to_string(),
            additions: 0,
            deletions: 2,
        },
    ];
    engine_repository
        .complete_node(
            &WorkflowNodeRunId::new("nr-ok"),
            Some("done".to_string()),
            None,
            Some("end_turn".to_string()),
            file_changes.clone(),
            50,
        )
        .unwrap();
    engine_repository
        .fail_node(
            &WorkflowNodeRunId::new("nr-fail"),
            NodeFailure::new(NodeFailureKind::Session, "review failed")
                .with_file_changes(file_changes),
            ora_application::FailurePropagation::Run,
            60,
        )
        .unwrap();

    let nodes = engine_repository.list_node_runs(&run_id).unwrap();
    let succeeded = nodes.iter().find(|node| node.node_id == "ok").unwrap();
    let failed = nodes.iter().find(|node| node.node_id == "fail").unwrap();
    let succeeded_payload: serde_json::Value =
        serde_json::from_str(succeeded.payload.as_deref().unwrap()).unwrap();
    let failed_payload: serde_json::Value =
        serde_json::from_str(failed.payload.as_deref().unwrap()).unwrap();
    println!("payload={}", failed.payload.as_deref().unwrap());
    assert_eq!(
        failed_payload["file_changes"],
        succeeded_payload["file_changes"]
    );
    assert_eq!(
        failed_payload["file_changes"],
        serde_json::json!([
            {"path": "src/a.ts", "additions": 3, "deletions": 1},
            {"path": "src/b.ts", "additions": 0, "deletions": 2},
        ])
    );
    assert_eq!(failed_payload["checkpoint"], "def456");
}

/// `complete_node` keeps checkpoint keys when merging stop_reason and file_changes.
#[test]
fn complete_node_merges_stop_reason_into_existing_checkpoint_payload() {
    let (temp_dir, pool) = bootstrapped_pool();
    let engine_repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let run_id = seed_pending_run(&temp_dir, &pool);
    engine_repository
        .start_ready_nodes(
            &run_id,
            &[NodeRunToStart {
                id: WorkflowNodeRunId::new("nr-review"),
                scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                node_id: "review".to_string(),
                node_type: "agent".to_string(),
                input: None,
                iteration: None,
            }],
            40,
        )
        .unwrap();
    engine_repository
        .record_node_checkpoint(
            &WorkflowNodeRunId::new("nr-review"),
            "snapshot-1",
            Some("abc123"),
            None,
            50,
        )
        .unwrap();
    engine_repository
        .complete_node(
            &WorkflowNodeRunId::new("nr-review"),
            Some("done".to_string()),
            None,
            Some("end_turn".to_string()),
            vec![FileChange {
                path: "src/a.ts".to_string(),
                additions: 1,
                deletions: 0,
            }],
            60,
        )
        .unwrap();

    let nodes = engine_repository.list_node_runs(&run_id).unwrap();
    let node = nodes.iter().find(|node| node.node_id == "review").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(node.payload.as_deref().unwrap()).unwrap();
    assert_eq!(
        parsed,
        serde_json::json!({
            "checkpoint": "abc123",
            "snapshot_id": "snapshot-1",
            "stop_reason": "end_turn",
            "file_changes": [{"path": "src/a.ts", "additions": 1, "deletions": 0}],
        })
    );
}

/// Crash recovery writes `interrupted_by_restart` error detail on orphaned in-flight nodes.
#[test]
fn fail_orphaned_node_runs_writes_interrupted_by_restart_error_detail() {
    let (temp_dir, pool) = bootstrapped_pool();
    let engine_repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let run_repository = SqliteWorkflowRunRepository::new(pool.clone());
    let run_id = seed_pending_run(&temp_dir, &pool);
    engine_repository
        .start_run(
            &run_id,
            &NodeRunToStart {
                id: WorkflowNodeRunId::new("nr-start"),
                scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                node_id: "start".to_string(),
                node_type: "start".to_string(),
                input: None,
                iteration: None,
            },
            40,
        )
        .unwrap();
    engine_repository
        .fail_orphaned_node_runs(&[run_id.clone()], 80)
        .unwrap();

    let run = run_repository.find_run(&run_id).unwrap().unwrap();
    assert_eq!(run.status, WorkflowRunStatus::Failed);
    assert_eq!(
        run.error.as_deref(),
        Some(r#"{"reason":"interrupted_by_restart"}"#)
    );
    let nodes = engine_repository.list_node_runs(&run_id).unwrap();
    let node = nodes.iter().find(|node| node.node_id == "start").unwrap();
    assert_eq!(node.status, WorkflowNodeStatus::Failed);
    assert_eq!(
        node.error.as_deref(),
        Some(r#"{"reason":"interrupted_by_restart"}"#)
    );
    let parsed: serde_json::Value = serde_json::from_str(node.payload.as_deref().unwrap()).unwrap();
    assert_eq!(parsed["error_detail"]["kind"], "interrupted_by_restart");
    assert_eq!(parsed["error_detail"]["resumable"], true);
    assert_eq!(parsed["error_detail"]["injects_previous_failure"], false);
    assert_eq!(parsed["error_detail"]["attempt"], 1);
    assert_eq!(parsed["error_detail"]["recorded_at"], 80);
}

/// Verifies deleting a run preserves the workspace and its independent session aggregate.
#[test]
fn deleting_workflow_run_does_not_delete_workspace_or_session() {
    let (temp_dir, pool) = bootstrapped_pool();
    let workspace_path = existing_workspace_path(&temp_dir);
    let project_repository =
        SqliteProjectRepository::with_clock(pool.clone(), crate::test_clock::TestClock::new(1));
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
        ora_domain::SessionMcpSelection::Automatic,
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

/// Verifies a created-but-never-started Pending run can be discarded.
#[test]
fn not_started_pending_run_can_be_deleted() {
    let (temp_dir, pool) = bootstrapped_pool();
    let run_repository = SqliteWorkflowRunRepository::new(pool.clone());
    let run_id = seed_pending_run(&temp_dir, &pool);

    assert_eq!(
        run_repository.soft_delete_run(&run_id, 40).unwrap(),
        ora_application::DeleteWorkflowRunResult::Deleted,
    );
    assert_eq!(run_repository.find_run(&run_id).unwrap(), None);
}

/// Verifies an executing run stays protected until it reaches a terminal status.
#[test]
fn running_run_cannot_be_deleted() {
    let (temp_dir, pool) = bootstrapped_pool();
    let run_repository = SqliteWorkflowRunRepository::new(pool.clone());
    let engine_repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let run_id = seed_pending_run(&temp_dir, &pool);

    assert_eq!(
        engine_repository
            .start_run(
                &run_id,
                &NodeRunToStart {
                    id: WorkflowNodeRunId::new("node-run-1"),
                    scope_id: ora_domain::WorkflowScopeId::new(format!("root:{run_id}")),
                    node_id: "agent-1".to_string(),
                    node_type: "agent".to_string(),
                    input: None,
                    iteration: None,
                },
                40,
            )
            .unwrap(),
        StartWorkflowRunResult::Started
    );
    assert_eq!(
        run_repository.soft_delete_run(&run_id, 50).unwrap(),
        ora_application::DeleteWorkflowRunResult::ActiveRun,
    );
}

/// Verifies the library lists newest workflows first so a just-created row is on top.
#[test]
fn list_workflows_returns_newest_first() {
    let (_temp_dir, pool) = bootstrapped_pool();
    let workflow_repository = SqliteWorkflowRepository::new(pool);
    for (id, created_at) in [("workflow-old", 10i64), ("workflow-new", 20i64)] {
        workflow_repository
            .create_workflow(
                Workflow::new(
                    WorkflowId::new(id),
                    Namespace::local(),
                    id,
                    None,
                    AuditFields::new(created_at, created_at, false),
                )
                .unwrap(),
                WorkflowSnapshot::new(
                    WorkflowSnapshotId::new(format!("snap-{id}")),
                    WorkflowId::new(id),
                    "draft",
                    "{}",
                    created_at,
                    Some(created_at),
                    false,
                ),
            )
            .unwrap();
    }
    let listed = workflow_repository.list_workflows().unwrap();
    assert_eq!(
        listed
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec!["workflow-new", "workflow-old"],
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

/// Seeds a run with an explicit status and payload for engine-repository fixtures.
fn seed_run(
    temp_dir: &TempDir,
    pool: &RepositoryPool,
    status: WorkflowRunStatus,
    payload: Option<String>,
    error: Option<String>,
    started_at: Option<i64>,
    finished_at: Option<i64>,
) -> WorkflowRunId {
    let workspace_path = existing_workspace_path(temp_dir);
    let project_repository =
        SqliteProjectRepository::with_clock(pool.clone(), crate::test_clock::TestClock::new(1));
    let workspace_repository = SqliteWorkspaceRepository::new(pool.clone());
    let workflow_repository = SqliteWorkflowRepository::new(pool.clone());
    let run_repository = SqliteWorkflowRunRepository::new(pool.clone());
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
    let run_id = WorkflowRunId::new("run-1");
    run_repository
        .create_run(WorkflowRun::new(
            run_id.clone(),
            workspace.id,
            workflow_id,
            snapshot_id,
            "Review run",
            status,
            Some(r#"{"current_nodes":[]}"#.to_string()),
            Some("新任务".to_string()),
            None,
            error,
            payload,
            started_at,
            finished_at,
            AuditFields::new(20, 30, false),
        ))
        .unwrap();
    run_id
}

/// Creates a workspace session owned by the given run's workspace so bind can succeed.
fn create_run_workspace_session(pool: &RepositoryPool, run_id: &WorkflowRunId) -> SessionId {
    let run = SqliteWorkflowRunRepository::new(pool.clone())
        .find_run(run_id)
        .unwrap()
        .unwrap();
    let session_id = SessionId::new("session-bind");
    SqliteSessionRepository::new(pool.clone())
        .create_session(Session::new(
            session_id.clone(),
            run.workspace_id,
            AgentRef::parse("ora-space.opencode").unwrap(),
            "provider-bind",
            SessionStatus::Running,
            ora_domain::SessionMcpSelection::Automatic,
            AuditFields::new(20, 20, false),
        ))
        .unwrap();
    session_id
}

/// Seeds a created-but-never-started Pending run for deletion-policy fixtures.
fn seed_pending_run(temp_dir: &TempDir, pool: &RepositoryPool) -> WorkflowRunId {
    let workspace_path = existing_workspace_path(temp_dir);
    let project_repository =
        SqliteProjectRepository::with_clock(pool.clone(), crate::test_clock::TestClock::new(1));
    let workspace_repository = SqliteWorkspaceRepository::new(pool.clone());
    let workflow_repository = SqliteWorkflowRepository::new(pool.clone());
    let run_repository = SqliteWorkflowRunRepository::new(pool.clone());
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
    let run_id = WorkflowRunId::new("run-1");
    run_repository
        .create_run(WorkflowRun::new(
            run_id.clone(),
            workspace.id,
            workflow_id,
            snapshot_id,
            "Review run",
            WorkflowRunStatus::Pending,
            Some(r#"{"current_nodes":[]}"#.to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
            ora_domain::AuditFields::new(20, 20, false),
        ))
        .unwrap();
    run_id
}

/// Creates the existing local directory required for an admitted main Workspace fixture.
fn existing_workspace_path(temp_dir: &TempDir) -> std::path::PathBuf {
    let path = temp_dir.path().join("repository");
    std::fs::create_dir_all(&path).expect("create workspace directory");
    path
}
