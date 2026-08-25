# git_cleanup

Durable Git cleanup execution for aggregate deletion.

## Responsibilities

- Own the single in-process worker that consumes persisted `git_cleanup_jobs`
  and expired `worktree_provisioning_leases`, executes the physical worktree
  and branch removals through `ora_application::TaskGitResourceCleaner`, and
  records per-job state transitions (`pending` → `completed` /
  `manual_attention`).
- Provide the process-wide coordination primitives around physical checkouts:
  - the **worktree-use lease** (`KeyedResourceLocks`, keyed by Workspace id):
    consumers of a Workspace checkout acquire shared use, physical cleanup acquires
    exclusive use;
  - the **repository mutation gate** (keyed by normalized repository root):
    serializes `git worktree add/remove` and branch mutations per repository
    between provisioning and cleanup.
- Expose `GitCleanupHandle` so deletion paths can wake the worker after their
  transaction commits and so consumers can acquire shared use leases.

## Non-responsibilities

- Deciding _what_ to clean: cleanup identity is captured by the database
  cascade (`ora-db`) in the same transaction as the soft deletes.
- Interpreting Git outcomes: identity validation, outcome reduction, and the
  cleaner implementation live in `ora-application`; this module only drives
  them and persists the resulting transitions.

## Key invariants

- Jobs have no persisted "running" state. The worker executes jobs while they
  stay `pending`, so a crash at any point leaves them replayable; the first
  pass after start is the reconciliation.
- The wake signal is a coalesced latency optimization only; the SQLite queue
  is the source of truth and the periodic scan (60s) recovers lost wake-ups.
- One batch pulls at most 16 jobs, grouped by repository; groups run
  concurrently under a global cap (4), jobs within one repository run
  serially. Retries follow a bounded backoff schedule and park as
  `manual_attention` after 5 failed attempts.
- Lock order is fixed: worktree-use lease first, then repository mutation
  gate. Neither is ever held across a database transaction wait.
- Long-lived checkout holders (running agent sessions, active workflow runs)
  deliberately do **not** hold use leases: deletion already refuses aggregates
  with running sessions or active runs at the database gate, and new
  admissions are rejected by row visibility. The lease protects short-lived
  readers (diff, commit, push, spec reads) that resolved a checkout path
  before a deletion committed.

## Failure semantics

- A worker pass never fails startup; every persistence or Git error is logged
  with `operation = "git_cleanup"` plus job identity fields and converted into
  the job's own retry/manual-attention bookkeeping.
- A cleaner panic is caught per job, logged with the same fields as ordinary
  failures, and treated as a retryable failure; sibling jobs continue.
- Process death mid-pass loses nothing: unfinished jobs remain `pending` and
  expired leases are reclaimed on the next start.
