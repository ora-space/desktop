//! Power-loss durability coverage for the pooled SQLite engine.
//!
//! The Desktop database stores the sidebar's projects, tasks, worktrees, and
//! conversations, so a power cut that rolls acknowledged transactions back is
//! experienced as those rows disappearing after reboot. These tests simulate
//! the cut at the VFS layer (see [`crash_simulator`]) and verify both the
//! failure mechanism of the old `synchronous = NORMAL` profile and the
//! survival of committed repository rows under the `synchronous = FULL`
//! profile the connection manager now applies.

mod crash_simulator;

use crash_simulator::register_simulator;
use ora_application::{
    ProjectRepository, SessionRepository, TaskRepository, TaskWorkspaceCommit,
    WorkspaceCommitOutcome, WorktreeRepository,
};
use ora_domain::{
    AgentRef, AuditFields, Project, ProjectId, Session, SessionId, SessionMcpSelection,
    SessionStatus, Task, TaskId, WorkspaceId, WorkspaceLocation, Worktree, WorktreeActivity,
    WorktreeBaseline, WorktreeProvisioningLease, WorktreeProvisioningLeaseId,
};
use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;
use rusqlite::Connection;
use tempfile::TempDir;

use crate::{
    DatabaseBootstrapper, DatabaseLocation, SqliteProjectRepository, SqliteSessionRepository,
    SqliteTaskRepository, SqliteTaskWorkspaceRepository, SqliteWorktreeProvisioningLeaseRepository,
    SqliteWorktreeRepository, TimestampSource, default_migration_catalog, test_clock::TestClock,
};

/// Supplies a deterministic migration timestamp without touching process state.
#[derive(Clone, Copy, Debug)]
struct FixedTimestampSource;

impl TimestampSource for FixedTimestampSource {
    /// Returns the fixed timestamp used while opening the test database.
    fn current_timestamp_millis(&self) -> i64 {
        1
    }
}

/// Reproduces the reported failure mechanism: under `synchronous = NORMAL` a
/// COMMIT the connection already acknowledged is rolled back by a power cut.
///
/// This control proves the simulator really drops unsynced writes, so the
/// survival tests below cannot pass merely because the cut is a no-op.
#[test]
fn acknowledged_commit_under_normal_synchronous_is_rolled_back_by_a_power_cut()
-> rusqlite::Result<()> {
    let simulator = register_simulator();
    let directory = TempDir::new().expect("create temporary database directory");
    let uri = simulator.database_uri(directory.path());

    let connection = Connection::open(&uri)?;
    connection.pragma_update(/*schema_name*/ None, "journal_mode", "WAL")?;
    // The durable prefix: created while commits were still synced.
    connection.pragma_update(/*schema_name*/ None, "synchronous", "FULL")?;
    connection.execute_batch("CREATE TABLE acknowledged (value INTEGER NOT NULL)")?;
    // One more transaction under the profile the bug report hit.
    connection.pragma_update(/*schema_name*/ None, "synchronous", "NORMAL")?;
    connection.execute("INSERT INTO acknowledged (value) VALUES (1)", [])?;
    let visible: i64 =
        connection.query_row("SELECT count(*) FROM acknowledged", [], |row| row.get(0))?;
    assert_eq!(
        visible, 1,
        "the connection observes its own acknowledged COMMIT"
    );

    // The power cut happens while the app is still running; the drop below only
    // releases dead handles, so no close-time checkpoint can rescue the commit.
    simulator.crash();
    drop(connection);

    let after_reboot = Connection::open(&uri)?;
    let surviving: i64 =
        after_reboot.query_row("SELECT count(*) FROM acknowledged", [], |row| row.get(0))?;
    assert_eq!(
        surviving, 0,
        "NORMAL never synced the WAL, so the power cut rolled the acknowledged COMMIT back",
    );
    Ok(())
}

/// Proves the same power cut keeps the acknowledged COMMIT once every commit
/// syncs the WAL, which is the durability profile the pool now configures.
#[test]
fn acknowledged_commit_under_full_synchronous_survives_a_power_cut() -> rusqlite::Result<()> {
    let simulator = register_simulator();
    let directory = TempDir::new().expect("create temporary database directory");
    let uri = simulator.database_uri(directory.path());

    let connection = Connection::open(&uri)?;
    connection.pragma_update(/*schema_name*/ None, "journal_mode", "WAL")?;
    connection.pragma_update(/*schema_name*/ None, "synchronous", "FULL")?;
    connection.execute_batch("CREATE TABLE acknowledged (value INTEGER NOT NULL)")?;
    connection.execute("INSERT INTO acknowledged (value) VALUES (1)", [])?;
    let visible: i64 =
        connection.query_row("SELECT count(*) FROM acknowledged", [], |row| row.get(0))?;
    assert_eq!(
        visible, 1,
        "the connection observes its own acknowledged COMMIT"
    );

    // Same cut, same still-running connection; only the sync profile differs.
    simulator.crash();
    drop(connection);

    let after_reboot = Connection::open(&uri)?;
    let surviving: i64 =
        after_reboot.query_row("SELECT count(*) FROM acknowledged", [], |row| row.get(0))?;
    assert_eq!(
        surviving, 1,
        "FULL synced the WAL at commit, so the acknowledged COMMIT survives the power cut",
    );
    Ok(())
}

/// Verifies the full production path end to end: a pool bootstrapped the way
/// Desktop opens its database, rows committed through the production
/// repositories, a power cut with the app still running, and a reboot that
/// bootstraps the same database again — exactly the sequence a user's machine
/// went through, minus the actual outage.
#[test]
fn repository_rows_survive_a_power_cut_with_the_pool_still_running()
-> Result<(), Box<dyn std::error::Error>> {
    let simulator = register_simulator();
    let directory = TempDir::new()?;
    let location = DatabaseLocation::path(simulator.database_uri(directory.path()));
    let catalog = default_migration_catalog()?;

    let pool = with_trace_logging(|| {
        DatabaseBootstrapper::new(FixedTimestampSource)
            .bootstrap_repository_pool(&location, &catalog)
    })?;

    // The rows the sidebar's project list and each project's task tree are
    // built from, written through the repositories Desktop composes.
    let project = SqliteProjectRepository::new(pool.clone()).create_project(
        Project::new(
            ProjectId::new("project-1"),
            "Power-loss project",
            AuditFields::new(10, 10, /*is_deleted*/ false),
        ),
        WorkspaceLocation::local_filesystem(
            directory
                .path()
                .join("repository")
                .to_string_lossy()
                .into_owned(),
        ),
    )?;

    let workspace_id = WorkspaceId::new("workspace-task-1");
    let lease = WorktreeProvisioningLease::new(
        WorktreeProvisioningLeaseId::new("lease-1"),
        project.id.clone(),
        workspace_id.clone(),
        directory
            .path()
            .join("repository")
            .to_string_lossy()
            .into_owned(),
        directory
            .path()
            .join("worktrees")
            .join("task-1")
            .to_string_lossy()
            .into_owned(),
        "ora/task-1",
        /*lease_expires_at*/ 60_000,
        /*now*/ 10,
    );
    SqliteWorktreeProvisioningLeaseRepository::new(pool.clone()).create_lease(&lease)?;

    let task = Task::new(
        TaskId::new("task-1"),
        project.id.clone(),
        workspace_id.clone(),
        "Power-loss task",
        AuditFields::new(10, 10, /*is_deleted*/ false),
    );
    let worktree = Worktree::new(
        workspace_id.clone(),
        Some("ora/task-1".to_string()),
        WorktreeBaseline::recorded("base-commit")?,
        WorktreeActivity::Active,
        AuditFields::new(10, 10, /*is_deleted*/ false),
    );
    let outcome = SqliteTaskWorkspaceRepository::with_clock(pool.clone(), TestClock::new(10))
        .commit_worktree_task(&task, &worktree, &lease.id)?;
    assert_eq!(outcome, WorkspaceCommitOutcome::Committed);

    let session = Session::new(
        SessionId::new("session-1"),
        workspace_id,
        AgentRef::parse("ora-space.opencode")?,
        "provider-session-1",
        SessionStatus::Running,
        SessionMcpSelection::Automatic,
        AuditFields::new(20, 20, /*is_deleted*/ false),
    );
    assert_eq!(
        SqliteSessionRepository::new(pool.clone()).create_session(session.clone())?,
        session
    );

    // The power cut happens while the app is still running: the dropped pool
    // below only releases dead handles, never gets to checkpoint or clean up.
    simulator.crash();
    drop(pool);

    let rebooted = with_trace_logging(|| {
        DatabaseBootstrapper::new(FixedTimestampSource)
            .bootstrap_repository_pool(&location, &catalog)
    })?;

    assert_eq!(
        SqliteProjectRepository::new(rebooted.clone()).list_projects()?,
        vec![project],
        "the project must still be listed after the power cut",
    );
    assert_eq!(
        SqliteTaskRepository::new(rebooted.clone()).list_tasks()?,
        vec![task],
        "the task row must survive the power cut",
    );
    assert_eq!(
        SqliteWorktreeRepository::new(rebooted.clone()).list_worktrees()?,
        vec![worktree],
        "the worktree row must survive the power cut",
    );
    assert_eq!(
        SqliteSessionRepository::new(rebooted.clone()).list_sessions()?,
        vec![session],
        "the conversation row must survive the power cut",
    );
    assert_eq!(
        SqliteWorktreeProvisioningLeaseRepository::new(rebooted.clone()).list_leases()?,
        Vec::<WorktreeProvisioningLease>::new(),
        "committing the task deletes its lease in the same transaction, so that deletion must survive the power cut",
    );
    rebooted.with_connection(|connection| {
        let integrity: String =
            connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        assert_eq!(integrity, "ok");
        Ok(())
    })?;
    Ok(())
}

/// Proves a provisioning lease that has not been consumed yet is still listed
/// after a power cut. `commit_worktree_task` deletes the lease, so the end-to-end
/// test above cannot show that the write-ahead record itself is durable.
#[test]
fn outstanding_provisioning_lease_survives_a_power_cut() -> Result<(), Box<dyn std::error::Error>> {
    let simulator = register_simulator();
    let directory = TempDir::new()?;
    let location = DatabaseLocation::path(simulator.database_uri(directory.path()));
    let catalog = default_migration_catalog()?;

    let pool = with_trace_logging(|| {
        DatabaseBootstrapper::new(FixedTimestampSource)
            .bootstrap_repository_pool(&location, &catalog)
    })?;
    let project = SqliteProjectRepository::new(pool.clone()).create_project(
        Project::new(
            ProjectId::new("project-lease"),
            "Lease project",
            AuditFields::new(10, 10, /*is_deleted*/ false),
        ),
        WorkspaceLocation::local_filesystem(
            directory
                .path()
                .join("repository")
                .to_string_lossy()
                .into_owned(),
        ),
    )?;
    let lease = WorktreeProvisioningLease::new(
        WorktreeProvisioningLeaseId::new("lease-outstanding"),
        project.id,
        WorkspaceId::new("workspace-lease"),
        directory
            .path()
            .join("repository")
            .to_string_lossy()
            .into_owned(),
        directory
            .path()
            .join("worktrees")
            .join("lease")
            .to_string_lossy()
            .into_owned(),
        "ora/lease",
        /*lease_expires_at*/ 60_000,
        /*now*/ 10,
    );
    SqliteWorktreeProvisioningLeaseRepository::new(pool.clone()).create_lease(&lease)?;

    simulator.crash();
    drop(pool);

    let rebooted = with_trace_logging(|| {
        DatabaseBootstrapper::new(FixedTimestampSource)
            .bootstrap_repository_pool(&location, &catalog)
    })?;
    assert_eq!(
        SqliteWorktreeProvisioningLeaseRepository::new(rebooted).list_leases()?,
        vec![lease],
        "an acknowledged provisioning lease must still be reclaimable after the power cut",
    );
    Ok(())
}

#[test]
fn a_crash_between_lease_write_and_ownership_commit_leaves_no_half_commit()
-> Result<(), Box<dyn std::error::Error>> {
    use crate::{RepositoryPool, SqliteScheduledRunRepository, SqliteScheduledTaskRepository};
    use ora_application::{
        ScheduledOccurrenceCandidate, ScheduledOccurrenceRepository, ScheduledTaskRepository,
    };
    use ora_domain::{
        ScheduledOccurrence, ScheduledRun, ScheduledRunId, ScheduledTask, ScheduledTaskApproval,
        ScheduledTaskExecutionLocation, ScheduledTaskId, ScheduledTaskSchedule,
    };

    let simulator = register_simulator();
    let directory = TempDir::new()?;
    let location = DatabaseLocation::path(simulator.database_uri(directory.path()));
    let catalog = default_migration_catalog()?;

    let pool = with_trace_logging(|| {
        DatabaseBootstrapper::new(FixedTimestampSource)
            .bootstrap_repository_pool(&location, &catalog)
    })?;
    SqliteProjectRepository::new(pool.clone()).create_project(
        Project::new(
            ProjectId::new("project-1"),
            "Crash-window project",
            AuditFields::new(10, 10, /*is_deleted*/ false),
        ),
        WorkspaceLocation::local_filesystem(
            directory.path().join("repo").to_string_lossy().into_owned(),
        ),
    )?;
    let definition = ScheduledTask::new(
        ScheduledTaskId::new("scheduled-task-1"),
        "Quality report",
        true,
        ScheduledTaskSchedule::daily(9, 0)?,
        ScheduledTaskExecutionLocation::Local,
        ProjectId::new("project-1"),
        AgentRef::parse("ora/agent")?,
        None,
        "Run the quality gate",
        Some(ScheduledTaskApproval::full_auto_in_worktree(10)),
        AuditFields::new(10, 10, false),
    )?;
    SqliteScheduledTaskRepository::new(pool.clone()).create_scheduled_task(definition.clone())?;
    let run_id = ScheduledRunId::new("run-crash-window");
    let run = ScheduledRun::accepted(
        run_id.clone(),
        ScheduledOccurrence {
            task_id: definition.id.clone(),
            scheduled_at: 1_000,
        },
        &definition,
        1_001,
    );
    SqliteScheduledRunRepository::new(pool.clone())
        .resolve_scheduled_occurrence(ScheduledOccurrenceCandidate::Run(Box::new(run)))?;
    let workspace_id = WorkspaceId::new("workspace-crash");
    let lease = WorktreeProvisioningLease::new(
        WorktreeProvisioningLeaseId::new("lease-crash"),
        ProjectId::new("project-1"),
        workspace_id,
        directory.path().join("repo").to_string_lossy().into_owned(),
        directory
            .path()
            .join("worktrees")
            .join("crash")
            .to_string_lossy()
            .into_owned(),
        "ora/task-crash",
        /*lease_expires_at*/ i64::MAX,
        /*now*/ 10,
    );
    SqliteWorktreeProvisioningLeaseRepository::new(pool.clone()).create_lease(&lease)?;

    // The write-ahead lease and the admitted run are durable; the ownership
    // commit never ran. Cut power while the pool is still live: nothing gets a
    // normal Drop path or compensation.
    simulator.crash();
    drop(pool);

    let assert_crash_window_state =
        |after: &RepositoryPool| -> Result<(), Box<dyn std::error::Error>> {
            assert_eq!(
                SqliteWorktreeProvisioningLeaseRepository::new(after.clone()).list_leases()?,
                vec![lease.clone()],
                "the write-ahead lease still owns the resources"
            );
            assert_eq!(
                crate::repository::SqliteTaskRepository::new(after.clone()).list_tasks()?,
                Vec::new(),
                "no task rows were partially committed"
            );
            let binding = after.with_connection(|connection| {
            let row = connection.query_row(
                "SELECT worktree_task_id, worktree_path FROM scheduled_runs WHERE id = 'run-crash-window'",
                [],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?)),
            )?;
            Ok(row)
        })?;
            assert_eq!(
                binding,
                (None, None),
                "the run never gained a partial ownership binding"
            );
            Ok(())
        };

    // First reboot: the in-memory state is gone, the durable rows remain.
    let rebooted = with_trace_logging(|| {
        DatabaseBootstrapper::new(FixedTimestampSource)
            .bootstrap_repository_pool(&location, &catalog)
    })?;
    assert_crash_window_state(&rebooted)?;
    drop(rebooted);

    // Repeated reopen is idempotent: no recovery pass may create a task or
    // duplicate a lease/binding on a second boot.
    let rebooted_again = with_trace_logging(|| {
        DatabaseBootstrapper::new(FixedTimestampSource)
            .bootstrap_repository_pool(&location, &catalog)
    })?;
    assert_crash_window_state(&rebooted_again)?;

    let _ = run_id;
    Ok(())
}
