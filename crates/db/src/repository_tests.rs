use std::{
    path::PathBuf,
    sync::{Arc, Barrier},
    thread,
};

use ora_application::{
    ActivateVersionResult, AgentDefinitionRepository, DeleteSnapshotResult, DeleteWorkflowResult,
    DeleteWorkflowRunResult, ProjectRepository, ProjectWorkContextRepository,
    PublishSnapshotResult, RepositoryError, RollbackDraftResult, SessionRepository,
    SkillRepository, TaskRepository, WorkflowRepository, WorkflowRunRepository, WorktreeRepository,
};
use ora_domain::{
    AgentCli, AgentDefinition, AgentDefinitionId, AuditFields, HistoryState, Project, ProjectId,
    ProjectWorkContext, ProjectWorkContextId, ProjectWorkContextSurface, Session, SessionId,
    SessionStatus, Skill, SkillId, Task, TaskId, TaskStatus, Workflow, WorkflowId, WorkflowRun,
    WorkflowRunDetail, WorkflowRunId, WorkflowRunStatus, WorkflowRunSummary, WorkflowSnapshot,
    WorkflowSnapshotId, Worktree, WorktreeActivity, WorktreeBaseline, WorktreeId,
};
use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;
use tempfile::TempDir;

use crate::{
    CascadeDeleteOutcome, DatabaseBootstrapper, DatabaseLocation, RepositoryPool,
    SqliteAgentDefinitionRepository, SqliteCascadeRepository, SqliteProjectRepository,
    SqliteProjectWorkContextRepository, SqliteSessionRepository, SqliteSkillRepository,
    SqliteTaskRepository, SqliteWorkflowRepository, SqliteWorkflowRunRepository,
    SqliteWorktreeRepository, TimestampSource, default_migration_catalog,
};

/// Verifies catalog repositories use stable identifiers and hide soft-deleted rows.
#[test]
fn catalog_repositories_support_id_based_crud_and_allow_duplicate_names() {
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
    let earlier_skill = skill("skill-0", "review", "Builds", 0, 0, false);
    let earlier_agent = agent("agent-0", "opencode", "Assists", 0, 0, false);
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
        TaskStatus::Todo,
        run_id.clone(),
        worktree_id.clone(),
        AuditFields::new(30, 30, /*is_deleted*/ false),
    );
    let worktree = Worktree::new(
        worktree_id.clone(),
        task_id.clone(),
        Some("ora/task-1".to_string()),
        WorktreeBaseline::recorded("base-commit").unwrap(),
        WorktreeActivity::Active,
        AuditFields::new(30, 30, /*is_deleted*/ false),
    );

    assert_eq!(
        run_repository
            .create_run(run.clone(), task.clone(), worktree.clone())
            .unwrap(),
        run.clone()
    );
    assert_eq!(run_repository.find_run(&run_id).unwrap(), Some(run.clone()));
    assert_eq!(
        run_repository.get_run_detail(&run_id).unwrap(),
        Some(WorkflowRunDetail {
            run: run.clone(),
            name: "Workflow workflow-a 30".to_string(),
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
            "INSERT INTO tasks (id, project_id, title, status, type, workflow_run_id, created_at, updated_at, is_deleted)
             VALUES ('task-orphan', 'project-1', 'orphan', 0, 1, 'run-missing', 1, 1, 0)",
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
        TaskStatus::Todo,
        run_id.clone(),
        worktree_id.clone(),
        AuditFields::new(30, 30, /*is_deleted*/ false),
    );
    let worktree = Worktree::new(
        worktree_id.clone(),
        task_id.clone(),
        Some("ora/task-1".to_string()),
        WorktreeBaseline::recorded("base-commit").unwrap(),
        WorktreeActivity::Active,
        AuditFields::new(30, 30, /*is_deleted*/ false),
    );
    run_repository.create_run(run, task, worktree).unwrap();
    (run_id, task_id, worktree_id)
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
        name,
        description,
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
        repository
            .find_project_by_name(&created_project.name)
            .unwrap(),
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
            .find_project_by_name(&updated_project.name)
            .unwrap(),
        Some(updated_project.clone())
    );
    assert_eq!(
        repository
            .soft_delete_project(&updated_project.id, /*deleted_at*/ 30)
            .unwrap(),
        true
    );
    assert_eq!(repository.find_project(&updated_project.id).unwrap(), None);
    assert_eq!(
        repository
            .find_project_by_name(&updated_project.name)
            .unwrap(),
        None
    );
    assert_eq!(repository.list_projects().unwrap(), Vec::<Project>::new());
}

/// Verifies the SQLite-backed project repository can load one visible project by exact name.
#[test]
fn project_repository_finds_visible_project_by_name() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteProjectRepository::new(pool);
    let project = Project::new(
        ProjectId::new("project-1"),
        "Ora",
        "/tmp/ora",
        AuditFields::new(14, 14, false),
    );

    repository.create_project(project.clone()).unwrap();

    assert_eq!(
        repository.find_project_by_name("Ora").unwrap(),
        Some(project)
    );
    assert_eq!(repository.find_project_by_name("Missing").unwrap(), None);
}

/// Verifies the SQLite-backed project repository hides soft-deleted rows during name-based lookup.
#[test]
fn project_repository_ignores_soft_deleted_projects_during_name_lookup() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteProjectRepository::new(pool);
    let project = Project::new(
        ProjectId::new("project-1"),
        "Ora",
        "/tmp/ora",
        AuditFields::new(15, 15, false),
    );

    repository.create_project(project.clone()).unwrap();
    repository
        .soft_delete_project(&project.id, /*deleted_at*/ 16)
        .unwrap();

    assert_eq!(repository.find_project_by_name("Ora").unwrap(), None);
}

/// Verifies the SQLite-backed project work context repository preserves lease-aware rows and cleanup.
#[test]
fn project_work_context_repository_supports_active_lookup_and_cleanup() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteProjectWorkContextRepository::new(pool);
    let created_context = ProjectWorkContext::new(
        ProjectWorkContextId::new("context-1"),
        ProjectWorkContextSurface::Tauri,
        "window-1",
        ProjectId::new("project-1"),
        120,
        10,
        10,
    );

    assert_eq!(
        repository
            .create_project_work_context(created_context.clone())
            .unwrap(),
        created_context.clone()
    );
    assert_eq!(
        repository
            .find_project_work_context(ProjectWorkContextSurface::Tauri, "window-1")
            .unwrap(),
        Some(created_context.clone())
    );
    assert_eq!(
        repository
            .find_active_project_work_context_for_project(&created_context.project_id, 100)
            .unwrap(),
        Some(created_context.clone())
    );
    assert_eq!(
        repository
            .find_active_project_work_context_for_project(&created_context.project_id, 120)
            .unwrap(),
        None
    );

    let updated_context = ProjectWorkContext::new(
        created_context.id.clone(),
        created_context.surface,
        created_context.window_id.clone(),
        ProjectId::new("project-2"),
        240,
        created_context.created_at,
        40,
    );

    assert_eq!(
        repository
            .update_project_work_context(updated_context.clone())
            .unwrap(),
        updated_context.clone()
    );
    assert_eq!(
        repository
            .find_active_project_work_context_for_project(&ProjectId::new("project-2"), 200)
            .unwrap(),
        Some(updated_context.clone())
    );
    assert_eq!(
        repository
            .delete_expired_project_work_contexts(200)
            .unwrap(),
        0
    );
    assert_eq!(
        repository
            .delete_project_work_context(ProjectWorkContextSurface::Tauri, "window-1")
            .unwrap(),
        true
    );
    assert_eq!(
        repository
            .find_project_work_context(ProjectWorkContextSurface::Tauri, "window-1")
            .unwrap(),
        None
    );
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
        TaskStatus::Todo,
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
        TaskStatus::Doing,
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
            TaskStatus::Todo,
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
        repository.update_session(updated_session.clone()).unwrap(),
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

    assert_eq!(repository.update_session(rebound.clone()).unwrap(), rebound);
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
    repository.update_session(degraded.clone()).unwrap();
    assert_eq!(
        repository.find_session(&degraded.id).unwrap(),
        Some(degraded.clone())
    );

    let recovered = degraded.with_history_state(HistoryState::Writable, /*updated_at*/ 50);
    repository.update_session(recovered.clone()).unwrap();

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
        TaskStatus::Todo,
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
    assert_eq!(cascade_flags(&pool), (0, 0, 0, 0, 1));
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
    assert_eq!(cascade_flags(&pool), (0, 1, 1, 1, 1));
}

/// Verifies project deletion removes its transient lease and soft-deletes the full Ora aggregate.
#[test]
fn project_cascade_delete_removes_work_context_without_touching_external_state() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    insert_cascade_fixture(&pool, SessionStatus::Stopped);
    let repository = SqliteCascadeRepository::new(pool.clone());

    assert_eq!(
        repository
            .delete_project(&ProjectId::new("project-1"), 30)
            .unwrap(),
        CascadeDeleteOutcome::Deleted
    );
    assert_eq!(cascade_flags(&pool), (1, 1, 1, 1, 0));
}

/// Inserts one complete aggregate using only Ora-owned rows, deliberately without Git fixtures.
fn insert_cascade_fixture(pool: &RepositoryPool, session_status: SessionStatus) {
    pool.with_connection(|connection| {
        connection.execute_batch(
            "INSERT INTO projects VALUES ('project-1', 'Ora', '/not/a/repository', 1, 1, 0);
             INSERT INTO tasks (id, project_id, title, status, worktree_id, created_at, updated_at, is_deleted)
             VALUES ('task-1', 'project-1', 'Task', 0, 'worktree-1', 1, 1, 0);
             INSERT INTO worktrees (
                 id, task_id, branch_name, is_active, created_at, updated_at, is_deleted, base_commit_id
             ) VALUES ('worktree-1', 'task-1', 'ora/task-1', 1, 1, 1, 0, 'base-commit');
             INSERT INTO project_work_contexts VALUES ('context-1', 'web', 'main', 'project-1', 100, 1, 1);",
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

/// Reads all aggregate deletion markers plus the remaining transient work-context count.
fn cascade_flags(pool: &RepositoryPool) -> (i64, i64, i64, i64, i64) {
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
            connection.query_row("SELECT COUNT(*) FROM project_work_contexts", [], |row| {
                row.get(0)
            })?,
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

/// Verifies task repositories translate invalid persisted status values into application-owned errors.
#[test]
fn task_repository_reports_row_mapping_failures() {
    let (_temp_dir, pool) = bootstrapped_repository_pool();
    let repository = SqliteTaskRepository::new(pool.clone());

    insert_invalid_task_row(&pool);

    assert_repository_source(
        repository
            .find_task(&TaskId::new("task-invalid"))
            .unwrap_err(),
        "domain model error: invalid task status value: 99",
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

/// Inserts one task row with an invalid status integer for row-mapping error coverage.
fn insert_invalid_task_row(pool: &RepositoryPool) {
    pool.with_connection(|connection| {
        connection.execute(
            "INSERT INTO tasks (id, project_id, title, status, worktree_id, created_at, updated_at, is_deleted)
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
