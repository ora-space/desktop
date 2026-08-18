use std::{
    cell::Cell,
    path::PathBuf,
    sync::{Arc, Barrier, Mutex},
    thread,
};

use ora_application::{
    ActivateVersionResult, AdvanceWorkflowRunResult, AgentDefinitionRepository,
    CancelWorkflowRunResult, Clock, DeleteSnapshotResult, DeleteWorkflowResult,
    DeleteWorkflowRunResult, EngineError, ExecutionContext, NodeExecutor, NodeRunToStart, NodeType,
    ProjectRepository, PublishSnapshotResult, RepositoryError, RestartWorkflowRunResult,
    RollbackDraftResult, SessionRepository, SkillRepository, StartWorkflowRunResult,
    TaskRepository, UpdateWorkflowRunInputResult, WorkflowGraphNode, WorkflowNodeRunIdGenerator,
    WorkflowRepository, WorkflowRunControlHandler, WorkflowRunCreateOutcome, WorkflowRunEngine,
    WorkflowRunEngineRepository, WorkflowRunRepository, WorkflowValidationError,
    WorktreeRepository,
};
use ora_contracts::{StartWorkflowRunRequest, WorkflowRunStatus as ContractRunStatus};
use ora_domain::{
    AgentCli, AgentDefinition, AgentDefinitionId, AuditFields, HistoryState, Namespace, Project,
    ProjectId, Session, SessionId, SessionStatus, SessionTitle, Skill, SkillId, Task, TaskId,
    Workflow, WorkflowId, WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRun, WorkflowRunDetail,
    WorkflowRunId, WorkflowRunStatus, WorkflowRunSummary, WorkflowSnapshot, WorkflowSnapshotId,
    Worktree, WorktreeActivity, WorktreeBaseline, WorktreeId, WorktreeProvisioningLeaseId,
};
use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;
use tempfile::TempDir;

use crate::{
    CascadeDeleteOutcome, DatabaseBootstrapper, DatabaseError, DatabaseLocation, RepositoryPool,
    SqliteAgentDefinitionRepository, SqliteCascadeRepository, SqliteProjectRepository,
    SqliteSessionRepository, SqliteSkillRepository, SqliteTaskRepository, SqliteWorkflowRepository,
    SqliteWorkflowRunEngineRepository, SqliteWorkflowRunRepository, SqliteWorktreeRepository,
    TimestampSource, default_migration_catalog,
};

/// Verifies catalog repositories scope duplicate names by namespace and hide soft-deleted rows.
#[test]
fn catalog_repositories_support_id_based_crud_and_namespaced_names() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let skill_repository = SqliteSkillRepository::new(pool.clone());
    let agent_repository = SqliteAgentDefinitionRepository::new(pool);
    let created_skill = skill("skill-1", "review", "Reviews changes", 1, 1, false);
    let created_agent = agent("agent-1", "opencode", "OpenCode", 1, 1, false);

    assert_eq!(
        skill_repository
            .create_skill(created_skill.clone())
            .unwrap(),
        created_skill.clone()
    );
    assert_eq!(
        agent_repository
            .create_agent_definition(created_agent.clone())
            .unwrap(),
        created_agent.clone()
    );
    let mut earlier_skill = skill("skill-0", "review", "Builds", 0, 0, false);
    earlier_skill.namespace = Namespace::new("ora.plugin").unwrap();
    let mut earlier_agent = agent("agent-0", "opencode", "Assists", 0, 0, false);
    earlier_agent.namespace = Namespace::new("ora.plugin").unwrap();
    skill_repository
        .create_skill(earlier_skill.clone())
        .unwrap();
    agent_repository
        .create_agent_definition(earlier_agent.clone())
        .unwrap();
    assert_eq!(
        skill_repository.list_skills().unwrap(),
        vec![earlier_skill.clone(), created_skill.clone()]
    );
    assert_eq!(
        agent_repository.list_agent_definitions().unwrap(),
        vec![earlier_agent.clone(), created_agent.clone()]
    );
    assert_eq!(
        skill_repository
            .find_skill_by_name(&Namespace::local(), "REVIEW")
            .unwrap(),
        Some(created_skill.clone())
    );
    assert_eq!(
        skill_repository
            .find_skill_by_name(&earlier_skill.namespace, "REVIEW")
            .unwrap(),
        Some(earlier_skill.clone())
    );
    let renamed_skill = skill("skill-1", "reviewer", "Reviews code", 1, 2, false);
    let renamed_agent = agent("agent-1", "reviewer-agent", "Reviews code", 1, 2, false);
    assert_eq!(
        skill_repository
            .update_skill(renamed_skill.clone())
            .unwrap(),
        renamed_skill.clone()
    );
    assert_eq!(
        agent_repository
            .update_agent_definition(renamed_agent.clone())
            .unwrap(),
        renamed_agent.clone()
    );
    assert_eq!(
        skill_repository
            .soft_delete_skill(&SkillId::new("skill-1"), 3)
            .unwrap(),
        true
    );
    assert_eq!(
        agent_repository
            .soft_delete_agent_definition(&AgentDefinitionId::new("agent-1"), 3)
            .unwrap(),
        true
    );
    assert_eq!(
        skill_repository
            .find_skill(&SkillId::new("skill-1"))
            .unwrap(),
        None
    );
    assert_eq!(
        agent_repository
            .find_agent_definition(&AgentDefinitionId::new("agent-1"))
            .unwrap(),
        None
    );
    assert_eq!(
        skill_repository
            .soft_delete_skill(&SkillId::new("missing"), 4)
            .unwrap(),
        false
    );
    assert_eq!(
        agent_repository
            .soft_delete_agent_definition(&AgentDefinitionId::new("missing"), 4)
            .unwrap(),
        false
    );
}

/// Verifies workflow names are unique case-insensitively within each visible namespace.
#[test]
fn workflow_repository_scopes_visible_name_uniqueness_by_namespace() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteWorkflowRepository::new(pool);
    let (mut local, local_draft) = workflow_with_draft("local-workflow", "{}", 1);
    local.name = "Review".to_string();
    repository
        .create_workflow(local.clone(), local_draft)
        .unwrap();

    let (mut duplicate, duplicate_draft) = workflow_with_draft("duplicate", "{}", 2);
    duplicate.name = "REVIEW".to_string();
    assert!(
        repository
            .create_workflow(duplicate, duplicate_draft)
            .is_err()
    );

    let (mut plugin, plugin_draft) = workflow_with_draft("plugin-workflow", "{}", 3);
    plugin.namespace = Namespace::new("ora.plugin").unwrap();
    plugin.name = "review".to_string();
    repository
        .create_workflow(plugin.clone(), plugin_draft)
        .unwrap();
    assert_eq!(
        repository
            .find_workflow_by_name(&plugin.namespace, "REVIEW")
            .unwrap(),
        Some(plugin)
    );

    assert_eq!(
        repository.soft_delete_workflow(&local.id, 4).unwrap(),
        DeleteWorkflowResult::Deleted
    );
    let (mut replacement, replacement_draft) = workflow_with_draft("replacement", "{}", 5);
    replacement.name = "review".to_string();
    repository
        .create_workflow(replacement, replacement_draft)
        .unwrap();
}

/// Verifies lifecycle commands cannot use another workflow's snapshot as their source.
#[test]
fn workflow_repository_rejects_cross_workflow_lifecycle_targets() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteWorkflowRepository::new(pool);
    let (workflow_a, draft_a) = workflow_with_draft("workflow-a", "{\"nodes\":[]}", 10);
    let (workflow_b, draft_b) = workflow_with_draft("workflow-b", "{\"nodes\":[1]}", 20);
    repository
        .create_workflow(workflow_a.clone(), draft_a.clone())
        .unwrap();
    repository
        .create_workflow(workflow_b.clone(), draft_b.clone())
        .unwrap();

    let snapshot_b = published_snapshot("snapshot-b", &workflow_b.id, "v1", &draft_b.graph, 30);
    assert_eq!(
        repository
            .publish_snapshot(
                &workflow_b.id,
                snapshot_b.id.clone(),
                snapshot_b.version.clone(),
                snapshot_b.created_at,
            )
            .unwrap(),
        PublishSnapshotResult::Published(snapshot_b.clone())
    );
    assert_eq!(
        repository
            .activate_version(&workflow_b.id, &snapshot_b.id, 40)
            .unwrap(),
        ActivateVersionResult::Activated(WorkflowSnapshot::new(
            draft_b.id.clone(),
            workflow_b.id.clone(),
            "draft",
            snapshot_b.graph.clone(),
            20,
            Some(40),
            /*is_deleted*/ false,
        ))
    );
    assert_eq!(
        repository
            .find_workflow(&workflow_b.id)
            .unwrap()
            .expect("workflow B remains visible"),
        Workflow::new(
            workflow_b.id.clone(),
            Namespace::local(),
            "Workflow workflow-b",
            Some(snapshot_b.id.clone()),
            AuditFields::new(20, 40, /*is_deleted*/ false),
        )
        .unwrap()
    );

    assert_eq!(
        repository
            .rollback_draft(&workflow_a.id, &snapshot_b.id, 40)
            .unwrap(),
        RollbackDraftResult::SnapshotNotFound
    );
    assert_eq!(
        repository
            .activate_version(&workflow_a.id, &snapshot_b.id, 40)
            .unwrap(),
        ActivateVersionResult::SnapshotNotFound
    );
    assert_eq!(
        repository
            .find_snapshot_by_version(&workflow_a.id, "draft")
            .unwrap(),
        Some(draft_a)
    );
    assert_eq!(
        repository
            .find_workflow(&workflow_a.id)
            .unwrap()
            .expect("workflow A remains visible"),
        workflow_a
    );
}

/// Verifies a visible version name can be reused after its previous snapshot is soft-deleted.
#[test]
fn workflow_repository_reuses_soft_deleted_version_names() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteWorkflowRepository::new(pool);
    let (workflow, draft) = workflow_with_draft("workflow-a", "{}", 10);
    repository
        .create_workflow(workflow.clone(), draft.clone())
        .unwrap();

    let first = published_snapshot("snapshot-1", &workflow.id, "v1", &draft.graph, 20);
    assert_eq!(
        repository
            .publish_snapshot(
                &workflow.id,
                first.id.clone(),
                first.version.clone(),
                first.created_at,
            )
            .unwrap(),
        PublishSnapshotResult::Published(first.clone())
    );
    assert_eq!(
        repository
            .soft_delete_snapshot(&workflow.id, &first.id, 30)
            .unwrap(),
        DeleteSnapshotResult::ActiveSnapshot
    );

    let second = published_snapshot("snapshot-2", &workflow.id, "v2", &draft.graph, 40);
    repository
        .publish_snapshot(
            &workflow.id,
            second.id.clone(),
            second.version.clone(),
            second.created_at,
        )
        .unwrap();
    assert_eq!(
        repository
            .soft_delete_snapshot(&workflow.id, &first.id, 50)
            .unwrap(),
        DeleteSnapshotResult::Deleted(first)
    );

    let replacement = published_snapshot("snapshot-3", &workflow.id, "v1", &draft.graph, 60);
    assert_eq!(
        repository
            .publish_snapshot(
                &workflow.id,
                replacement.id.clone(),
                replacement.version.clone(),
                replacement.created_at,
            )
            .unwrap(),
        PublishSnapshotResult::Published(replacement)
    );
}

/// Verifies soft deletion never changes the edit timestamp of an immutable published snapshot.
#[test]
fn workflow_repository_preserves_published_snapshot_timestamps_when_soft_deleted() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteWorkflowRepository::new(pool.clone());
    let (workflow, draft) = workflow_with_draft("workflow-a", "{}", 10);
    repository
        .create_workflow(workflow.clone(), draft.clone())
        .unwrap();

    let first = published_snapshot("snapshot-1", &workflow.id, "v1", &draft.graph, 20);
    let second = published_snapshot("snapshot-2", &workflow.id, "v2", &draft.graph, 30);
    repository
        .publish_snapshot(
            &workflow.id,
            first.id.clone(),
            first.version.clone(),
            first.created_at,
        )
        .unwrap();
    repository
        .publish_snapshot(
            &workflow.id,
            second.id.clone(),
            second.version.clone(),
            second.created_at,
        )
        .unwrap();
    repository
        .soft_delete_snapshot(&workflow.id, &first.id, /*deleted_at*/ 40)
        .unwrap();

    let (cascade_workflow, cascade_draft) = workflow_with_draft("workflow-b", "{}", 50);
    repository
        .create_workflow(cascade_workflow.clone(), cascade_draft.clone())
        .unwrap();
    let cascade_snapshot = published_snapshot(
        "snapshot-3",
        &cascade_workflow.id,
        "v1",
        &cascade_draft.graph,
        60,
    );
    repository
        .publish_snapshot(
            &cascade_workflow.id,
            cascade_snapshot.id.clone(),
            cascade_snapshot.version.clone(),
            cascade_snapshot.created_at,
        )
        .unwrap();
    assert_eq!(
        repository
            .soft_delete_workflow(&cascade_workflow.id, /*deleted_at*/ 70)
            .unwrap(),
        DeleteWorkflowResult::Deleted
    );

    let timestamps = pool
        .with_connection(|connection| {
            let direct = connection.query_row(
                "SELECT updated_at FROM workflow_snapshots WHERE id = ?1",
                rusqlite::params![first.id.as_ref()],
                |row| row.get::<_, Option<i64>>(0),
            )?;
            let cascade = connection.query_row(
                "SELECT updated_at FROM workflow_snapshots WHERE id = ?1",
                rusqlite::params![cascade_snapshot.id.as_ref()],
                |row| row.get::<_, Option<i64>>(0),
            )?;

            Ok((direct, cascade))
        })
        .unwrap();

    assert_eq!(timestamps, (None, None));
}

/// Verifies publishing an active version name reports a business conflict instead of a database error.
#[test]
fn workflow_repository_reports_active_version_conflicts() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteWorkflowRepository::new(pool);
    let (workflow, draft) = workflow_with_draft("workflow-a", "{}", 10);
    repository.create_workflow(workflow.clone(), draft).unwrap();

    let first = published_snapshot("snapshot-1", &workflow.id, "v1", "{\"nodes\":[1]}", 20);
    repository
        .publish_snapshot(
            &workflow.id,
            first.id.clone(),
            first.version.clone(),
            first.created_at,
        )
        .unwrap();
    let duplicate = published_snapshot("snapshot-2", &workflow.id, "v1", "{\"nodes\":[2]}", 30);

    assert_eq!(
        repository
            .publish_snapshot(
                &workflow.id,
                duplicate.id,
                duplicate.version,
                duplicate.created_at,
            )
            .unwrap(),
        PublishSnapshotResult::VersionAlreadyExists
    );
}

/// Verifies concurrent publishers serialize through SQLite and expose one deterministic conflict.
#[test]
fn workflow_repository_serializes_concurrent_version_conflicts() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteWorkflowRepository::new(pool);
    let (workflow, draft) = workflow_with_draft("workflow-a", "{}", 10);
    repository.create_workflow(workflow.clone(), draft).unwrap();

    let barrier = Arc::new(Barrier::new(2));
    let first_repository = repository.clone();
    let second_repository = repository.clone();
    let first_workflow_id = workflow.id.clone();
    let second_workflow_id = workflow.id.clone();
    let first_barrier = barrier.clone();
    let second_barrier = barrier.clone();

    let (first, second) = thread::scope(|scope| {
        let first = scope.spawn(move || {
            first_barrier.wait();
            first_repository.publish_snapshot(
                &first_workflow_id,
                WorkflowSnapshotId::new("snapshot-1"),
                "v1".to_string(),
                20,
            )
        });
        let second = scope.spawn(move || {
            second_barrier.wait();
            second_repository.publish_snapshot(
                &second_workflow_id,
                WorkflowSnapshotId::new("snapshot-2"),
                "v1".to_string(),
                20,
            )
        });

        (
            first.join().unwrap().unwrap(),
            second.join().unwrap().unwrap(),
        )
    });

    let published_count = usize::from(matches!(&first, PublishSnapshotResult::Published(_)))
        + usize::from(matches!(&second, PublishSnapshotResult::Published(_)));
    let conflict_count = usize::from(matches!(
        &first,
        PublishSnapshotResult::VersionAlreadyExists
    )) + usize::from(matches!(
        &second,
        PublishSnapshotResult::VersionAlreadyExists
    ));
    assert_eq!((published_count, conflict_count), (1, 1));
    assert_eq!(
        repository.list_versions(&workflow.id).unwrap(),
        vec![ora_domain::WorkflowVersion {
            id: match (first, second) {
                (PublishSnapshotResult::Published(snapshot), _)
                | (_, PublishSnapshotResult::Published(snapshot)) => snapshot.id.to_string(),
                _ => unreachable!("one concurrent publisher must succeed"),
            },
            version: "v1".to_string(),
            created_at: 20,
        }]
    );
}

/// Verifies a snapshot resolves by identifier only within its owning workflow.
#[test]
fn workflow_repository_finds_snapshot_by_id_within_workflow() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteWorkflowRepository::new(pool);
    let (workflow, draft) = workflow_with_draft("workflow-a", "{\"nodes\":[]}", 10);
    repository
        .create_workflow(workflow.clone(), draft.clone())
        .unwrap();
    let snapshot = published_snapshot("snapshot-a", &workflow.id, "v1", &draft.graph, 20);
    repository
        .publish_snapshot(
            &workflow.id,
            snapshot.id.clone(),
            snapshot.version.clone(),
            snapshot.created_at,
        )
        .unwrap();

    assert_eq!(
        repository
            .find_snapshot_by_id(&workflow.id, &snapshot.id)
            .unwrap(),
        Some(snapshot.clone())
    );
    // A snapshot belonging to another workflow must not resolve under this workflow's scope.
    assert_eq!(
        repository
            .find_snapshot_by_id(&WorkflowId::new("workflow-other"), &snapshot.id)
            .unwrap(),
        None
    );
    // The snapshot must resolve by id alone, independent of its workflow, for run read models.
    assert_eq!(
        repository.find_snapshot_any_workflow(&snapshot.id).unwrap(),
        Some(snapshot.clone())
    );
    assert_eq!(
        repository
            .find_snapshot_any_workflow(&WorkflowSnapshotId::new("snapshot-missing"))
            .unwrap(),
        None
    );
}

/// Verifies a published snapshot referenced by a live run cannot be soft-deleted.
#[test]
fn workflow_repository_rejects_deleting_snapshot_referenced_by_live_run() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteWorkflowRepository::new(pool.clone());
    let (workflow, draft) = workflow_with_draft("workflow-a", "{\"nodes\":[]}", 10);
    repository
        .create_workflow(workflow.clone(), draft.clone())
        .unwrap();
    let snapshot = published_snapshot("snapshot-a", &workflow.id, "v1", &draft.graph, 20);
    repository
        .publish_snapshot(
            &workflow.id,
            snapshot.id.clone(),
            snapshot.version.clone(),
            snapshot.created_at,
        )
        .unwrap();
    // Publishing a second snapshot moves the active pointer off `snapshot`, so the
    // run-reference guard (not the active-version guard) decides its deletion.
    let newer = published_snapshot("snapshot-b", &workflow.id, "v2", &draft.graph, 25);
    repository
        .publish_snapshot(&workflow.id, newer.id, newer.version, newer.created_at)
        .unwrap();

    insert_run_referencing_snapshot(&pool, "run-1", &workflow.id, &snapshot.id, false);

    assert_eq!(
        repository
            .soft_delete_snapshot(&workflow.id, &snapshot.id, 30)
            .unwrap(),
        DeleteSnapshotResult::SnapshotInUse
    );
}

/// Verifies a snapshot referenced only by a soft-deleted run remains deletable.
#[test]
fn workflow_repository_deletes_snapshot_referenced_only_by_soft_deleted_run() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteWorkflowRepository::new(pool.clone());
    let (workflow, draft) = workflow_with_draft("workflow-a", "{\"nodes\":[]}", 10);
    repository
        .create_workflow(workflow.clone(), draft.clone())
        .unwrap();
    let snapshot = published_snapshot("snapshot-a", &workflow.id, "v1", &draft.graph, 20);
    repository
        .publish_snapshot(
            &workflow.id,
            snapshot.id.clone(),
            snapshot.version.clone(),
            snapshot.created_at,
        )
        .unwrap();
    let newer = published_snapshot("snapshot-b", &workflow.id, "v2", &draft.graph, 25);
    repository
        .publish_snapshot(&workflow.id, newer.id, newer.version, newer.created_at)
        .unwrap();

    insert_run_referencing_snapshot(&pool, "run-1", &workflow.id, &snapshot.id, true);

    assert_eq!(
        repository
            .soft_delete_snapshot(&workflow.id, &snapshot.id, 30)
            .unwrap(),
        DeleteSnapshotResult::Deleted(snapshot)
    );
}

/// Verifies the draft and active-version guards run before the run-reference guard.
#[test]
fn workflow_repository_snapshot_in_use_guard_yields_to_draft_and_active() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteWorkflowRepository::new(pool.clone());
    let (workflow, draft) = workflow_with_draft("workflow-a", "{\"nodes\":[]}", 10);
    repository
        .create_workflow(workflow.clone(), draft.clone())
        .unwrap();
    let snapshot = published_snapshot("snapshot-a", &workflow.id, "v1", &draft.graph, 20);
    repository
        .publish_snapshot(
            &workflow.id,
            snapshot.id.clone(),
            snapshot.version.clone(),
            snapshot.created_at,
        )
        .unwrap();
    repository
        .activate_version(&workflow.id, &snapshot.id, 25)
        .unwrap();
    insert_run_referencing_snapshot(&pool, "run-1", &workflow.id, &snapshot.id, false);

    // The active-version guard takes precedence over the run-reference guard.
    assert_eq!(
        repository
            .soft_delete_snapshot(&workflow.id, &snapshot.id, 30)
            .unwrap(),
        DeleteSnapshotResult::ActiveSnapshot
    );
    // The draft guard also takes precedence regardless of run references.
    assert_eq!(
        repository
            .soft_delete_snapshot(&workflow.id, &draft.id, 30)
            .unwrap(),
        DeleteSnapshotResult::DraftSnapshot
    );
}

/// Verifies a run is created atomically with its task and worktree and can be read back.
#[test]
fn workflow_run_repository_creates_and_reads_run() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    ensure_project(&pool, "project-1");
    let workflow_repository = SqliteWorkflowRepository::new(pool.clone());
    let run_repository = SqliteWorkflowRunRepository::new(pool.clone());

    let (workflow, draft) = workflow_with_draft("workflow-a", "{\"nodes\":[]}", 10);
    workflow_repository
        .create_workflow(workflow.clone(), draft.clone())
        .unwrap();
    let snapshot = published_snapshot("snapshot-a", &workflow.id, "v1", &draft.graph, 20);
    workflow_repository
        .publish_snapshot(
            &workflow.id,
            snapshot.id.clone(),
            snapshot.version.clone(),
            snapshot.created_at,
        )
        .unwrap();

    let run_id = WorkflowRunId::new("run-1");
    let task_id = TaskId::new("task-1");
    let worktree_id = WorktreeId::new("worktree-1");
    let run = WorkflowRun::new(
        run_id.clone(),
        workflow.id.clone(),
        snapshot.id.clone(),
        WorkflowRunStatus::Pending,
        Some("{\"current_nodes\":[]}".to_string()),
        Some("kickoff".to_string()),
        None,
        None,
        None,
        None,
        None,
        AuditFields::new(30, 30, /*is_deleted*/ false),
    );
    let task = Task::workflow_run(
        task_id.clone(),
        ProjectId::new("project-1"),
        "Workflow workflow-a 30",
        run_id.clone(),
        worktree_id.clone(),
        AuditFields::new(30, 30, /*is_deleted*/ false),
    );
    let worktree = Worktree::new(
        worktree_id.clone(),
        task_id.clone(),
        Some("ora/task-1".to_string()),
        None,
        WorktreeBaseline::recorded("base-commit").unwrap(),
        WorktreeActivity::Active,
        AuditFields::new(30, 30, /*is_deleted*/ false),
    );

    assert_eq!(
        run_repository
            .create_run(
                run.clone(),
                task.clone(),
                worktree.clone(),
                &WorktreeProvisioningLeaseId::new("lease-absent"),
            )
            .unwrap(),
        WorkflowRunCreateOutcome::Created(Box::new(run.clone()))
    );
    assert_eq!(run_repository.find_run(&run_id).unwrap(), Some(run.clone()));
    assert_eq!(
        run_repository.get_run_detail(&run_id).unwrap(),
        Some(WorkflowRunDetail {
            run: run.clone(),
            name: "Workflow workflow-a 30".to_string(),
            project_id: ProjectId::new("project-1"),
            task_id: task_id.clone(),
            nodes: Vec::new(),
        })
    );
    assert_eq!(
        run_repository
            .list_runs_by_project(&ProjectId::new("project-1"))
            .unwrap(),
        vec![WorkflowRunSummary {
            id: run_id.clone(),
            name: "Workflow workflow-a 30".to_string(),
            project_id: ProjectId::new("project-1"),
            workflow_id: workflow.id.clone(),
            status: WorkflowRunStatus::Pending,
            started_at: None,
            finished_at: None,
            created_at: 30,
        }]
    );
    assert_eq!(
        run_repository.list_runs_by_workflow(&workflow.id).unwrap(),
        vec![WorkflowRunSummary {
            id: run_id.clone(),
            name: "Workflow workflow-a 30".to_string(),
            project_id: ProjectId::new("project-1"),
            workflow_id: workflow.id.clone(),
            status: WorkflowRunStatus::Pending,
            started_at: None,
            finished_at: None,
            created_at: 30,
        }]
    );
    assert_eq!(run_repository.list_node_runs(&run_id).unwrap(), Vec::new());
}

/// Verifies the run row must exist before a task can reference it under enforced foreign keys.
///
/// This pins the create_run insert order (`workflow_runs → tasks → worktrees`): inserting a task
/// that references a missing run row must fail, so a correct create_run cannot interleave them.
#[test]
fn workflow_run_repository_requires_run_row_before_task_row() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();

    let result = pool.with_connection(|connection| {
        connection.execute(
            "INSERT INTO tasks (id, project_id, title, type, workflow_run_id, created_at, updated_at, is_deleted)
             VALUES ('task-orphan', 'project-1', 'orphan', 1, 'run-missing', 1, 1, 0)",
            [],
        )?;
        Ok(())
    });

    assert!(
        result.is_err(),
        "a task referencing a run that does not exist yet must violate the foreign key"
    );
}

/// Creates one pending run with its task and worktree, returning their identifiers.
fn create_pending_run_fixture(pool: &RepositoryPool) -> (WorkflowRunId, TaskId, WorktreeId) {
    let workflow_repository = SqliteWorkflowRepository::new(pool.clone());
    let run_repository = SqliteWorkflowRunRepository::new(pool.clone());
    // create_run re-validates project visibility, so the owning project must exist.
    ensure_project(pool, "project-1");
    let (workflow, draft) = workflow_with_draft("workflow-a", "{\"nodes\":[]}", 10);
    workflow_repository
        .create_workflow(workflow.clone(), draft.clone())
        .unwrap();
    let snapshot = published_snapshot("snapshot-a", &workflow.id, "v1", &draft.graph, 20);
    workflow_repository
        .publish_snapshot(
            &workflow.id,
            snapshot.id.clone(),
            snapshot.version.clone(),
            snapshot.created_at,
        )
        .unwrap();

    let run_id = WorkflowRunId::new("run-1");
    let task_id = TaskId::new("task-1");
    let worktree_id = WorktreeId::new("worktree-1");
    let run = WorkflowRun::new(
        run_id.clone(),
        workflow.id.clone(),
        snapshot.id.clone(),
        WorkflowRunStatus::Pending,
        Some("{\"current_nodes\":[]}".to_string()),
        None,
        None,
        None,
        None,
        None,
        None,
        AuditFields::new(30, 30, /*is_deleted*/ false),
    );
    let task = Task::workflow_run(
        task_id.clone(),
        ProjectId::new("project-1"),
        "Workflow workflow-a 30",
        run_id.clone(),
        worktree_id.clone(),
        AuditFields::new(30, 30, /*is_deleted*/ false),
    );
    let worktree = Worktree::new(
        worktree_id.clone(),
        task_id.clone(),
        Some("ora/task-1".to_string()),
        None,
        WorktreeBaseline::recorded("base-commit").unwrap(),
        WorktreeActivity::Active,
        AuditFields::new(30, 30, /*is_deleted*/ false),
    );
    run_repository
        .create_run(
            run,
            task,
            worktree,
            &WorktreeProvisioningLeaseId::new("lease-absent"),
        )
        .unwrap();
    (run_id, task_id, worktree_id)
}

/// Inserts a visible project row when a fixture needs an owning project.
fn ensure_project(pool: &RepositoryPool, project_id: &str) {
    let repository = SqliteProjectRepository::new(pool.clone());
    if repository
        .find_project(&ProjectId::new(project_id))
        .unwrap()
        .is_none()
    {
        repository
            .create_project(Project::new(
                ProjectId::new(project_id),
                "Fixture project",
                "/tmp/fixture-project",
                AuditFields::new(1, 1, false),
            ))
            .unwrap();
    }
}

/// Builds the node-run descriptor for a run's `start` node.
fn start_node_run(run_input: Option<String>) -> NodeRunToStart {
    NodeRunToStart {
        id: WorkflowNodeRunId::new("node-start"),
        node_id: "start".to_string(),
        node_type: "start".to_string(),
        input: run_input,
    }
}

/// Builds an agent node-run descriptor for a ready-node wave.
fn agent_node_run(id: &str, node_id: &str) -> NodeRunToStart {
    NodeRunToStart {
        id: WorkflowNodeRunId::new(id),
        node_id: node_id.to_string(),
        node_type: "agent".to_string(),
        input: None,
    }
}

/// Verifies a running run cannot be soft-deleted.
#[test]
fn workflow_run_repository_rejects_deleting_running_run() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let (run_id, _, _) = create_pending_run_fixture(&pool);
    let repository = SqliteWorkflowRunRepository::new(pool.clone());
    pool.with_connection(|connection| {
        connection.execute(
            "UPDATE workflow_runs SET run_status = 1 WHERE id = ?1",
            rusqlite::params![run_id.as_ref()],
        )?;
        Ok(())
    })
    .unwrap();

    assert_eq!(
        repository.soft_delete_run(&run_id, 40).unwrap(),
        DeleteWorkflowRunResult::ActiveRun
    );
}

/// Verifies a run with a non-terminal node run cannot be soft-deleted.
#[test]
fn workflow_run_repository_rejects_deleting_run_with_pending_node() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let (run_id, _, _) = create_pending_run_fixture(&pool);
    let repository = SqliteWorkflowRunRepository::new(pool.clone());
    pool.with_connection(|connection| {
        connection.execute(
            "INSERT INTO workflow_node_runs (id, run_id, node_id, node_type, status, created_at, updated_at, is_deleted)
             VALUES ('node-1', ?1, 'start', 'start', 0, 30, 30, 0)",
            rusqlite::params![run_id.as_ref()],
        )?;
        Ok(())
    })
    .unwrap();

    assert_eq!(
        repository.soft_delete_run(&run_id, 40).unwrap(),
        DeleteWorkflowRunResult::ActiveRun
    );
}

/// Verifies a run whose task has a running session cannot be soft-deleted.
#[test]
fn workflow_run_repository_rejects_deleting_run_with_running_session() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let (run_id, task_id, _) = create_pending_run_fixture(&pool);
    let repository = SqliteWorkflowRunRepository::new(pool.clone());
    pool.with_connection(|connection| {
        connection.execute(
            "INSERT INTO sessions (id, task_id, agent_cli, agent_session_id, status, created_at, updated_at, is_deleted)
             VALUES ('session-1', ?1, 'ora-space.opencode', 'provider-1', 0, 30, 30, 0)",
            rusqlite::params![task_id.as_ref()],
        )?;
        Ok(())
    })
    .unwrap();

    assert_eq!(
        repository.soft_delete_run(&run_id, 40).unwrap(),
        DeleteWorkflowRunResult::ActiveRun
    );
}

/// Verifies a non-active run soft-deletes with its task, worktree, and stopped sessions.
#[test]
fn workflow_run_repository_soft_deletes_run_and_cascades() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let (run_id, task_id, worktree_id) = create_pending_run_fixture(&pool);
    let repository = SqliteWorkflowRunRepository::new(pool.clone());
    pool.with_connection(|connection| {
        connection.execute(
            "INSERT INTO sessions (id, task_id, agent_cli, agent_session_id, status, created_at, updated_at, is_deleted)
             VALUES ('session-1', ?1, 'ora-space.opencode', 'provider-1', 1, 30, 30, 0)",
            rusqlite::params![task_id.as_ref()],
        )?;
        Ok(())
    })
    .unwrap();

    assert_eq!(
        repository.soft_delete_run(&run_id, 40).unwrap(),
        DeleteWorkflowRunResult::Deleted
    );
    assert_eq!(repository.find_run(&run_id).unwrap(), None);
    assert_eq!(
        repository
            .list_runs_by_project(&ProjectId::new("project-1"))
            .unwrap(),
        Vec::new()
    );
    let task_repository = SqliteTaskRepository::new(pool.clone());
    assert_eq!(task_repository.find_task(&task_id).unwrap(), None);
    let worktree_repository = SqliteWorktreeRepository::new(pool.clone());
    assert_eq!(
        worktree_repository.find_worktree(&worktree_id).unwrap(),
        None
    );
    // A second delete reports not-found because the run is no longer visible.
    assert_eq!(
        repository.soft_delete_run(&run_id, 50).unwrap(),
        DeleteWorkflowRunResult::NotFound
    );
}

/// Verifies the engine repository starts a pending run by creating the start node-run and
/// anchoring it in `current_nodes`.
#[test]
fn engine_repository_starts_a_pending_run() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let (run_id, _, _) = create_pending_run_fixture(&pool);
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());

    assert_eq!(
        repository
            .start_run(&run_id, &start_node_run(None), 40)
            .unwrap(),
        StartWorkflowRunResult::Started
    );
    let run = SqliteWorkflowRunRepository::new(pool.clone())
        .find_run(&run_id)
        .unwrap()
        .unwrap();
    assert_eq!(run.status, WorkflowRunStatus::Running);
    assert_eq!(run.started_at, Some(40));
    assert_eq!(
        run.state.as_deref(),
        Some("{\"current_nodes\":[\"start\"]}")
    );
    let node_runs = SqliteWorkflowRunRepository::new(pool)
        .list_node_runs(&run_id)
        .unwrap();
    assert_eq!(node_runs.len(), 1);
    assert_eq!(node_runs[0].node_id, "start");
    assert_eq!(node_runs[0].node_type, "start");
    assert_eq!(node_runs[0].status, WorkflowNodeStatus::Running);
    assert_eq!(node_runs[0].started_at, Some(40));
}

/// Verifies starting an already-started run is idempotent and adds no node runs.
#[test]
fn engine_repository_start_is_idempotent_for_a_started_run() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let (run_id, _, _) = create_pending_run_fixture(&pool);
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());

    assert_eq!(
        repository
            .start_run(&run_id, &start_node_run(None), 40)
            .unwrap(),
        StartWorkflowRunResult::Started
    );
    assert_eq!(
        repository
            .start_run(&run_id, &start_node_run(None), 41)
            .unwrap(),
        StartWorkflowRunResult::Current
    );
    assert_eq!(
        SqliteWorkflowRunRepository::new(pool)
            .list_node_runs(&run_id)
            .unwrap()
            .len(),
        1
    );
}

/// Verifies starting a run that already reached a terminal status is a no-op.
#[test]
fn engine_repository_start_is_current_for_a_terminal_run() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let (run_id, _, _) = create_pending_run_fixture(&pool);
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    pool.with_connection(|connection| {
        connection.execute(
            "UPDATE workflow_runs SET run_status = 2 WHERE id = ?1",
            rusqlite::params![run_id.as_ref()],
        )?;
        Ok(())
    })
    .unwrap();

    assert_eq!(
        repository
            .start_run(&run_id, &start_node_run(None), 40)
            .unwrap(),
        StartWorkflowRunResult::Current
    );
}

/// Verifies starting a missing run reports not-found.
#[test]
fn engine_repository_start_reports_not_found() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteWorkflowRunEngineRepository::new(pool);

    assert_eq!(
        repository
            .start_run(&WorkflowRunId::new("missing"), &start_node_run(None), 40)
            .unwrap(),
        StartWorkflowRunResult::NotFound
    );
}

/// Verifies the execution context bundles the run, task, worktree, and frozen graph in one read.
#[test]
fn engine_repository_finds_execution_context() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let (run_id, task_id, worktree_id) = create_pending_run_fixture(&pool);
    let repository = SqliteWorkflowRunEngineRepository::new(pool);

    let context = repository.find_execution_context(&run_id).unwrap().unwrap();
    assert_eq!(context.run.id, run_id);
    assert_eq!(context.task.id, task_id);
    assert_eq!(context.worktree.id, worktree_id);
    assert_eq!(context.graph_json, "{\"nodes\":[]}");
    assert_eq!(
        repository
            .find_execution_context(&WorkflowRunId::new("missing"))
            .unwrap(),
        None
    );
}

/// Verifies completing a node removes it from `current_nodes` and records output and stop reason.
#[test]
fn engine_repository_completes_a_node_and_advances_current_nodes() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let (run_id, _, _) = create_pending_run_fixture(&pool);
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    repository
        .start_run(&run_id, &start_node_run(None), 40)
        .unwrap();

    assert_eq!(
        repository
            .complete_node(
                &WorkflowNodeRunId::new("node-start"),
                Some("out".to_string()),
                Some("end_turn".to_string()),
                Vec::new(),
                41,
            )
            .unwrap(),
        AdvanceWorkflowRunResult::Advanced
    );
    let run = SqliteWorkflowRunRepository::new(pool.clone())
        .find_run(&run_id)
        .unwrap()
        .unwrap();
    assert_eq!(run.state.as_deref(), Some("{\"current_nodes\":[]}"));
    let node_runs = SqliteWorkflowRunRepository::new(pool.clone())
        .list_node_runs(&run_id)
        .unwrap();
    assert_eq!(node_runs[0].status, WorkflowNodeStatus::Succeeded);
    assert_eq!(node_runs[0].output.as_deref(), Some("out"));
    assert_eq!(
        node_runs[0].payload.as_deref(),
        Some("{\"stop_reason\":\"end_turn\"}")
    );
    assert_eq!(node_runs[0].finished_at, Some(41));

    // A late or duplicate callback is rejected idempotently.
    assert_eq!(
        repository
            .complete_node(
                &WorkflowNodeRunId::new("node-start"),
                None,
                None,
                Vec::new(),
                42
            )
            .unwrap(),
        AdvanceWorkflowRunResult::NotRunning
    );
}

/// Verifies ready nodes are inserted as running rows and tracked in `current_nodes`.
#[test]
fn engine_repository_starts_ready_nodes_and_tracks_them() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let (run_id, _, _) = create_pending_run_fixture(&pool);
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    repository
        .start_run(&run_id, &start_node_run(None), 40)
        .unwrap();
    repository
        .complete_node(
            &WorkflowNodeRunId::new("node-start"),
            None,
            None,
            Vec::new(),
            41,
        )
        .unwrap();

    repository
        .start_ready_nodes(&run_id, &[agent_node_run("node-a", "a")], 42)
        .unwrap();
    let run = SqliteWorkflowRunRepository::new(pool.clone())
        .find_run(&run_id)
        .unwrap()
        .unwrap();
    assert_eq!(run.state.as_deref(), Some("{\"current_nodes\":[\"a\"]}"));
    let node_runs = SqliteWorkflowRunRepository::new(pool)
        .list_node_runs(&run_id)
        .unwrap();
    assert_eq!(node_runs.len(), 2);
    assert_eq!(node_runs[1].node_id, "a");
    assert_eq!(node_runs[1].status, WorkflowNodeStatus::Running);
}

/// Verifies a running node run is bound to its real Ora session after attach.
#[test]
fn engine_repository_binds_a_node_run_to_its_session() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let (run_id, _, _) = create_pending_run_fixture(&pool);
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    repository
        .start_run(&run_id, &start_node_run(None), 40)
        .unwrap();

    repository
        .set_node_run_session_id(
            &WorkflowNodeRunId::new("node-start"),
            &SessionId::new("session-1"),
            41,
        )
        .unwrap();
    let node_runs = SqliteWorkflowRunRepository::new(pool)
        .list_node_runs(&run_id)
        .unwrap();
    assert_eq!(
        node_runs[0].session_id.as_ref().map(ToString::to_string),
        Some("session-1".to_string())
    );
}

/// Verifies a failed node fails the run and anchors the failed node in `current_nodes`.
#[test]
fn engine_repository_fail_node_fails_run_and_anchors_the_node() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let (run_id, _, _) = create_pending_run_fixture(&pool);
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    repository
        .start_run(&run_id, &start_node_run(None), 40)
        .unwrap();

    assert_eq!(
        repository
            .fail_node(
                &WorkflowNodeRunId::new("node-start"),
                "boom".to_string(),
                Some("partial".to_string()),
                41,
            )
            .unwrap(),
        AdvanceWorkflowRunResult::Advanced
    );
    let run = SqliteWorkflowRunRepository::new(pool.clone())
        .find_run(&run_id)
        .unwrap()
        .unwrap();
    assert_eq!(run.status, WorkflowRunStatus::Failed);
    assert_eq!(run.error.as_deref(), Some("boom"));
    assert_eq!(run.finished_at, Some(41));
    assert_eq!(
        run.state.as_deref(),
        Some("{\"current_nodes\":[\"start\"]}")
    );
    let node_runs = SqliteWorkflowRunRepository::new(pool)
        .list_node_runs(&run_id)
        .unwrap();
    assert_eq!(node_runs[0].status, WorkflowNodeStatus::Failed);
    assert_eq!(node_runs[0].output.as_deref(), Some("partial"));

    // A late fail callback after the run is terminal is a no-op.
    assert_eq!(
        repository
            .fail_node(
                &WorkflowNodeRunId::new("node-start"),
                "late".to_string(),
                None,
                42
            )
            .unwrap(),
        AdvanceWorkflowRunResult::NotRunning
    );
}

/// Verifies finishing a run records the succeeded status and its output.
#[test]
fn engine_repository_finish_run_succeeds() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let (run_id, _, _) = create_pending_run_fixture(&pool);
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    repository
        .start_run(&run_id, &start_node_run(None), 40)
        .unwrap();
    repository
        .complete_node(
            &WorkflowNodeRunId::new("node-start"),
            None,
            None,
            Vec::new(),
            41,
        )
        .unwrap();

    repository
        .finish_run(&run_id, Some("final".to_string()), 45)
        .unwrap();
    let run = SqliteWorkflowRunRepository::new(pool)
        .find_run(&run_id)
        .unwrap()
        .unwrap();
    assert_eq!(run.status, WorkflowRunStatus::Succeeded);
    assert_eq!(run.output.as_deref(), Some("final"));
    assert_eq!(run.finished_at, Some(45));
    assert_eq!(run.state.as_deref(), Some("{\"current_nodes\":[]}"));
}

/// Verifies cancelling a run clears the anchor and cancels non-terminal node runs.
#[test]
fn engine_repository_cancel_cancels_run_and_node_runs() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let (run_id, _, _) = create_pending_run_fixture(&pool);
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    repository
        .start_run(&run_id, &start_node_run(None), 40)
        .unwrap();
    repository
        .start_ready_nodes(&run_id, &[agent_node_run("node-a", "a")], 41)
        .unwrap();

    assert_eq!(
        repository.cancel_run(&run_id, 50).unwrap(),
        CancelWorkflowRunResult::Cancelled
    );
    let run = SqliteWorkflowRunRepository::new(pool.clone())
        .find_run(&run_id)
        .unwrap()
        .unwrap();
    assert_eq!(run.status, WorkflowRunStatus::Cancelled);
    assert_eq!(run.state.as_deref(), Some("{\"current_nodes\":[]}"));
    let node_runs = SqliteWorkflowRunRepository::new(pool.clone())
        .list_node_runs(&run_id)
        .unwrap();
    assert_eq!(node_runs.len(), 2);
    assert!(
        node_runs
            .iter()
            .all(|node_run| node_run.status == WorkflowNodeStatus::Cancelled)
    );

    // Cancelling a terminal run is a no-op.
    assert_eq!(
        repository.cancel_run(&run_id, 51).unwrap(),
        CancelWorkflowRunResult::NotActive
    );
}

/// Verifies restarting a finished run resets it and drops its node runs.
#[test]
fn engine_repository_restart_resets_run_and_deletes_node_runs() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let (run_id, _, _) = create_pending_run_fixture(&pool);
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    repository
        .start_run(&run_id, &start_node_run(None), 40)
        .unwrap();
    repository
        .complete_node(
            &WorkflowNodeRunId::new("node-start"),
            Some("out".to_string()),
            None,
            Vec::new(),
            41,
        )
        .unwrap();
    repository
        .finish_run(&run_id, Some("final".to_string()), 42)
        .unwrap();

    assert_eq!(
        repository.restart_run(&run_id, 50).unwrap(),
        RestartWorkflowRunResult::Restarted
    );
    let run = SqliteWorkflowRunRepository::new(pool.clone())
        .find_run(&run_id)
        .unwrap()
        .unwrap();
    assert_eq!(run.status, WorkflowRunStatus::Pending);
    assert_eq!(run.state.as_deref(), Some("{\"current_nodes\":[]}"));
    assert_eq!(run.output, None);
    assert_eq!(run.started_at, None);
    assert_eq!(run.finished_at, None);
    // The fresh run sees no node runs...
    assert_eq!(
        SqliteWorkflowRunRepository::new(pool.clone())
            .list_node_runs(&run_id)
            .unwrap(),
        Vec::new()
    );
    // ...but the prior node runs are soft-deleted, not removed, so history stays queryable.
    let soft_deleted_rows = pool
        .with_connection(|connection| {
            Ok(connection.query_row(
                "SELECT COUNT(*) FROM workflow_node_runs WHERE run_id = ?1 AND is_deleted = 1",
                rusqlite::params![run_id.as_ref()],
                |row| row.get::<_, i64>(0),
            )?)
        })
        .unwrap();
    assert_eq!(soft_deleted_rows, 1);
}

/// Verifies a pending run's kickoff input can be updated, and is frozen once the run starts.
#[test]
fn engine_repository_updates_pending_run_input() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let (run_id, _, _) = create_pending_run_fixture(&pool);
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    assert_eq!(
        repository
            .update_run_input(&run_id, Some("kickoff".to_string()), 40)
            .unwrap(),
        UpdateWorkflowRunInputResult::Updated
    );
    let run = SqliteWorkflowRunRepository::new(pool.clone())
        .find_run(&run_id)
        .unwrap()
        .unwrap();
    assert_eq!(run.input.as_deref(), Some("kickoff"));

    // Once started, the input is frozen.
    repository
        .start_run(&run_id, &start_node_run(None), 41)
        .unwrap();
    assert_eq!(
        repository
            .update_run_input(&run_id, Some("late".to_string()), 42)
            .unwrap(),
        UpdateWorkflowRunInputResult::NotEditable
    );
}

/// Verifies a terminal run's kickoff input is editable again so a re-run can change it.
#[test]
fn engine_repository_updates_terminal_run_input() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let (run_id, _, _) = create_pending_run_fixture(&pool);
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    repository
        .start_run(&run_id, &start_node_run(None), 40)
        .unwrap();
    assert_eq!(
        repository.cancel_run(&run_id, 41).unwrap(),
        CancelWorkflowRunResult::Cancelled
    );
    // A cancelled (terminal) run is editable again, preparing the next execution.
    assert_eq!(
        repository
            .update_run_input(&run_id, Some("rerun".to_string()), 42)
            .unwrap(),
        UpdateWorkflowRunInputResult::Updated
    );
}

/// Verifies a running run cannot be restarted.
#[test]
fn engine_repository_restart_refuses_a_running_run() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let (run_id, _, _) = create_pending_run_fixture(&pool);
    let repository = SqliteWorkflowRunEngineRepository::new(pool);
    repository
        .start_run(&run_id, &start_node_run(None), 40)
        .unwrap();

    assert_eq!(
        repository.restart_run(&run_id, 41).unwrap(),
        RestartWorkflowRunResult::NotRestartable
    );
}

/// Verifies recoverable runs are exactly those in `Running` or `Failed` status.
#[test]
fn engine_repository_lists_recoverable_runs() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let (run_id, _, _) = create_pending_run_fixture(&pool);
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());

    assert_eq!(
        repository.list_recoverable_runs().unwrap(),
        Vec::<WorkflowRunId>::new()
    );
    repository
        .start_run(&run_id, &start_node_run(None), 40)
        .unwrap();
    assert_eq!(
        repository.list_recoverable_runs().unwrap(),
        vec![run_id.clone()]
    );
    repository
        .fail_node(
            &WorkflowNodeRunId::new("node-start"),
            "boom".to_string(),
            None,
            41,
        )
        .unwrap();
    assert_eq!(repository.list_recoverable_runs().unwrap(), vec![run_id]);
}

/// Verifies the boot sweep fails orphaned node runs and running runs, stops running sessions,
/// preserves the `current_nodes` anchor, and is idempotent.
#[test]
fn engine_repository_fail_orphaned_node_runs_is_idempotent_and_preserves_anchor() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let (run_id, task_id, _) = create_pending_run_fixture(&pool);
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    repository
        .start_run(&run_id, &start_node_run(None), 40)
        .unwrap();
    pool.with_connection(|connection| {
        connection.execute(
            "INSERT INTO sessions (id, task_id, agent_cli, agent_session_id, status, created_at, updated_at, is_deleted)
             VALUES ('session-1', ?1, 'ora-space.opencode', 'provider-1', 0, 30, 30, 0)",
            rusqlite::params![task_id.as_ref()],
        )?;
        Ok(())
    })
    .unwrap();

    repository
        .fail_orphaned_node_runs(&[run_id.clone()], 50)
        .unwrap();
    let run = SqliteWorkflowRunRepository::new(pool.clone())
        .find_run(&run_id)
        .unwrap()
        .unwrap();
    assert_eq!(run.status, WorkflowRunStatus::Failed);
    assert_eq!(
        run.error.as_deref(),
        Some("{\"reason\":\"interrupted_by_restart\"}")
    );
    // The anchor is preserved so the failed node stays visible.
    assert_eq!(
        run.state.as_deref(),
        Some("{\"current_nodes\":[\"start\"]}")
    );
    let node_runs = SqliteWorkflowRunRepository::new(pool.clone())
        .list_node_runs(&run_id)
        .unwrap();
    assert_eq!(node_runs[0].status, WorkflowNodeStatus::Failed);
    assert_eq!(
        node_runs[0].error.as_deref(),
        Some("{\"reason\":\"interrupted_by_restart\"}")
    );
    let session_status = pool
        .with_connection(|connection| {
            Ok(connection.query_row(
                "SELECT status FROM sessions WHERE id = 'session-1'",
                [],
                |row| row.get::<_, i64>(0),
            )?)
        })
        .unwrap();
    assert_eq!(session_status, SessionStatus::Stopped.database_value());

    // The sweep is idempotent: a second pass changes nothing.
    repository
        .fail_orphaned_node_runs(&[run_id.clone()], 51)
        .unwrap();
    let run = SqliteWorkflowRunRepository::new(pool)
        .find_run(&run_id)
        .unwrap()
        .unwrap();
    assert_eq!(run.status, WorkflowRunStatus::Failed);
    assert_eq!(
        run.state.as_deref(),
        Some("{\"current_nodes\":[\"start\"]}")
    );
}

/// Records every agent dispatch so tests can drive completion and assert fan-out.
#[derive(Clone, Default)]
struct RecordingNodeExecutor {
    dispatched: Arc<Mutex<Vec<(String, String)>>>,
}

impl NodeExecutor for RecordingNodeExecutor {
    fn dispatch(
        &self,
        node_run_id: &WorkflowNodeRunId,
        node: &WorkflowGraphNode,
        _context: &ExecutionContext,
    ) {
        self.dispatched
            .lock()
            .unwrap()
            .push((node_run_id.to_string(), node.id.clone()));
    }
}

/// Assigns deterministic ascending node-run ids so tests can predict dispatch order.
#[derive(Default)]
struct SequenceNodeRunIdGenerator {
    next: Cell<u64>,
}

impl WorkflowNodeRunIdGenerator for SequenceNodeRunIdGenerator {
    fn generate_node_run_id(&self) -> WorkflowNodeRunId {
        let current = self.next.get();
        self.next.set(current + 1);
        WorkflowNodeRunId::new(format!("node-{current}"))
    }
}

/// A deterministic clock for engine scheduling tests.
#[derive(Clone, Copy)]
struct FixedClock {
    now: i64,
}

impl FixedClock {
    fn new(now: i64) -> Self {
        Self { now }
    }
}

impl Clock for FixedClock {
    fn now_timestamp_millis(&self) -> i64 {
        self.now
    }
}

/// Builds one pending run against a workflow whose frozen graph is the given JSON.
fn create_pending_run_with_graph(
    pool: &RepositoryPool,
    graph_json: &str,
) -> (WorkflowRunId, TaskId, WorktreeId) {
    let workflow_repository = SqliteWorkflowRepository::new(pool.clone());
    let run_repository = SqliteWorkflowRunRepository::new(pool.clone());
    // create_run re-validates project visibility, so the owning project must exist.
    ensure_project(pool, "project-1");
    let (workflow, draft) = workflow_with_draft("workflow-engine", graph_json, 10);
    workflow_repository
        .create_workflow(workflow.clone(), draft.clone())
        .unwrap();
    let snapshot = published_snapshot("snapshot-engine", &workflow.id, "v1", &draft.graph, 20);
    workflow_repository
        .publish_snapshot(
            &workflow.id,
            snapshot.id.clone(),
            snapshot.version.clone(),
            snapshot.created_at,
        )
        .unwrap();

    let run_id = WorkflowRunId::new("run-1");
    let task_id = TaskId::new("task-1");
    let worktree_id = WorktreeId::new("worktree-1");
    let run = WorkflowRun::new(
        run_id.clone(),
        workflow.id.clone(),
        snapshot.id.clone(),
        WorkflowRunStatus::Pending,
        Some("{\"current_nodes\":[]}".to_string()),
        Some("kickoff".to_string()),
        None,
        None,
        None,
        None,
        None,
        AuditFields::new(30, 30, /*is_deleted*/ false),
    );
    let task = Task::workflow_run(
        task_id.clone(),
        ProjectId::new("project-1"),
        "Workflow workflow-engine 30",
        run_id.clone(),
        worktree_id.clone(),
        AuditFields::new(30, 30, /*is_deleted*/ false),
    );
    let worktree = Worktree::new(
        worktree_id.clone(),
        task_id.clone(),
        Some("ora/task-1".to_string()),
        None,
        WorktreeBaseline::recorded("base-commit").unwrap(),
        WorktreeActivity::Active,
        AuditFields::new(30, 30, /*is_deleted*/ false),
    );
    run_repository
        .create_run(
            run,
            task,
            worktree,
            &WorktreeProvisioningLeaseId::new("lease-absent"),
        )
        .unwrap();
    (run_id, task_id, worktree_id)
}

/// A linear chain: start → agent `a` → output `out`.
fn linear_graph() -> &'static str {
    r#"{"nodes":[
        {"id":"start","data":{"kind":"start"}},
        {"id":"a","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"open_code","modelId":"m"},"roleId":"R","skills":[],"prompt":"do a"}}},
        {"id":"out","data":{"kind":"output"}}
    ],"edges":[{"source":"start","target":"a"},{"source":"a","target":"out"}]}"#
}

/// Two parallel branches merging into `merge`, then `out`.
fn fan_in_graph() -> &'static str {
    r#"{"nodes":[
        {"id":"start","data":{"kind":"start"}},
        {"id":"l","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"roleId":"R","skills":[],"prompt":"l"}}},
        {"id":"r","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"roleId":"R","skills":[],"prompt":"r"}}},
        {"id":"merge","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"roleId":"R","skills":[],"prompt":"merge"}}},
        {"id":"out","data":{"kind":"output"}}
    ],"edges":[{"source":"start","target":"l"},{"source":"start","target":"r"},{"source":"l","target":"merge"},{"source":"r","target":"merge"},{"source":"merge","target":"out"}]}"#
}

/// A graph containing a v1-unsupported `condition` node.
fn unsupported_graph() -> &'static str {
    r#"{"nodes":[
        {"id":"start","data":{"kind":"start"}},
        {"id":"c","data":{"kind":"condition","condition":"x"}}
    ],"edges":[{"source":"start","target":"c"}]}"#
}

/// A graph with no start node.
fn no_start_graph() -> &'static str {
    r#"{"nodes":[{"id":"a","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"roleId":"R","skills":[],"prompt":"a"}}}],"edges":[]}"#
}

/// A graph with an orphaned node unreachable from start.
fn unreachable_graph() -> &'static str {
    r#"{"nodes":[
        {"id":"start","data":{"kind":"start"}},
        {"id":"a","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"roleId":"R","skills":[],"prompt":"a"}}},
        {"id":"orphan","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"roleId":"R","skills":[],"prompt":"orphan"}}}
    ],"edges":[{"source":"start","target":"a"}]}"#
}

/// Builds a one-turn assistant conversation array used as an agent node output.
fn assistant_conversation(text: &str) -> String {
    serde_json::json!([{ "role": "assistant", "text": text }]).to_string()
}

/// Verifies the engine runs a linear chain to `Succeeded`, executing control nodes synchronously
/// and driving the agent node through the executor.
#[test]
fn engine_runs_a_linear_chain_to_success() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let (run_id, _, _) = create_pending_run_with_graph(&pool, linear_graph());
    let executor = RecordingNodeExecutor::default();
    let engine = WorkflowRunEngine::new(
        SqliteWorkflowRunEngineRepository::new(pool.clone()),
        executor.clone(),
        SequenceNodeRunIdGenerator::default(),
        FixedClock::new(40),
    );

    assert_eq!(
        engine.start(&run_id).unwrap(),
        StartWorkflowRunResult::Started
    );

    // The start node completed synchronously; the agent `a` is dispatched and in flight.
    let node_runs = SqliteWorkflowRunRepository::new(pool.clone())
        .list_node_runs(&run_id)
        .unwrap();
    let start_run = node_runs
        .iter()
        .find(|node_run| node_run.node_id == "start")
        .unwrap();
    assert_eq!(start_run.status, WorkflowNodeStatus::Succeeded);
    assert_eq!(start_run.input.as_deref(), Some("kickoff"));
    let agent_run = node_runs
        .iter()
        .find(|node_run| node_run.node_id == "a")
        .unwrap();
    assert_eq!(agent_run.status, WorkflowNodeStatus::Running);
    assert_eq!(agent_run.input.as_deref(), Some("do a"));
    assert_eq!(executor.dispatched.lock().unwrap().len(), 1);

    // Completing the agent lets the output node finish synchronously and the run succeed.
    engine
        .complete_node(
            &run_id,
            &agent_run.id,
            Some(assistant_conversation("done")),
            Some("end_turn".to_string()),
            Vec::new(),
        )
        .unwrap();
    let run = SqliteWorkflowRunRepository::new(pool)
        .find_run(&run_id)
        .unwrap()
        .unwrap();
    assert_eq!(run.status, WorkflowRunStatus::Succeeded);
    assert_eq!(run.output.as_deref(), Some("done"));
}

/// Verifies a failing agent fails the run and anchors the failed node in `current_nodes`.
#[test]
fn engine_fails_the_run_when_a_node_fails() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let (run_id, _, _) = create_pending_run_with_graph(&pool, linear_graph());
    let engine = WorkflowRunEngine::new(
        SqliteWorkflowRunEngineRepository::new(pool.clone()),
        RecordingNodeExecutor::default(),
        SequenceNodeRunIdGenerator::default(),
        FixedClock::new(40),
    );
    assert_eq!(
        engine.start(&run_id).unwrap(),
        StartWorkflowRunResult::Started
    );

    let agent_run = SqliteWorkflowRunRepository::new(pool.clone())
        .list_node_runs(&run_id)
        .unwrap()
        .into_iter()
        .find(|node_run| node_run.node_id == "a")
        .unwrap();
    engine
        .fail_node(
            &agent_run.id,
            "boom".to_string(),
            Some("partial".to_string()),
        )
        .unwrap();

    let run = SqliteWorkflowRunRepository::new(pool.clone())
        .find_run(&run_id)
        .unwrap()
        .unwrap();
    assert_eq!(run.status, WorkflowRunStatus::Failed);
    assert_eq!(run.error.as_deref(), Some("boom"));
    assert_eq!(run.state.as_deref(), Some("{\"current_nodes\":[\"a\"]}"));
    let agent_run = SqliteWorkflowRunRepository::new(pool)
        .list_node_runs(&run_id)
        .unwrap()
        .into_iter()
        .find(|node_run| node_run.node_id == "a")
        .unwrap();
    assert_eq!(agent_run.status, WorkflowNodeStatus::Failed);
    assert_eq!(agent_run.output.as_deref(), Some("partial"));
}

/// Verifies both branches of a fan-in graph dispatch concurrently and `merge` waits for both.
#[test]
fn engine_dispatches_parallel_branches_concurrently() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let (run_id, _, _) = create_pending_run_with_graph(&pool, fan_in_graph());
    let executor = RecordingNodeExecutor::default();
    let engine = WorkflowRunEngine::new(
        SqliteWorkflowRunEngineRepository::new(pool.clone()),
        executor.clone(),
        SequenceNodeRunIdGenerator::default(),
        FixedClock::new(40),
    );

    assert_eq!(
        engine.start(&run_id).unwrap(),
        StartWorkflowRunResult::Started
    );

    // Both leaves dispatched in the same wave; the merge node waits.
    let dispatched: Vec<String> = executor
        .dispatched
        .lock()
        .unwrap()
        .iter()
        .map(|(_, node_id)| node_id.clone())
        .collect();
    assert_eq!(dispatched, vec!["l", "r"]);
    let node_runs = SqliteWorkflowRunRepository::new(pool.clone())
        .list_node_runs(&run_id)
        .unwrap();
    assert!(!node_runs.iter().any(|node_run| node_run.node_id == "merge"));

    // Completing both leaves makes `merge` ready.
    let left = node_runs
        .iter()
        .find(|node_run| node_run.node_id == "l")
        .unwrap()
        .id
        .clone();
    let right = node_runs
        .iter()
        .find(|node_run| node_run.node_id == "r")
        .unwrap()
        .id
        .clone();
    engine
        .complete_node(
            &run_id,
            &left,
            Some(assistant_conversation("left")),
            None,
            Vec::new(),
        )
        .unwrap();
    engine
        .complete_node(
            &run_id,
            &right,
            Some(assistant_conversation("right")),
            None,
            Vec::new(),
        )
        .unwrap();
    let dispatched: Vec<String> = executor
        .dispatched
        .lock()
        .unwrap()
        .iter()
        .map(|(_, node_id)| node_id.clone())
        .collect();
    assert_eq!(dispatched, vec!["l", "r", "merge"]);

    // Completing `merge` drains the graph and the run succeeds.
    let merge = SqliteWorkflowRunRepository::new(pool.clone())
        .list_node_runs(&run_id)
        .unwrap()
        .into_iter()
        .find(|node_run| node_run.node_id == "merge")
        .unwrap();
    engine
        .complete_node(
            &run_id,
            &merge.id,
            Some(assistant_conversation("merged")),
            None,
            Vec::new(),
        )
        .unwrap();
    let run = SqliteWorkflowRunRepository::new(pool)
        .find_run(&run_id)
        .unwrap()
        .unwrap();
    assert_eq!(run.status, WorkflowRunStatus::Succeeded);
    assert_eq!(run.output.as_deref(), Some("merged"));
}

/// Verifies cancelling a running run commits the cancelled transition.
#[test]
fn engine_cancels_a_running_run() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let (run_id, _, _) = create_pending_run_with_graph(&pool, linear_graph());
    let engine = WorkflowRunEngine::new(
        SqliteWorkflowRunEngineRepository::new(pool.clone()),
        RecordingNodeExecutor::default(),
        SequenceNodeRunIdGenerator::default(),
        FixedClock::new(40),
    );
    assert_eq!(
        engine.start(&run_id).unwrap(),
        StartWorkflowRunResult::Started
    );

    assert_eq!(
        engine.cancel(&run_id).unwrap(),
        CancelWorkflowRunResult::Cancelled
    );
    let run = SqliteWorkflowRunRepository::new(pool)
        .find_run(&run_id)
        .unwrap()
        .unwrap();
    assert_eq!(run.status, WorkflowRunStatus::Cancelled);
}

/// Verifies restarting a finished run resets it and immediately re-runs it.
#[test]
fn engine_restarts_a_finished_run() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let (run_id, _, _) = create_pending_run_with_graph(&pool, linear_graph());
    let engine = WorkflowRunEngine::new(
        SqliteWorkflowRunEngineRepository::new(pool.clone()),
        RecordingNodeExecutor::default(),
        SequenceNodeRunIdGenerator::default(),
        FixedClock::new(40),
    );
    assert_eq!(
        engine.start(&run_id).unwrap(),
        StartWorkflowRunResult::Started
    );
    let agent_run = SqliteWorkflowRunRepository::new(pool.clone())
        .list_node_runs(&run_id)
        .unwrap()
        .into_iter()
        .find(|node_run| node_run.node_id == "a")
        .unwrap();
    engine
        .complete_node(
            &run_id,
            &agent_run.id,
            Some(assistant_conversation("done")),
            None,
            Vec::new(),
        )
        .unwrap();
    assert_eq!(
        SqliteWorkflowRunRepository::new(pool.clone())
            .find_run(&run_id)
            .unwrap()
            .unwrap()
            .status,
        WorkflowRunStatus::Succeeded
    );

    assert_eq!(
        engine.restart(&run_id).unwrap(),
        RestartWorkflowRunResult::Restarted
    );
    let run = SqliteWorkflowRunRepository::new(pool.clone())
        .find_run(&run_id)
        .unwrap()
        .unwrap();
    assert_eq!(run.status, WorkflowRunStatus::Running);
    let node_runs = SqliteWorkflowRunRepository::new(pool)
        .list_node_runs(&run_id)
        .unwrap();
    assert!(
        node_runs.iter().any(|node_run| node_run.node_id == "start"
            && node_run.status == WorkflowNodeStatus::Succeeded)
    );
    assert!(
        node_runs
            .iter()
            .any(|node_run| node_run.node_id == "a"
                && node_run.status == WorkflowNodeStatus::Running)
    );
}

/// Verifies the control handler starts a run and returns its running state.
#[test]
fn workflow_run_control_start_returns_the_running_run() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let (run_id, _, _) = create_pending_run_with_graph(&pool, linear_graph());
    let engine = WorkflowRunEngine::new(
        SqliteWorkflowRunEngineRepository::new(pool.clone()),
        RecordingNodeExecutor::default(),
        SequenceNodeRunIdGenerator::default(),
        FixedClock::new(40),
    );
    let handler =
        WorkflowRunControlHandler::new(engine, Arc::new(SqliteWorkflowRunRepository::new(pool)));

    let response = handler
        .start(StartWorkflowRunRequest {
            run_id: run_id.to_string(),
        })
        .unwrap();
    assert_eq!(response.run.status, ContractRunStatus::Running);
    assert_eq!(response.run.id, run_id.to_string());
}

/// Verifies start rejects a graph containing a v1-unsupported node type.
#[test]
fn engine_rejects_unsupported_node_type() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let (run_id, _, _) = create_pending_run_with_graph(&pool, unsupported_graph());
    let engine = WorkflowRunEngine::new(
        SqliteWorkflowRunEngineRepository::new(pool),
        RecordingNodeExecutor::default(),
        SequenceNodeRunIdGenerator::default(),
        FixedClock::new(40),
    );

    let error = engine.start(&run_id).unwrap_err();
    assert!(matches!(
        error,
        EngineError::Validation(WorkflowValidationError::UnsupportedNodeType {
            node_id,
            node_type,
        }) if node_id == "c" && node_type == NodeType::Condition
    ));
}

/// Verifies start rejects a graph without a start node.
#[test]
fn engine_rejects_missing_start_node() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let (run_id, _, _) = create_pending_run_with_graph(&pool, no_start_graph());
    let engine = WorkflowRunEngine::new(
        SqliteWorkflowRunEngineRepository::new(pool),
        RecordingNodeExecutor::default(),
        SequenceNodeRunIdGenerator::default(),
        FixedClock::new(40),
    );

    assert!(matches!(
        engine.start(&run_id).unwrap_err(),
        EngineError::Validation(WorkflowValidationError::MissingStartNode)
    ));
}

/// Verifies start rejects nodes unreachable from the unique start node.
#[test]
fn engine_rejects_unreachable_nodes() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let (run_id, _, _) = create_pending_run_with_graph(&pool, unreachable_graph());
    let engine = WorkflowRunEngine::new(
        SqliteWorkflowRunEngineRepository::new(pool),
        RecordingNodeExecutor::default(),
        SequenceNodeRunIdGenerator::default(),
        FixedClock::new(40),
    );

    let error = engine.start(&run_id).unwrap_err();
    assert!(matches!(
        error,
        EngineError::Validation(WorkflowValidationError::UnreachableNodes { node_ids })
            if node_ids == vec!["orphan".to_string()]
    ));
}

/// Verifies a workflow with live runs cannot be deleted, protecting the runs' frozen snapshots.
#[test]
fn workflow_repository_rejects_deleting_workflow_with_live_runs() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteWorkflowRepository::new(pool.clone());
    let (workflow, draft) = workflow_with_draft("workflow-a", "{\"nodes\":[]}", 10);
    repository
        .create_workflow(workflow.clone(), draft.clone())
        .unwrap();
    let snapshot = published_snapshot("snapshot-a", &workflow.id, "v1", &draft.graph, 20);
    repository
        .publish_snapshot(
            &workflow.id,
            snapshot.id.clone(),
            snapshot.version.clone(),
            snapshot.created_at,
        )
        .unwrap();
    insert_run_referencing_snapshot(&pool, "run-1", &workflow.id, &snapshot.id, false);

    assert_eq!(
        repository.soft_delete_workflow(&workflow.id, 30).unwrap(),
        DeleteWorkflowResult::ActiveRuns
    );
    assert!(repository.find_workflow(&workflow.id).unwrap().is_some());

    // Once the run is soft-deleted, the workflow can be deleted.
    pool.with_connection(|connection| {
        connection.execute(
            "UPDATE workflow_runs SET is_deleted = 1 WHERE id = 'run-1'",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    assert_eq!(
        repository.soft_delete_workflow(&workflow.id, 40).unwrap(),
        DeleteWorkflowResult::Deleted
    );
    assert!(repository.find_workflow(&workflow.id).unwrap().is_none());
}

/// Inserts one workflow run row referencing a snapshot for delete-guard fixtures.
fn insert_run_referencing_snapshot(
    pool: &RepositoryPool,
    run_id: &str,
    workflow_id: &WorkflowId,
    snapshot_id: &WorkflowSnapshotId,
    is_deleted: bool,
) {
    pool.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO workflow_runs (id, workflow_id, snapshot_id, run_status, state, created_at, updated_at, is_deleted)
                 VALUES (?1, ?2, ?3, 0, ?5, 10, 10, ?4)",
                rusqlite::params![
                    run_id,
                    workflow_id.as_ref(),
                    snapshot_id.as_ref(),
                    i64::from(is_deleted),
                    "{\"current_nodes\":[]}",
                ],
            )?;
        Ok(())
    })
    .unwrap();
}

/// Builds a workflow and its required draft snapshot for repository integration tests.
fn workflow_with_draft(id: &str, graph: &str, created_at: i64) -> (Workflow, WorkflowSnapshot) {
    let workflow_id = WorkflowId::new(id);
    let workflow = Workflow::new(
        workflow_id.clone(),
        Namespace::local(),
        format!("Workflow {id}"),
        /*published_snapshot_id*/ None,
        AuditFields::new(created_at, created_at, /*is_deleted*/ false),
    )
    .unwrap();
    let draft = WorkflowSnapshot::new(
        WorkflowSnapshotId::new(format!("{id}-draft")),
        workflow_id,
        "draft",
        graph,
        created_at,
        Some(created_at),
        /*is_deleted*/ false,
    );

    (workflow, draft)
}

/// Builds one immutable published snapshot for repository integration tests.
fn published_snapshot(
    id: &str,
    workflow_id: &WorkflowId,
    version: &str,
    graph: &str,
    created_at: i64,
) -> WorkflowSnapshot {
    WorkflowSnapshot::new(
        WorkflowSnapshotId::new(id),
        workflow_id.clone(),
        version,
        graph,
        created_at,
        /*updated_at*/ None,
        /*is_deleted*/ false,
    )
}

fn skill(
    id: &str,
    name: &str,
    description: &str,
    created_at: i64,
    updated_at: i64,
    is_deleted: bool,
) -> Skill {
    Skill::new(
        SkillId::new(id),
        Namespace::local(),
        name,
        description,
        AuditFields::new(created_at, updated_at, is_deleted),
    )
    .unwrap()
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
        Namespace::local(),
        name,
        description,
        "",
        AuditFields::new(created_at, updated_at, is_deleted),
    )
    .unwrap()
}

/// Produces deterministic bootstrap timestamps so repository tests can assert stored objects.
#[derive(Clone, Copy, Debug)]
struct FixedTimestampSource {
    now: i64,
}

impl TimestampSource for FixedTimestampSource {
    /// Returns the deterministic timestamp configured for the current test.
    fn current_timestamp_millis(&self) -> i64 {
        self.now
    }
}

/// Verifies pooled repository connections use the requested SQLite runtime settings.
#[test]
fn bootstrapped_repository_pool_configures_sqlite_pragmas() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();

    let (journal_mode, busy_timeout, synchronous) = pool
        .with_connection(|connection| {
            let journal_mode = connection
                .pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))?;
            let busy_timeout =
                connection.pragma_query_value(None, "busy_timeout", |row| row.get::<_, i64>(0))?;
            let synchronous =
                connection.pragma_query_value(None, "synchronous", |row| row.get::<_, i64>(0))?;

            Ok((journal_mode, busy_timeout, synchronous))
        })
        .unwrap();

    assert_eq!(journal_mode, "wal".to_string());
    assert_eq!(busy_timeout, 5_000_i64);
    assert_eq!(synchronous, 1_i64);
}

/// Verifies the SQLite-backed project repository preserves CRUD snapshots and hides soft-deleted rows.
#[test]
fn project_repository_supports_crud_and_soft_delete() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteProjectRepository::new(pool);
    let created_project = Project::new(
        ProjectId::new("project-1"),
        "Ora",
        "/tmp/ora",
        AuditFields::new(10, 10, false),
    );

    assert_eq!(
        repository.create_project(created_project.clone()).unwrap(),
        created_project.clone()
    );
    assert_eq!(
        repository.find_project(&created_project.id).unwrap(),
        Some(created_project.clone())
    );
    assert_eq!(
        repository.list_projects().unwrap(),
        vec![created_project.clone()]
    );

    let updated_project = Project::new(
        created_project.id.clone(),
        "Ora Updated",
        "/tmp/ora-updated",
        AuditFields::new(10, 20, false),
    );

    assert_eq!(
        repository.update_project(updated_project.clone()).unwrap(),
        updated_project.clone()
    );
    assert_eq!(
        repository.find_project(&updated_project.id).unwrap(),
        Some(updated_project.clone())
    );
    assert_eq!(
        repository
            .soft_delete_project(&updated_project.id, /*deleted_at*/ 30)
            .unwrap(),
        true
    );
    assert_eq!(repository.find_project(&updated_project.id).unwrap(), None);
    assert_eq!(repository.list_projects().unwrap(), Vec::<Project>::new());
}

/// Verifies the SQLite-backed task repository preserves CRUD snapshots and hides soft-deleted rows.
#[test]
fn task_repository_supports_crud_and_soft_delete() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteTaskRepository::new(pool);
    let created_task = Task::new(
        TaskId::new("task-1"),
        ProjectId::new("project-1"),
        "Wire the pool",
        Some(WorktreeId::new("worktree-1")),
        AuditFields::new(11, 11, false),
    );

    assert_eq!(
        repository.create_task(created_task.clone()).unwrap(),
        created_task.clone()
    );
    assert_eq!(
        repository.find_task(&created_task.id).unwrap(),
        Some(created_task.clone())
    );
    assert_eq!(repository.list_tasks().unwrap(), vec![created_task.clone()]);

    let updated_task = Task::new(
        created_task.id.clone(),
        created_task.project_id.clone(),
        "Wire the repository pool",
        None,
        AuditFields::new(11, 21, false),
    );

    assert_eq!(
        repository.update_task(updated_task.clone()).unwrap(),
        updated_task.clone()
    );
    assert_eq!(
        repository.find_task(&updated_task.id).unwrap(),
        Some(updated_task.clone())
    );
    assert_eq!(
        repository
            .soft_delete_task(&updated_task.id, /*deleted_at*/ 31)
            .unwrap(),
        true
    );
    assert_eq!(repository.find_task(&updated_task.id).unwrap(), None);
    assert_eq!(repository.list_tasks().unwrap(), Vec::<Task>::new());
}

/// Verifies the SQLite-backed session repository preserves CRUD snapshots and hides soft-deleted rows.
#[test]
fn session_repository_supports_crud_and_soft_delete() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let project_repository = SqliteProjectRepository::new(pool.clone());
    let task_repository = SqliteTaskRepository::new(pool.clone());
    let repository = SqliteSessionRepository::new(pool.clone());
    project_repository
        .create_project(Project::new(
            ProjectId::new("project-1"),
            "Ora",
            "/tmp/ora",
            AuditFields::new(10, 10, false),
        ))
        .unwrap();
    task_repository
        .create_task(Task::new(
            TaskId::new("task-1"),
            ProjectId::new("project-1"),
            "Test sessions",
            None,
            AuditFields::new(11, 11, false),
        ))
        .unwrap();
    let created_session = Session::new(
        SessionId::new("session-1"),
        TaskId::new("task-1"),
        AgentCli::OpenCode,
        "provider-1",
        SessionStatus::Running,
        AuditFields::new(12, 12, false),
    );

    assert_eq!(
        repository.create_session(created_session.clone()).unwrap(),
        created_session.clone()
    );
    assert_eq!(
        pool.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT agent_cli FROM sessions WHERE id = ?1",
                    rusqlite::params![created_session.id.as_ref()],
                    |row| row.get::<_, String>(0),
                )
                .map_err(crate::DatabaseError::from)
        })
        .unwrap(),
        "ora-space.opencode"
    );
    assert_eq!(
        repository.find_session(&created_session.id).unwrap(),
        Some(created_session.clone())
    );
    assert_eq!(
        repository.list_sessions().unwrap(),
        vec![created_session.clone()]
    );

    let updated_session = Session::new(
        created_session.id.clone(),
        created_session.task_id.clone(),
        created_session.agent_cli,
        created_session.agent_session_id.clone(),
        SessionStatus::Stopped,
        AuditFields::new(12, 22, false),
    );

    assert_eq!(
        repository
            .update_session_status(&updated_session.id, updated_session.status, /*now*/ 22,)
            .unwrap(),
        updated_session.clone()
    );
    assert_eq!(
        repository.find_session(&updated_session.id).unwrap(),
        Some(updated_session.clone())
    );
    assert_eq!(
        repository
            .soft_delete_session(&updated_session.id, /*deleted_at*/ 32)
            .unwrap(),
        true
    );
    assert_eq!(repository.find_session(&updated_session.id).unwrap(), None);
    assert_eq!(repository.list_sessions().unwrap(), Vec::<Session>::new());
}

/// Verifies each session mutation preserves fields owned by the other independent operations.
#[test]
fn session_repository_updates_do_not_overwrite_unrelated_columns() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    insert_cascade_fixture(&pool, SessionStatus::Stopped);
    let repository = SqliteSessionRepository::new(pool);
    let session_id = SessionId::new("session-1");
    let title = SessionTitle::parse("Generated title").unwrap();
    let existing = repository
        .find_session(&session_id)
        .unwrap()
        .expect("fixture session");

    let titled = repository
        .update_session_title(&session_id, &title, /*now*/ 40)
        .unwrap();
    let mut expected_titled = existing.clone();
    expected_titled.title = Some(title.clone());
    expected_titled.audit_fields.updated_at = 40;
    assert_eq!(titled, expected_titled);

    let rebound = repository
        .update_session_binding(&session_id, AgentCli::Nga, "provider-2", /*now*/ 41)
        .unwrap();
    let expected_rebound = expected_titled.with_binding(AgentCli::Nga, "provider-2", 41);
    assert_eq!(rebound, expected_rebound);

    let running = repository
        .update_session_status(&session_id, SessionStatus::Running, /*now*/ 42)
        .unwrap();
    let expected_running = expected_rebound.with_status(SessionStatus::Running, 42);
    assert_eq!(running, expected_running);

    let degraded = HistoryState::Degraded {
        reason: "recording failed".to_string(),
    };
    let history_updated = repository
        .update_session_history_state(&session_id, &degraded, /*now*/ 43)
        .unwrap();
    let expected_history_updated = expected_running.with_history_state(degraded, 43);
    assert_eq!(history_updated, expected_history_updated);
}

/// Verifies switching agents rewrites the provider binding while the conversation keeps its identity.
#[test]
fn session_repository_rebinds_a_session_to_another_agent_cli() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    insert_cascade_fixture(&pool, SessionStatus::Stopped);
    let repository = SqliteSessionRepository::new(pool);
    let existing = repository
        .find_session(&SessionId::new("session-1"))
        .unwrap()
        .expect("fixture session");

    let rebound =
        existing
            .clone()
            .with_binding(AgentCli::Nga, "provider-2", /*updated_at*/ 40);

    assert_eq!(
        repository
            .update_session_binding(
                &rebound.id,
                rebound.agent_cli,
                &rebound.agent_session_id,
                /*now*/ 40,
            )
            .unwrap(),
        rebound
    );
    assert_eq!(
        repository.find_session(&rebound.id).unwrap(),
        Some(rebound.clone())
    );
    // The conversation is the row, not the provider session behind it.
    assert_eq!(rebound.id, existing.id);
    assert_eq!(rebound.task_id, existing.task_id);
}

/// Verifies a degraded history reason survives storage and clears when the session recovers.
#[test]
fn session_repository_round_trips_history_state() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    insert_cascade_fixture(&pool, SessionStatus::Stopped);
    let repository = SqliteSessionRepository::new(pool);
    let existing = repository
        .find_session(&SessionId::new("session-1"))
        .unwrap()
        .expect("fixture session");
    assert_eq!(existing.history_state, HistoryState::Writable);

    let degraded = existing.clone().with_history_state(
        HistoryState::Degraded {
            reason: "no space left on device".to_string(),
        },
        /*updated_at*/ 40,
    );
    repository
        .update_session_history_state(&degraded.id, &degraded.history_state, /*now*/ 40)
        .unwrap();
    assert_eq!(
        repository.find_session(&degraded.id).unwrap(),
        Some(degraded.clone())
    );

    let recovered = degraded.with_history_state(HistoryState::Writable, /*updated_at*/ 50);
    repository
        .update_session_history_state(&recovered.id, &recovered.history_state, /*now*/ 50)
        .unwrap();

    assert_eq!(
        repository.find_session(&recovered.id).unwrap(),
        Some(recovered)
    );
}

/// Verifies a completed ACP handshake cannot attach a new session to a deleted task.
#[test]
fn session_repository_rejects_soft_deleted_task() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    insert_cascade_fixture(&pool, SessionStatus::Stopped);
    let cascade = SqliteCascadeRepository::new(pool.clone());
    assert_eq!(
        cascade.delete_task(&TaskId::new("task-1"), 20).unwrap(),
        CascadeDeleteOutcome::Deleted
    );
    let session = Session::new(
        SessionId::new("session-after-delete"),
        TaskId::new("task-1"),
        AgentCli::OpenCode,
        "provider-after-delete",
        SessionStatus::Running,
        AuditFields::new(21, 21, false),
    );

    assert!(
        SqliteSessionRepository::new(pool)
            .create_session(session)
            .is_err()
    );
}

/// Verifies the SQLite-backed worktree repository preserves CRUD snapshots and hides soft-deleted rows.
#[test]
fn worktree_repository_supports_crud_and_soft_delete() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteWorktreeRepository::new(pool);
    let created_worktree = Worktree::new(
        WorktreeId::new("worktree-1"),
        TaskId::new("task-1"),
        Some("feature/db-pool".to_string()),
        Some("/worktrees/task-1".to_string()),
        ora_domain::WorktreeBaseline::recorded("base-commit").unwrap(),
        WorktreeActivity::Inactive,
        AuditFields::new(13, 13, false),
    );

    assert_eq!(
        repository
            .create_worktree(created_worktree.clone())
            .unwrap(),
        created_worktree.clone()
    );
    assert_eq!(
        repository.find_worktree(&created_worktree.id).unwrap(),
        Some(created_worktree.clone())
    );
    assert_eq!(
        repository.list_worktrees().unwrap(),
        vec![created_worktree.clone()]
    );

    let updated_worktree = Worktree::new(
        created_worktree.id.clone(),
        created_worktree.task_id.clone(),
        None,
        None,
        ora_domain::WorktreeBaseline::recorded("updated-base-commit").unwrap(),
        WorktreeActivity::Active,
        AuditFields::new(13, 23, false),
    );

    assert_eq!(
        repository
            .update_worktree(updated_worktree.clone())
            .unwrap(),
        updated_worktree.clone()
    );
    assert_eq!(
        repository.find_worktree(&updated_worktree.id).unwrap(),
        Some(updated_worktree.clone())
    );
    assert_eq!(
        repository
            .soft_delete_worktree(&updated_worktree.id, /*deleted_at*/ 33)
            .unwrap(),
        true
    );
    assert_eq!(
        repository.find_worktree(&updated_worktree.id).unwrap(),
        None
    );
    assert_eq!(repository.list_worktrees().unwrap(), Vec::<Worktree>::new());
}

/// Verifies a single repository pool can back all four application repository adapters together.
#[test]
fn repository_pool_composes_all_repository_adapters() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let project_repository = SqliteProjectRepository::new(pool.clone());
    let task_repository = SqliteTaskRepository::new(pool.clone());
    let session_repository = SqliteSessionRepository::new(pool.clone());
    let worktree_repository = SqliteWorktreeRepository::new(pool);
    let project = Project::new(
        ProjectId::new("project-1"),
        "Ora",
        "/tmp/ora",
        AuditFields::new(40, 40, false),
    );
    let task = Task::new(
        TaskId::new("task-1"),
        project.id.clone(),
        "Implement pool composition",
        Some(WorktreeId::new("worktree-1")),
        AuditFields::new(41, 41, false),
    );
    let session = Session::new(
        SessionId::new("session-1"),
        task.id.clone(),
        AgentCli::OpenCode,
        "provider-1",
        SessionStatus::Running,
        AuditFields::new(42, 42, false),
    );
    let worktree = Worktree::new(
        WorktreeId::new("worktree-1"),
        task.id.clone(),
        Some("feature/composition".to_string()),
        None,
        ora_domain::WorktreeBaseline::recorded("base-commit").unwrap(),
        WorktreeActivity::Active,
        AuditFields::new(43, 43, false),
    );

    assert_eq!(
        project_repository.create_project(project.clone()).unwrap(),
        project.clone()
    );
    assert_eq!(
        task_repository.create_task(task.clone()).unwrap(),
        task.clone()
    );
    assert_eq!(
        session_repository.create_session(session.clone()).unwrap(),
        session.clone()
    );
    assert_eq!(
        worktree_repository
            .create_worktree(worktree.clone())
            .unwrap(),
        worktree.clone()
    );
    assert_eq!(
        project_repository.find_project(&project.id).unwrap(),
        Some(project)
    );
    assert_eq!(task_repository.find_task(&task.id).unwrap(), Some(task));
    assert_eq!(
        session_repository.find_session(&session.id).unwrap(),
        Some(session)
    );
    assert_eq!(
        worktree_repository.find_worktree(&worktree.id).unwrap(),
        Some(worktree)
    );
}

/// Verifies task aggregate deletion rejects running sessions and then commits every soft delete.
#[test]
fn task_cascade_delete_is_atomic_and_does_not_require_git() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    insert_cascade_fixture(&pool, SessionStatus::Running);
    let repository = SqliteCascadeRepository::new(pool.clone());

    assert_eq!(
        repository.delete_task(&TaskId::new("task-1"), 20).unwrap(),
        CascadeDeleteOutcome::ActiveSession
    );
    assert_eq!(cascade_flags(&pool), (0, 0, 0, 0));
    pool.with_connection(|connection| {
        connection.execute(
            "UPDATE sessions SET status = ?1 WHERE id = 'session-1'",
            rusqlite::params![SessionStatus::Stopped.database_value()],
        )?;
        Ok(())
    })
    .unwrap();

    assert_eq!(
        repository.delete_task(&TaskId::new("task-1"), 30).unwrap(),
        CascadeDeleteOutcome::Deleted
    );
    assert_eq!(cascade_flags(&pool), (0, 1, 1, 1));
}

/// Verifies project deletion soft-deletes the full Ora aggregate without touching external state.
#[test]
fn project_cascade_delete_soft_deletes_aggregate_without_touching_external_state() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    insert_cascade_fixture(&pool, SessionStatus::Stopped);
    let repository = SqliteCascadeRepository::new(pool.clone());

    assert_eq!(
        repository
            .delete_project(&ProjectId::new("project-1"), 30)
            .unwrap(),
        CascadeDeleteOutcome::Deleted
    );
    assert_eq!(cascade_flags(&pool), (1, 1, 1, 1));
}

/// Inserts one complete aggregate using only Ora-owned rows, deliberately without Git fixtures.
fn insert_cascade_fixture(pool: &RepositoryPool, session_status: SessionStatus) {
    pool.with_connection(|connection| {
        connection.execute_batch(
            "INSERT INTO projects VALUES ('project-1', 'Ora', '/not/a/repository', 1, 1, 0);
             INSERT INTO tasks (id, project_id, title, worktree_id, created_at, updated_at, is_deleted)
             VALUES ('task-1', 'project-1', 'Task', 'worktree-1', 1, 1, 0);
             INSERT INTO worktrees (
                 id, task_id, branch_name, is_active, created_at, updated_at, is_deleted, base_commit_id
             ) VALUES ('worktree-1', 'task-1', 'ora/task-1', 1, 1, 1, 0, 'base-commit');",
        )?;
        // Columns are named rather than positional so a later schema addition
        // does not silently shift this fixture's values into the wrong ones.
        connection.execute(
            "INSERT INTO sessions (id, task_id, agent_cli, agent_session_id, status, created_at, updated_at, is_deleted)
             VALUES ('session-1', 'task-1', 'ora-space.opencode', 'provider-1', ?1, 1, 1, 0)",
            rusqlite::params![session_status.database_value()],
        )?;
        Ok(())
    })
    .unwrap();
}

/// Reads all aggregate deletion markers touched by a cascade.
fn cascade_flags(pool: &RepositoryPool) -> (i64, i64, i64, i64) {
    pool.with_connection(|connection| {
        Ok((
            connection.query_row(
                "SELECT is_deleted FROM projects WHERE id = 'project-1'",
                [],
                |row| row.get(0),
            )?,
            connection.query_row(
                "SELECT is_deleted FROM tasks WHERE id = 'task-1'",
                [],
                |row| row.get(0),
            )?,
            connection.query_row(
                "SELECT is_deleted FROM worktrees WHERE id = 'worktree-1'",
                [],
                |row| row.get(0),
            )?,
            connection.query_row(
                "SELECT is_deleted FROM sessions WHERE id = 'session-1'",
                [],
                |row| row.get(0),
            )?,
        ))
    })
    .unwrap()
}

/// Verifies project repositories translate SQLite statement failures into application-owned errors.
#[test]
fn project_repository_reports_sqlite_failures() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteProjectRepository::new(pool);
    let project = Project::new(
        ProjectId::new("project-1"),
        "Ora",
        "/tmp/ora",
        AuditFields::new(50, 50, false),
    );

    repository.create_project(project.clone()).unwrap();

    assert_repository_source(
        repository.create_project(project).unwrap_err(),
        "sqlite error: UNIQUE constraint failed: projects.id",
    );
}

/// Verifies task repositories translate invalid persisted type values into application-owned errors.
#[test]
fn task_repository_reports_row_mapping_failures() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteTaskRepository::new(pool.clone());

    insert_invalid_task_row(&pool);

    assert_repository_source(
        repository
            .find_task(&TaskId::new("task-invalid"))
            .unwrap_err(),
        "domain model error: invalid task type value: 99",
    );
}

/// Verifies session repositories translate invalid persisted status values into application-owned errors.
#[test]
fn session_repository_reports_row_mapping_failures() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteSessionRepository::new(pool.clone());

    insert_invalid_session_row(&pool);

    assert_repository_source(
        repository
            .find_session(&SessionId::new("session-invalid"))
            .unwrap_err(),
        "domain model error: invalid session status value: 99",
    );
}

/// Verifies worktree repositories translate invalid persisted activity values into application-owned errors.
#[test]
fn worktree_repository_reports_row_mapping_failures() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteWorktreeRepository::new(pool.clone());

    insert_invalid_worktree_row(&pool);

    assert_repository_source(
        repository
            .find_worktree(&WorktreeId::new("worktree-invalid"))
            .unwrap_err(),
        "domain model error: invalid worktree activity value: 99",
    );
}

fn assert_repository_source(error: RepositoryError, expected: &str) {
    let source = std::error::Error::source(&error).expect("repository source must be retained");
    assert_eq!(source.to_string(), expected);
    assert!(
        source.downcast_ref::<DatabaseError>().is_some(),
        "repository source must be the concrete DatabaseError, got {source:?}"
    );
}

/// Bootstraps a file-backed SQLite database and returns the ready repository pool.
fn bootstrapped_repository_pool() -> (TempDir, RepositoryPool) {
    let temp_dir = TempDir::new().unwrap();
    let pool = with_trace_logging(|| {
        DatabaseBootstrapper::new(FixedTimestampSource {
            now: 1_700_000_000_000,
        })
        .bootstrap_repository_pool(
            &DatabaseLocation::path(database_path(&temp_dir)),
            &default_migration_catalog().unwrap(),
        )
        .unwrap()
    });

    (temp_dir, pool)
}

/// Builds the file path used by a repository integration test database.
fn database_path(temp_dir: &TempDir) -> PathBuf {
    temp_dir.path().join("repository.sqlite3")
}

/// Inserts one task row with an invalid type integer for row-mapping error coverage.
fn insert_invalid_task_row(pool: &RepositoryPool) {
    pool.with_connection(|connection| {
        connection.execute(
            "INSERT INTO tasks (id, project_id, title, type, worktree_id, created_at, updated_at, is_deleted)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                "task-invalid",
                "project-1",
                "Broken task",
                99,
                Option::<String>::None,
                60,
                60,
                0,
            ],
        )?;

        Ok(())
    })
    .unwrap();
}

/// Inserts one session row with an invalid status integer for row-mapping error coverage.
fn insert_invalid_session_row(pool: &RepositoryPool) {
    pool.with_connection(|connection| {
        connection.execute(
            "INSERT INTO sessions (id, task_id, agent_cli, agent_session_id, status, created_at, updated_at, is_deleted)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                "session-invalid",
                "task-1",
                AgentCli::OpenCode.database_value(),
                "provider-invalid",
                99,
                61,
                61,
                0,
            ],
        )?;

        Ok(())
    })
    .unwrap();
}

/// Inserts one worktree row with an invalid activity integer for row-mapping error coverage.
fn insert_invalid_worktree_row(pool: &RepositoryPool) {
    pool.with_connection(|connection| {
        connection.execute(
            "INSERT INTO worktrees (id, task_id, branch_name, is_active, created_at, updated_at, is_deleted)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                "worktree-invalid",
                "task-1",
                Option::<String>::None,
                99,
                62,
                62,
                0,
            ],
        )?;

        Ok(())
    })
    .unwrap();
}
