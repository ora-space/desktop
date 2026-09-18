# Node Worktree execution and persistence plan

English | [中文](worktree-execution.zh.md)

> 2026-09-18: the current minimal loop is now [cloning a specified repository and branch](../minimal-loop.md).
> This document retains implemented Worktree behavior and its original plan, not the next implementation sequence.

This change implements Worktree execution and recovery in `apps/ora-node`, with independent local
persistence in `crates/node-db`. Its in-process interface closes creation, removal, status, replay,
and acknowledgement. Completed items are checked below; implementation choices and direct test
evidence appear in section 7.

## Current integration boundary

The `node-db` work is now imported into `node-process`, retaining the current workspace SQLite
dependency and the existing `home_directory/ora-node.sqlite3` layout. Node business records are not
merged into `host.sqlite` or `guardian.sqlite`.

R2 now treats a valid owned checkout with new commits as successful creation; its original
`base_commit` remains the comparison baseline. R5 now completes a new cleanup request against retired
ownership with a WorktreeConflict failure when resources have reappeared, without touching them.
R3 tests retain unacknowledged creation and deletion events across reopen and acknowledge them independently.

R1 now admits nonconflicting work while retaining uncertain resource reservations. An owned branch-only
creation at the frozen base is cleaned and retried once per recovery pass under its original execution.
Changed branch content, other checkouts and nonempty unexplained residuals remain protected.
R4 now routes Git through the [process host](../../process/host/service.md). Node persists mutation
Scope/Run associations before dispatch, closes original attempts and verifies cleanup before resource
reconciliation. Guardian independently stops an owned Run on Node exit. Unavailable guardian evidence
keeps conflicting work blocked; BestEffort is not a Strong quiescence guarantee.
The [standalone Node](../runtime.md) supplies startup recovery and bounded shutdown, without Controller IPC.

Node IPC and Controller result takeover remain later steps. Trusted-local operation is the current
scope; authentication, secret tokens, leases and privileged Strong containment are deferred, not
prerequisites to this loop. Existing Backend writers are unchanged. No ADR status is changed here.

This is step 2 of the local Worktree loop: `feat(node): execute worktree operations with durable
recovery`. The preceding step supplied protocol messages and framing; the next step adds local IPC
and Controller integration. Standalone Node lifecycle is implemented in this slice. Git side effects and durable deduplication ship together.

## 1. Goal, decisions and scope

A Node bound to an existing main worktree accepts EnsureWorktree and RemoveWorktree, reports execution
evidence, and retains the original execution identity across lost responses and restarts. Input must
be durable before Git changes. Inconclusive side effects remain recoverable.

The implementation follows the [Node Worktree decision](../../../specs/decisions/node/worktree/0-worktree-management.md),
[Controller–Node Protocol decision](../../../specs/decisions/node/protocol/0-controller-node-protocol.md),
[current protocol](../../protocols/controller-node-protocol.zh.md), and
[task cleanup behavior](../../task-worktrees.md). Related ADRs remain `proposed` until the complete
local loop lands on main.

| Delivered here                                                    | Subsequent work                                                                |
| ----------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| Library and executable initialization, execution and recovery     | Controller IPC and handshake; authentication deferred                          |
| Independent Node SQLite schema and transactions                   | Controller business storage and scheduling                                     |
| Injected repository/Main Workspace bindings and authorized roots  | Registration and binding management                                            |
| Creation, removal, status, replay enumeration and acknowledgement | Transport delivery and Controller durable takeover                             |
| Local reconciliation and focused verification                     | Clone, fetch, push, migration, file transfer and switching old Backend writers |

External Git commands are not guaranteed exactly once. A process can stop after Git changes but
before the result commits. Recovery must reconcile facts; an execution row, command exit status,
or existing path alone cannot establish success.

## 2. Code and dependency boundaries

| Package                          | Responsibility                                                                                                     |
| -------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| `ora-node` (`apps/ora-node`)     | Initialization, validation, resource resolution, execution, recovery and protocol mapping                          |
| `ora-node-db` (`crates/node-db`) | Node identity, guarded acceptance/transitions, ownership, atomic results, recovery scans and exact acknowledgement |
| `gitlancer`                      | Typed Git operations, repository and checkout facts; no Ora execution policy                                       |
| `ora-utils`                      | Existing portable path and canonical containment capabilities                                                      |
| `ora-node-protocol`              | Existing messages and reusable semantic validation; no database dependency                                         |

Dependencies flow from Node to Node DB, protocol, Git and utilities. Node DB may use protocol types
but does not depend on Node, `ora-db`, Controller, Git or transport. It does not reuse Controller
business tables or migrations. Modules are private with explicit public exports; callers do not
compose SQL transactions or cleanup stages.

Git, storage failure boundaries and local time are injected through traits and generics. Persistence
tests reopen real temporary SQLite files; real repositories verify the production Git adapter.

## 3. Implementation sequence

### P1: Packages and initialization

- [x] Add both packages to workspace members, defaults and dependencies.
- [x] Accept explicit `home_directory`, repository/Main Workspace bindings, authorized roots and managed worktree roots.
- [x] Deployment explicitly supplies an absolute data directory; database name is `ora-node.sqlite3` inside it. No HOME-derived default is used.
- [x] Persist NodeId and generate a fresh NodeIncarnationId on each open; reject identity mismatches.
- [x] Hold one execution lease per database and serialize mutations and recovery.
- [x] Distinguish initialization errors, RecoveryPending and Ready; repeated recovery cannot bypass incomplete evidence.

### P2: Independent SQLite transactions

- [x] Validate Node schema identity/version and table/index definitions. Reject directories, foreign databases, empty existing files, corruption and unsupported schemas without replacement.
- [x] Store complete input separately from resolved targets, execution progress, ownership, results and replay metadata.
- [x] Atomically accept or read existing executions, uniquely bind operation/execution identities, and reserve resource identities, workspace, path and branch before Git.
- [x] Deletion references existing ownership; accepting a delete never creates ownership.
- [x] Guard transitions and atomically commit resource facts, terminal result and original event.
- [x] Query, scan recoverable records, enumerate pending events and precisely acknowledge delivery while retaining deduplication and results.

Targets freeze repository/Main Workspace paths, Git metadata directory, authorized and worktree roots,
actual branch and immutable base commit. Retransmission does not resolve a moved base ref or replace a
saved path. Opaque identities retain exact values; empty values are rejected without normalizing
nonempty identifiers. Git never runs inside a SQLite transaction.

### P3: Resource resolution and ownership

- [x] Match explicit repository and Main Workspace registration; verify an existing main checkout with Git.
- [x] Validate NodeManaged.directory_name as one portable directory segment, rejecting traversal, absolute paths, reserved forms and nested paths.
- [x] Construct paths with Path/PathBuf and shared canonical containment; reject static symbolic links and main-checkout overlap.
- [x] Resolve the base commit before creation; deletion uses retained ownership regardless of moved task commits or base refs.
- [x] Recheck configuration, Git registration and ownership before mutations and recovery; preserve unowned nonempty directories.

Static checks do not promise OS-level isolation against malicious symlink replacement between
validation and use.

### P4: Creation, removal and durable results

- [x] Validate in-process message semantics and deduplicate before accessing current Git or configuration.
- [x] Resolve, durably accept, persist mutation intent, mutate Git, inspect facts, then atomically complete.
- [x] Persist structured precondition failures and events without requiring resolved targets. Reject malformed input and identity conflicts without overwriting old records.
- [x] Create at the frozen path/branch/commit, verify actual checkout facts, and leave ambiguous partial effects for recovery.
- [x] Force-remove only owned linked checkouts and task branches in independently recoverable stages.
- [x] Verify registration, directory and local branch absence before success; remove only owned empty residual directories.
- [x] Retain original terminal envelopes and exact acknowledgement identity.

AlreadyAbsent requires every cleanup target to be absent. A missing checkout with a remaining owned
branch still requires branch cleanup. Main worktrees, other branches and unowned nonempty directories
are never force-cleaned.

### P5: Queries, replay and recovery

- [x] Reconcile all incomplete records, covering accepted input, possible mutations, partial removal and effects completed before the result committed.
- [x] Keep original identities and targets; never choose a new path, branch or base on restart.
- [x] Gate new mutations while recovery remains pending, retaining access to previous results and replay.
- [x] Keep queries free of Git and acknowledgement side effects; enumerate replay independently.
- [x] Validate exact Node, operation, execution and sequence; duplicate exact acknowledgements are idempotent.

### P6: Review evidence and handoff

- [x] Provide direct tests for V1–V12 and document their entry points.
- [x] Synchronize English/Chinese plans, final API, schema/layout and implementation tradeoffs.
- [x] Complete focused tests/lint and full `task test`, recording results and limitations.

## 4. State and recovery rules

| Internal evidence                                        | Protocol status   | Next action                                                    |
| -------------------------------------------------------- | ----------------- | -------------------------------------------------------------- |
| Accepted: input saved, Git not started                   | Accepted          | Persist Running intent before mutation                         |
| Running: mutation might have occurred                    | Running           | Continue active execution; reconcile after restart             |
| Unknown: insufficient evidence retained with diagnostics | Unknown           | Observe again and continue/complete only with sufficient facts |
| Completed(result): result/event committed atomically     | Completed(result) | Return the original result and replay pending delivery         |

An absent execution also reports Unknown, but querying cannot create it or start Git. A retained
Unknown is not a missing record and must not be sealed as Completed(Failed(ResultUnknown)).
Newly recovered results carry the current observing incarnation. Retained results/events preserve
their original incarnation. Status reports the current runtime outside the original result, with
the same persistent NodeId.

| Recovery observation                                                                      | Required action                                                                    |
| ----------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| No effects and safe retry is proven                                                       | Resume the original identity and frozen target                                     |
| Valid owned checkout, registration, path and expected branch match, including new commits | Complete creation and retain original base                                         |
| Checkout removed, owned local branch remains                                              | Continue verified branch cleanup                                                   |
| Every removal target absent                                                               | Complete with AlreadyAbsent                                                        |
| Definite failure without unexplained effects                                              | Persist structured terminal failure                                                |
| Owned branch only at the frozen base, no conflicting checkout or unexplained files        | Persist cleanup intent, remove residual and retry original execution once per pass |
| Inconsistent registration/directory, changed binding or failed inspection                 | Retain Unknown with diagnostics                                                    |
| Terminal result already committed but response/ack lost                                   | Return/replay original evidence without Git                                        |

Changed operation type/input under one execution, or rebinding an operation to another execution,
returns IdentityConflict and preserves old data. RequestId is correlation metadata, not execution
identity. Create and delete use different operation/execution identities for the same resource.

## 5. Acceptance matrix

| ID  | Required evidence                                                                                                                           |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| V1  | Identity survives reopen, incarnation changes, foreign storage is preserved, concurrent runtimes are excluded                               |
| V2  | Accept/Running write failures prevent Git; completion/event failures roll back atomically and remain recoverable                            |
| V3  | Sequential/concurrent retries mutate once; input and identity conflicts preserve original records; reservations prevent resource collisions |
| V4  | Wrong Node/repository/Main Workspace, invalid names, symlinks, overlap and other resources fail before mutation                             |
| V5  | Creation returns Git facts; moved refs/configuration and lost responses cannot replace retained input/results                               |
| V6  | Stops after acceptance, Running and add recover only with sufficient evidence; owned branch-only effects clean up and retry                 |
| V7  | Removal verifies checkout and branch cleanup; missing checkout alone is insufficient for AlreadyAbsent                                      |
| V8  | Restart after checkout removal continues only owned branch cleanup; branch failure never reports removal success                            |
| V9  | Main/foreign resources remain safe; owned dirty checkouts and advanced task branches still use force cleanup                                |
| V10 | Replay preserves original result, sequence and incarnation; status identifies the current reporting incarnation                             |
| V11 | Queries do not stop replay; exact duplicate ack is idempotent; wrong identities/sequences preserve delivery; ack retains result             |
| V12 | Recovery and commands serialize; Unknown blocks conflicting resources only; repeated recovery preserves identity and unrelated evidence     |

## 6. Completion criteria

- [x] P1–P6 and V1–V12 have passed verification for this Node slice, not Controller end-to-end acceptance.
- [x] Reviewers can trace requests through acceptance, Git, atomic result, replay/ack and restart recovery.
- [x] Every database/Git failure retains identifiable evidence and prevents identity-based bypass.
- [x] Main-checkout and task ownership are distinct; existing filesystem state is protected and old writers are not switched here.
- [x] IPC can consume the library API without adding persistence, deduplication, branch cleanup or recovery.

## 7. Final API, choices and direct evidence

`Node::open(NodeConfig, ProcessConfig, Shutdown)` uses managed Git and the Ora local clock; the process composition root first
initializes the logging timezone. Tests use `Node::open_with_dependencies(config, git, writes, clock)`
with WorktreeGit, WriteGuard and Clock. Node retains home_directory and does not read HOME.
See [storage layout and schema](storage.md).

- `submit(Command::Ensure(message) | Command::Remove(message))` returns ExecutionStatus.
- `status(&GetExecutionStatusMessage)` returns durable evidence with the current reporting runtime.
- `pending_events()` returns original protocol envelopes; `acknowledge(&EventAckMessage)` confirms an exact event.
- `state()` returns Ready or RecoveryPending; `recover()` repeatedly reconciles incomplete records.
- `identity()`, `node_id()`, `home_directory()` and `repositories()` expose immutable runtime information.

Mutation APIs require `&mut Node`; callers may share it behind a Mutex, with concurrent retransmissions
waiting for the original result. The database's exclusive OS lock prevents another runtime driving
it. Closing explicitly unlocks the file so transient descriptors inherited by concurrently spawned
children do not retain the lock. RecoveryPending reports incomplete executions but is not a global
admission gate. Resource reservations exclude conflicting work while unrelated tasks remain admissible.
Original retries, status, replay, acknowledgement and repeated recovery remain available.

Targets separately retain canonical binding paths, actual Git metadata directory, authorized and
managed roots, task path, branch and immutable base. Every mutation rechecks these facts. Git's main
registration must match the binding; merely running Git successfully inside a directory is
insufficient. Linked metadata is checked through its commondir and gitdir back-reference.

`gitlancer::create_worktree_for_recovery` performs add without implicit cleanup after an observation
failure. Node reconciles those effects. Removal uses typed Force modes and separate checkout,
empty-directory and branch stages. Exact local branch names are unaffected by same-named tags.
Deletion ignores mutable base_ref when matching retained ownership but requires all other resource
identity and target facts. Retired tombstones do not authorize deleting resources later appearing
at the old path. A definitive no-effect creation failure retires its reservation, releasing active
path, branch and Workspace uniqueness while retaining the original input and failed result.
Confirmed reappearing resources under retired ownership now yield terminal WorktreeConflict for a
new cleanup operation; genuine observation failures remain Unknown. Original operation replays return
their unchanged historical results.

Precondition failures atomically retain structured failure and event; malformed messages and
Node/execution identity conflicts are entry rejections. Post-mutation storage failures do not return
success. Unexplained residuals and inspection failures remain Unknown, never a terminal
ResultUnknown failure. Results retain original incarnation and sequence 1; acknowledgement removes
only outbox delivery.

Direct tests live in `crates/node-db/src/tests.rs` and `apps/ora-node/src/tests/`. These are crate unit
tests using real temporary SQLite files and Git repositories, injected failure boundaries, and
Barrier/Mutex/channel synchronization without sleep or process environment mutation.

| Acceptance | Test names (common module prefixes omitted)                                                                                                                                                                                                                                           |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| V1         | `identity_and_exclusive_owner_survive_reopen`, `preserves_foreign_corrupt_empty_and_future_files`, `rejects_modified_schema_without_migrating_or_rebuilding`, `injected_home_persists_node_but_not_incarnation`                                                                       |
| V2         | `completion_and_outbox_rollback_together_and_ack_retains_result`, `sqlite_failures_gate_mutations_and_recover_on_reopen`                                                                                                                                                              |
| V3         | `atomic_acceptance_deduplicates_and_reserves_resources`, `duplicate_inputs_and_resource_conflicts_preserve_original_result`, `concurrent_retransmissions_mutate_once`                                                                                                                 |
| V4         | `invalid_paths_bindings_and_unowned_resources_never_mutate_git`, `existing_targets_and_main_workspace_are_protected`, `static_symlink_escape_is_rejected` (Unix), `inconsistent_main_registration_is_rejected`, `main_checkout_overlap_is_rejected_even_for_a_distinct_task_identity` |
| V5         | `real_create_remove_replay_and_acknowledgement`, `completed_retry_ignores_moved_base_and_missing_configuration`                                                                                                                                                                       |
| V6         | `sqlite_failures_gate_mutations_and_recover_on_reopen`, `process_stops_before_and_after_create_keep_frozen_identity_and_base`, `branch_only_creation_recovers_and_preserves_other_results`, `changed_checkout_and_unavailable_git_keep_unknown_evidence`                              |
| V7         | `real_create_remove_replay_and_acknowledgement`, `missing_checkout_still_cleans_branch_and_empty_owned_directory`                                                                                                                                                                     |
| V8         | `partial_removal_continues_only_the_owned_branch`                                                                                                                                                                                                                                     |
| V9         | `existing_targets_and_main_workspace_are_protected`, `real_create_remove_replay_and_acknowledgement`, `missing_checkout_still_cleans_branch_and_empty_owned_directory`, `branch_cleanup_uses_exact_local_names_even_with_ambiguous_tags`                                              |
| V10        | `real_create_remove_replay_and_acknowledgement`                                                                                                                                                                                                                                       |
| V11        | `completion_and_outbox_rollback_together_and_ack_retains_result`, `real_create_remove_replay_and_acknowledgement`                                                                                                                                                                     |
| V12        | `recovery_and_new_submission_share_one_mutation_owner`, `branch_only_creation_recovers_and_preserves_other_results`, `configuration_change_keeps_recovery_pending_without_redirecting_mutations`                                                                                      |

Git adapter unit tests also live in `crates/gitlancer/src/git/inspection.rs`.

Validation commands:

```bash
task format
cargo test -p ora-node-db -p ora-node -p gitlancer -p ora-node-protocol
cargo clippy -p ora-node-db -p ora-node -p gitlancer -p ora-node-protocol --all-targets -- -D warnings
task test
```

On integration into `node-process`, `task format`, focused crate tests, all-target Clippy and
the complete `task test` passed locally on Linux. This supersedes the earlier branch's frontend
clipboard-test failure; it is not evidence of a remote macOS or Windows CI run.
`apps/ora-node/src/tests/review.rs` additionally covers R2 task commits before result persistence,
R3 independent creation/deletion acknowledgements, and R5 replacement preservation across completion
write failure and temporarily unavailable Git observations. R1 tests live in `tests/local_recovery.rs`;
real Node SIGKILL/restart, guardian loss and bounded shutdown tests live in `apps/ora-node/tests/standalone.rs`.
Controller IPC, Controller takeover and switching old Backend entry points remain subsequent work.
Static path checks do not defend against malicious TOCTOU replacement; external Git is not exactly
once and recovery relies on durable intent plus verifiable resource facts.
