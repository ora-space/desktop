use super::{GitCleanupWorker, MAX_JOB_ATTEMPTS};
use crate::clock::SystemClock;
use ora_application::{
    CleanupStage, Clock, GitCleanupError, RemoveTaskBranchRequest, RemoveTaskWorktreeRequest,
    ResourceRemoval, TaskGitResourceCleaner, WorktreeRemoval,
};
use ora_db::{
    DatabaseBootstrapper, DatabaseLocation, RepositoryPool, SqliteGitCleanupJobRepository,
    SqliteWorktreeProvisioningLeaseRepository, default_migration_catalog,
};
use ora_domain::{
    GitCleanupJob, GitCleanupJobId, GitCleanupJobState, ProjectId, WorkspaceId,
    WorktreeProvisioningLease, WorktreeProvisioningLeaseId,
};
use ora_logging::{with_recorded_trace_logging, with_trace_logging};
use pretty_assertions::assert_eq;
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use tempfile::TempDir;
use tracing::Level;
use tracing_subscriber::layer::{Context, Layer};

const TASK_A: &str = "11111111-2222-3333-4444-555555555555";
const TASK_B: &str = "22222222-2222-3333-4444-555555555555";
const TASK_C: &str = "33333333-2222-3333-4444-555555555555";

/// Names the scripted behavior of one fake cleanup stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StageScript {
    Removed,
    AlreadyAbsent,
    OwnershipLost,
    Fail,
    Panic,
}

/// Scripted cleaner recording invocations per task for worker assertions.
#[derive(Clone, Debug, Default)]
struct ScriptedCleaner {
    scripts: Arc<Mutex<HashMap<String, (StageScript, StageScript)>>>,
    invoked_worktrees: Arc<Mutex<Vec<String>>>,
    invoked_branches: Arc<Mutex<Vec<String>>>,
}

impl ScriptedCleaner {
    /// Scripts both stages for the task owning the given branch name.
    fn script(&self, branch_name: &str, worktree: StageScript, branch: StageScript) {
        self.scripts
            .lock()
            .unwrap()
            .insert(branch_name.to_string(), (worktree, branch));
    }

    fn invoked_worktrees(&self) -> Vec<String> {
        self.invoked_worktrees.lock().unwrap().clone()
    }

    fn invoked_branches(&self) -> Vec<String> {
        self.invoked_branches.lock().unwrap().clone()
    }

    fn stage_error(stage: CleanupStage) -> GitCleanupError {
        GitCleanupError::new(stage, "scripted failure", std::io::Error::other("boom"))
    }
}

impl TaskGitResourceCleaner for ScriptedCleaner {
    fn remove_worktree(
        &self,
        request: RemoveTaskWorktreeRequest,
    ) -> Result<WorktreeRemoval, GitCleanupError> {
        self.invoked_worktrees
            .lock()
            .unwrap()
            .push(request.branch_name.clone());
        let script = self
            .scripts
            .lock()
            .unwrap()
            .get(&request.branch_name)
            .map(|(worktree, _)| *worktree)
            .unwrap_or(StageScript::Removed);
        match script {
            StageScript::Removed => Ok(WorktreeRemoval::Removed),
            StageScript::AlreadyAbsent => Ok(WorktreeRemoval::AlreadyAbsent),
            StageScript::OwnershipLost => Ok(WorktreeRemoval::OwnershipLost),
            StageScript::Fail => Err(Self::stage_error(CleanupStage::Worktree)),
            StageScript::Panic => panic!("scripted worktree panic"),
        }
    }

    fn remove_branch(
        &self,
        request: RemoveTaskBranchRequest,
    ) -> Result<ResourceRemoval, GitCleanupError> {
        self.invoked_branches
            .lock()
            .unwrap()
            .push(request.branch_name.clone());
        let script = self
            .scripts
            .lock()
            .unwrap()
            .get(&request.branch_name)
            .map(|(_, branch)| *branch)
            .unwrap_or(StageScript::Removed);
        match script {
            StageScript::Removed => Ok(ResourceRemoval::Removed),
            StageScript::AlreadyAbsent => Ok(ResourceRemoval::AlreadyAbsent),
            StageScript::OwnershipLost => unreachable!("branch stage has no ownership outcome"),
            StageScript::Fail => Err(Self::stage_error(CleanupStage::Branch)),
            StageScript::Panic => panic!("scripted branch panic"),
        }
    }
}

/// Builds an isolated SQLite pool for one worker test.
fn bootstrapped_pool() -> (TempDir, RepositoryPool) {
    let temp_dir = TempDir::new().expect("create temp dir");
    let pool = DatabaseBootstrapper::system()
        .bootstrap_repository_pool(
            &DatabaseLocation::path(temp_dir.path().join("ora.sqlite3")),
            &default_migration_catalog().expect("catalog"),
        )
        .expect("bootstrap pool");
    (temp_dir, pool)
}

/// Builds a worker over the scripted cleaner without spawning its thread.
fn worker(pool: &RepositoryPool, cleaner: ScriptedCleaner) -> GitCleanupWorker<ScriptedCleaner> {
    GitCleanupWorker::with_cleaner(
        pool.clone(),
        Arc::new(RwLock::new(PathBuf::from("/tmp/ora-worktrees"))),
        SystemClock,
        cleaner,
    )
}

/// Derives the branch matching one full workspace-id constant.
fn branch_for(workspace_id: &str) -> String {
    format!("ora/{}", &workspace_id[..8])
}

/// Seeds one immediately-due pending job with a valid identity.
fn seed_job(pool: &RepositoryPool, workspace_id: &str, attempts: i64) -> GitCleanupJob {
    let mut job = GitCleanupJob::pending(
        GitCleanupJobId::new(format!("job-{workspace_id}")),
        ProjectId::new("project-1"),
        WorkspaceId::new(workspace_id),
        "/repos/project-1",
        None,
        branch_for(workspace_id),
        /*now*/ 0,
    );
    job.attempts = attempts;
    SqliteGitCleanupJobRepository::new(pool.clone())
        .insert_job(&job)
        .expect("seed job");
    job
}

/// Returns every job keyed by workspace id for whole-state assertions.
fn jobs_by_workspace(pool: &RepositoryPool) -> HashMap<String, GitCleanupJob> {
    SqliteGitCleanupJobRepository::new(pool.clone())
        .list_jobs()
        .expect("list jobs")
        .into_iter()
        .map(|job| (job.workspace_id.to_string(), job))
        .collect()
}

/// Verifies confirmed removals and confirmed absences both complete the job.
#[test]
fn completes_jobs_for_removed_and_already_absent_resources() {
    let (_temp, pool) = bootstrapped_pool();
    let cleaner = ScriptedCleaner::default();
    cleaner.script(
        &branch_for(TASK_A),
        StageScript::Removed,
        StageScript::Removed,
    );
    cleaner.script(
        &branch_for(TASK_B),
        StageScript::AlreadyAbsent,
        StageScript::AlreadyAbsent,
    );
    seed_job(&pool, TASK_A, 0);
    seed_job(&pool, TASK_B, 0);

    with_trace_logging(|| worker(&pool, cleaner.clone()).run_pass());

    let jobs = jobs_by_workspace(&pool);
    assert_eq!(jobs[TASK_A].state, GitCleanupJobState::Completed);
    assert_eq!(jobs[TASK_B].state, GitCleanupJobState::Completed);
    let mut branches = cleaner.invoked_branches();
    branches.sort();
    assert_eq!(branches, vec![branch_for(TASK_A), branch_for(TASK_B)]);
}

/// Verifies one panicking job neither aborts the pass nor blocks sibling jobs.
#[test]
fn sibling_jobs_continue_after_a_cleaner_panic() {
    let (_temp, pool) = bootstrapped_pool();
    let cleaner = ScriptedCleaner::default();
    cleaner.script(
        &branch_for(TASK_B),
        StageScript::Panic,
        StageScript::Removed,
    );
    seed_job(&pool, TASK_A, 0);
    seed_job(&pool, TASK_B, 0);
    seed_job(&pool, TASK_C, 0);

    with_trace_logging(|| worker(&pool, cleaner.clone()).run_pass());

    let jobs = jobs_by_workspace(&pool);
    // Both siblings of the panicking job completed.
    assert_eq!(jobs[TASK_A].state, GitCleanupJobState::Completed);
    assert_eq!(jobs[TASK_C].state, GitCleanupJobState::Completed);
    // The panic became an ordinary retryable failure with backoff.
    assert_eq!(jobs[TASK_B].state, GitCleanupJobState::Pending);
    assert_eq!(jobs[TASK_B].attempts, 1);
    assert!(
        jobs[TASK_B]
            .last_error
            .as_deref()
            .unwrap()
            .contains("panic")
    );
    assert!(jobs[TASK_B].next_attempt_at > jobs[TASK_B].created_at);
    // All three targets entered the cleaner.
    let mut worktrees = cleaner.invoked_worktrees();
    worktrees.sort();
    assert_eq!(
        worktrees,
        vec![branch_for(TASK_A), branch_for(TASK_B), branch_for(TASK_C)]
    );
}

/// Verifies a worktree failure still attempts the branch stage before retrying.
#[test]
fn branch_stage_runs_after_worktree_failure() {
    let (_temp, pool) = bootstrapped_pool();
    let cleaner = ScriptedCleaner::default();
    cleaner.script(&branch_for(TASK_A), StageScript::Fail, StageScript::Removed);
    seed_job(&pool, TASK_A, 0);

    with_trace_logging(|| worker(&pool, cleaner.clone()).run_pass());

    assert_eq!(cleaner.invoked_branches(), vec![branch_for(TASK_A)]);
    let jobs = jobs_by_workspace(&pool);
    assert_eq!(jobs[TASK_A].state, GitCleanupJobState::Pending);
    assert_eq!(jobs[TASK_A].attempts, 1);
}

/// Verifies retry exhaustion parks a job for manual attention.
#[test]
fn exhausted_retries_park_as_manual_attention() {
    let (_temp, pool) = bootstrapped_pool();
    let cleaner = ScriptedCleaner::default();
    cleaner.script(&branch_for(TASK_A), StageScript::Fail, StageScript::Removed);
    seed_job(&pool, TASK_A, MAX_JOB_ATTEMPTS - 1);

    with_trace_logging(|| worker(&pool, cleaner).run_pass());

    let jobs = jobs_by_workspace(&pool);
    assert_eq!(jobs[TASK_A].state, GitCleanupJobState::ManualAttention);
}

/// Verifies ownership loss parks immediately and never counts as absence.
#[test]
fn ownership_loss_parks_without_removal() {
    let (_temp, pool) = bootstrapped_pool();
    let cleaner = ScriptedCleaner::default();
    cleaner.script(
        &branch_for(TASK_A),
        StageScript::OwnershipLost,
        StageScript::Removed,
    );
    seed_job(&pool, TASK_A, 0);

    with_trace_logging(|| worker(&pool, cleaner.clone()).run_pass());

    let jobs = jobs_by_workspace(&pool);
    assert_eq!(jobs[TASK_A].state, GitCleanupJobState::ManualAttention);
    // The independent branch stage still executed.
    assert_eq!(cleaner.invoked_branches(), vec![branch_for(TASK_A)]);
}

/// Verifies corrupted identity is parked without any Git invocation.
#[test]
fn identity_violation_parks_without_touching_git() {
    let (_temp, pool) = bootstrapped_pool();
    let job = GitCleanupJob::pending(
        GitCleanupJobId::new("job-bad"),
        ProjectId::new("project-1"),
        WorkspaceId::new(TASK_A),
        "/repos/project-1",
        None,
        // Mismatched branch: does not equal ora/<task prefix>.
        "ora/deadbeef",
        0,
    );
    SqliteGitCleanupJobRepository::new(pool.clone())
        .insert_job(&job)
        .expect("seed job");
    let cleaner = ScriptedCleaner::default();

    with_trace_logging(|| worker(&pool, cleaner.clone()).run_pass());

    let jobs = jobs_by_workspace(&pool);
    assert_eq!(jobs[TASK_A].state, GitCleanupJobState::ManualAttention);
    assert!(cleaner.invoked_worktrees().is_empty());
    assert!(cleaner.invoked_branches().is_empty());
}

/// Verifies an expired provisioning lease is reclaimed into an executed job.
#[test]
fn expired_lease_is_reclaimed_and_cleaned() {
    let (_temp, pool) = bootstrapped_pool();
    let lease_repository = SqliteWorktreeProvisioningLeaseRepository::new(pool.clone());
    lease_repository
        .create_lease(&WorktreeProvisioningLease::new(
            WorktreeProvisioningLeaseId::new("lease-1"),
            ProjectId::new("project-1"),
            WorkspaceId::new(TASK_A),
            "/repos/project-1",
            "/tmp/ora-worktrees/task-a",
            branch_for(TASK_A),
            /*lease_expires_at*/ 1,
            /*now*/ 1,
        ))
        .expect("seed lease");
    let cleaner = ScriptedCleaner::default();

    with_trace_logging(|| worker(&pool, cleaner.clone()).run_pass());

    assert_eq!(lease_repository.list_leases().expect("list leases"), vec![]);
    let jobs = jobs_by_workspace(&pool);
    assert_eq!(jobs[TASK_A].state, GitCleanupJobState::Completed);
    assert_eq!(
        jobs[TASK_A].checkout_root.as_deref(),
        Some("/tmp/ora-worktrees/task-a")
    );
    assert_eq!(cleaner.invoked_worktrees(), vec![branch_for(TASK_A)]);
}

/// Verifies a live (unexpired) lease is never reclaimed by a pass.
#[test]
fn live_lease_is_not_reclaimed() {
    let (_temp, pool) = bootstrapped_pool();
    let lease_repository = SqliteWorktreeProvisioningLeaseRepository::new(pool.clone());
    let far_future = SystemClock.now_timestamp_millis() + 600_000;
    lease_repository
        .create_lease(&WorktreeProvisioningLease::new(
            WorktreeProvisioningLeaseId::new("lease-1"),
            ProjectId::new("project-1"),
            WorkspaceId::new(TASK_A),
            "/repos/project-1",
            "/tmp/ora-worktrees/task-a",
            branch_for(TASK_A),
            far_future,
            1,
        ))
        .expect("seed lease");
    let cleaner = ScriptedCleaner::default();

    with_trace_logging(|| worker(&pool, cleaner.clone()).run_pass());

    assert_eq!(
        lease_repository.list_leases().expect("list leases").len(),
        1
    );
    assert!(cleaner.invoked_worktrees().is_empty());
}

/// Captures failure-path events so field parity can be asserted per path.
#[derive(Clone, Debug, Default)]
struct FailureFieldRecorder {
    events: Arc<Mutex<Vec<(Level, BTreeSet<String>)>>>,
}

impl FailureFieldRecorder {
    fn layer(&self) -> FailureFieldLayer {
        FailureFieldLayer {
            events: self.events.clone(),
        }
    }

    /// Returns the field-name sets of recorded task-resource failure events.
    fn failure_field_sets(&self) -> Vec<BTreeSet<String>> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(level, fields)| {
                (*level == Level::WARN || *level == Level::ERROR)
                    && fields.contains("cleanup_stage")
            })
            .map(|(_, fields)| fields.clone())
            .collect()
    }
}

#[derive(Clone, Debug)]
struct FailureFieldLayer {
    events: Arc<Mutex<Vec<(Level, BTreeSet<String>)>>>,
}

impl<S> Layer<S> for FailureFieldLayer
where
    S: tracing::Subscriber,
{
    /// Records each event's level and complete field-name set.
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, S>) {
        let fields = event
            .metadata()
            .fields()
            .iter()
            .map(|field| field.name().to_string())
            .collect();
        self.events
            .lock()
            .unwrap()
            .push((*event.metadata().level(), fields));
    }
}

/// Verifies Err and panic failure paths log the same structured field set.
#[test]
fn error_and_panic_paths_log_identical_fields() {
    let (_temp, pool) = bootstrapped_pool();
    let cleaner = ScriptedCleaner::default();
    cleaner.script(&branch_for(TASK_A), StageScript::Fail, StageScript::Removed);
    cleaner.script(
        &branch_for(TASK_B),
        StageScript::Panic,
        StageScript::Removed,
    );
    seed_job(&pool, TASK_A, 0);
    seed_job(&pool, TASK_B, 0);
    let recorder = FailureFieldRecorder::default();

    with_recorded_trace_logging(recorder.layer(), || {
        worker(&pool, cleaner).run_pass();
    });

    let field_sets = recorder.failure_field_sets();
    assert_eq!(field_sets.len(), 2, "one failure event per failed job");
    let expected: BTreeSet<String> = [
        "message",
        // Added automatically by the ora-logging wrapper macros.
        "method",
        "operation",
        "cleanup_stage",
        "job_id",
        "project_id",
        "workspace_id",
        "branch_name",
        "attempts",
        "error",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    assert_eq!(field_sets[0], expected);
    assert_eq!(field_sets[1], expected);
}
