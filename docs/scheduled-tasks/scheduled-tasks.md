# Scheduled Tasks

English | [中文](scheduled-tasks.zh.md)

A scheduled task is a persisted, revisioned definition that runs a prompt against a project on a
schedule: a daily report, a recurring code-quality check, a to-do sweep. The governing decision is
the [local and cloud execution ownership](https://github.com/ora-space/specs/blob/main/decisions/desktop/core/scheduled-task/0-local-and-cloud-execution-ownership.md)
ADR (`approved`). Code lives in `crates/domain/src/scheduled_task.rs` and `scheduled_run.rs`,
`crates/application/src/scheduled_task`, `crates/backend/src/scheduled_task.rs` and the
`scheduled_tasks`, `scheduled_runs` and `scheduled_occurrences` tables (migration `0015`), the
verdict columns (migration `0020`) and the artifact manifest tables (migration `0021`).

## What is delivered

The definition use cases, history reads and the local scheduler are wired into Desktop. Occurrence
admission (`AdmitScheduledOccurrenceHandler`) and offline-gap recording
(`RecordOfflineMissedOccurrenceHandler`) are implemented in `ora-application` and composed into the
Backend: a `local` task triggers on schedule while Desktop is running, and due points that passed while
it was closed are recorded `missed` without backfill. Creating or updating a `cloud` task is rejected
as an invalid request.

| Operation                                | Effect                                                             |
| ---------------------------------------- | ------------------------------------------------------------------ |
| `createScheduledTask`                    | Validate and persist a new `local` task                            |
| `getScheduledTask`, `listScheduledTasks` | Read visible definitions                                           |
| `updateScheduledTask`                    | Replace editable fields, guarded by the exact snapshot it replaces |
| `deleteScheduledTask`                    | Soft-delete the definition; run and missed history is kept         |
| `revokeScheduledTaskApproval`            | Withdraw unattended consent and disable the task                   |
| `listScheduledTaskHistory`               | Newest-first runs and missed occurrences                           |
| `listScheduledRunArtifacts`              | One run's collected artifacts and rejected candidates              |
| `readScheduledRunArtifact`               | Bounded text preview of one stored artifact                        |

## Definition

A `ScheduledTask` has a name, an `enabled` flag, a schedule, an execution location, a target project,
an agent (`AgentRef`) with an optional model, a prompt, a revision and an optional approval.

- **Schedule.** `EveryHours { interval }` (at least 1), `Daily { hour, minute }` or
  `Weekly { weekdays, hour, minute }` with at least one weekday, all in local wall-clock time.
- **Execution location.** `local` or `cloud`, fixed at creation; to move a task, copy it and disable
  the old one.
- **Target.** An enabled task's project needs an admissible local Main Workspace that is a Git
  repository, checked with the same probe interactive task creation uses. Validation failures are
  invalid requests, and a concurrent modification returns `scheduled_task_conflict`. Creating a task
  (or moving it to another project) also freezes the worktree base branch; a project without a
  persisted default branch (every project added as a folder) resolves it from the main
  workspace's HEAD, so creating even a disabled task for a folder that is not a Git repository,
  or moving a task to one, is rejected as an invalid request. An existing disabled task keeps
  its frozen branch and can still be edited after its folder stops being a repository.
- **Approval.** Unattended local execution requires explicit consent to the only supported policy,
  `full_auto_in_worktree`, recorded with its consent time. A task can be enabled only with that
  consent; revoking it disables the task.
- **Revision.** Every update that changes the frozen inputs increments `revision`, so a run can always
  name the definition it executed.

Creation and update validate and freeze the definition first (`prepare`) and persist it separately,
so the adapter can check live runtime eligibility (such as agent availability) before committing.

## Runs and history

The domain fixes the semantics the scheduler will follow:

- Each planned trigger is a `ScheduledOccurrence` identified by task and scheduled time,
  independently of the revision that produced it. Admitting it is idempotent and freezes a
  `ScheduledRunSnapshot` (revision, project, agent, model, prompt, approval).
- A live observation may arrive at most ten minutes late. A later observation, or a time that passed
  while Desktop was not running, is recorded as a missed occurrence (`late_trigger`,
  `app_not_running`) rather than a failed run. After a long shutdown only the latest due time per task
  is recorded, and a time whose ten-minute window is still open at startup is left for the live
  scheduler.
- A run moves forward through `accepted`, `running`, `succeeded`, `failed` or `cancelled`. An
  occurrence that cannot start because the task is still running or because four local runs are
  already active becomes `skipped_busy` or `skipped_capacity`. Failure reasons are
  `target_unavailable`, `model_unavailable`, `worktree_failed`, `approval_policy_invalid`, `timeout`
  and `execution_error`.
- History keeps at most 100 run and missed entries per task, and deleting a task does not delete them.

## Verdicts and run artifacts

A run's business verdict is independent of its execution status: a `succeeded` run can report `fail`.
The runtime parses `QA_VERDICT: <pass|warn|fail|blocked|idle|range_unresolved>` (optionally followed
by `QA_DETAIL: ...`) from the agent's final Assistant turn and writes it once, before `RunFinished`.
Runs that never use the convention, failed/cancelled/timeout runs and old history read as
"not reported".

Artifacts use the fixed `.ora-scheduled-artifacts/` directory in the run's worktree. On a
`succeeded` completion, before the janitor may discard the worktree, the runtime copies whitelisted
files (`markdown`, `json`, `txt`, `log`, `xml`, `html`) into
`<data root>/scheduled-artifacts/<run id>/`. The directory is excluded from the worktree change
check, so a run that only wrote artifacts is still reclaimed. Per-run bounds are 64 artifacts,
10 MiB each and 64 MiB total; rejected candidates stay visible in history with their reason. Each
task keeps artifacts for at most ten runs (newest first), runs that still retain their worktree are
exempt, and pruning history removes the matching artifact rows and files.

## Leftover worktree tasks from older versions

Before the [worktree ownership commit](https://github.com/ora-space/specs/blob/main/decisions/desktop/core/scheduled-task/20261020-worktree-ownership-atomic-commit.md)
ADR (`approved`), a crash between provisioning and run start could leave an isolated worktree task that
no scheduled cleanup path could reclaim. Those tasks show up as ordinary tasks in the task list and are
never deleted automatically. To clean one up manually:

1. Look for a task titled `Scheduled run <UUID>` that has no linked scheduled run in its history.
2. Delete it from the task list as you would any task; the regular task cascade removes its worktree and
   branch.

The title is only a recognition hint, not a proof of ownership, so never delete a task on that pattern
alone if you are unsure where it came from. New runs no longer produce these leftovers; if such a task
cannot be seen or deleted in the UI, report it as a separate issue.
