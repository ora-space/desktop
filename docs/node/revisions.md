# Revisions

English | [中文](revisions.zh.md)

## Whole-workflow delivery

SQLite v14 records a writer-stop fact only after the live actor confirms plugin and restore shutdown. Restart-generated terminal results never fabricate that fact. Missing proof prevents snapshots and normal release; already frozen bytes stay retained. SQLite v13 separately records provider admission, so losing original history cannot turn an established session into `not_started`.

A restarted Node attaches to a fresh binding before resuming delivery transport. The only unfinished work admitted during same-owner reattachment is a registered WorkflowRun delivery whose sessions all have durable stop proof; run, actor, sandbox, generation and persistent Node identity remain unchanged. The original execution permit and frozen plan are retained verbatim. Controller relays those unfinished identities to Cloud, which validates them instead of treating the slot as empty. Other unfinished activities continue to block rebinding.

`DeliverRevisionSpec.workflow` selects the whole-run variant; it excludes the legacy single-session selector and prior-Revision restoration. Cloud supplies every ordered node attempt and its object key. The Node verifies that each registered session is terminal, no actor or restore writer remains, and all sessions share the frozen checkout. It freezes each established session's original bytes plus a versioned manifest before pushing or uploading. An explicit pre-plugin failure may be `not_started`; a missing established history fails delivery rather than shrinking the inventory.

Frozen plans retain the complete object set across restart. Uploads check frozen sizes and digests, request checksum-bound grants only for the fixed keys and upload the manifest last. Controller forwards the inventory verbatim and only dispatches it to a Node advertising `workflow_history_delivery`. Cloud independently verifies all objects before registration. The original Agent JSONL shape is unchanged; a manifest is identified by the explicit workflow variant, never by extension. The [whole-run ADR](https://github.com/ora-space/specs/blob/main/decisions/cloud/workflow/20261025-run-owned-git-delivery-and-history.md) remains `approved` pending complete acceptance.

When an Agent session ends, the Node saves its checkout and history as a **Revision**: it pushes the
final commit to a delivery ref on the platform Git Server and uploads only the sealed session JSONL to
object storage, through a grant Cloud signs. A later run of the same Issue restores that Revision from
the Git Server before its agent starts. No bundle is generated, uploaded or downloaded. The decisions
are the [internal snapshot root decision](https://github.com/ora-space/specs/blob/main/decisions/node/revision/0-internal-snapshot-and-incremental-bundle-upload.md),
whose snapshot commit still applies, and
[push the delivery ref and restore from the Git Server](https://github.com/ora-space/specs/blob/main/decisions/node/revision/20261017-push-delivery-ref-and-restore-from-git-server.md)
(`approved`; it becomes `implemented` after end-to-end acceptance), which replaces its bundle upload and the bundle restore of
[restore prior Revision before session](https://github.com/ora-space/specs/blob/main/decisions/node/revision/20261010-restore-prior-revision-before-session.md).
The Controller's side is in [Agent relay](../controller/agent-relay.md), the grant contract in
[Cloud contract](../controller/cloud-contract.md#revision-upload-checksums).

The Node advertises `revision_delivery` when it has both the `clone` and the `git_access` sections,
because delivery runs Git in clone checkouts with clone's hardened Git environment and pushes with the
git-access identity, and `revision_restore` exactly when it advertises both `agent_session` and
`revision_delivery`.

## Admission

- `controlled_deliver_revision` is the production entry: its runtime permit must match this Node,
  incarnation, operation and execution, checked at admission. A bare `deliver_revision` is accepted
  only on private IPC in a home that never enabled runtime control.
- The input names the ended session, its clone execution, the base commit, a local Revision ref under
  `refs/ora/revisions/`, the Git Server `git_repository_url`, a `delivery_ref` under
  `refs/ora/deliveries/`, the `workspace_ref` and the history object key. The Workspace ref must equal
  the clone's.
- The input is persisted before any work and the delivery recorded as `Running`. Repeating the same
  input only reports state; different input under the same identity is a protocol error.
- Deliveries live in `revision_deliveries` with their single terminal event in `revision_outbox`,
  separate from sessions so session recovery never sees them.

## Preparation

Preparation happens once:

1. The session must have its terminal result and no live actor (`failed{session_not_settled}`), its
   checkout must still be at the delivery's base commit (`failed{checkout_unavailable}`), and its JSONL
   must exist (`failed{history_unavailable}`).
2. The sealed JSONL is copied into `<home>/revision-deliveries/<sha256(execution)>/history.jsonl`.
3. A scratch index in the checkout's Git directory snapshots the worktree (`read-tree HEAD`, `add -A`,
   `write-tree`), as the workload user. If the tree equals `HEAD^{tree}` the final commit is `HEAD`;
   otherwise `commit-tree` adds one commit authored by the session's Git identity and committed by
   `Ora <revision@ora.invalid>`. Hooks, fsmonitor and signing are disabled, and the real index,
   worktree files and branches are not touched. `update-ref` points the local Revision ref at the final
   commit. `.ora-scheduled-artifacts/` is excluded from the staging for every run, manual or scheduled:
   those files are registered as objects with their own visibility and retention, so a snapshot must
   never publish them in a delivery ref. A copy the repository already tracks therefore stops moving
   with later snapshots, while what history already holds is kept as it is.
4. A final commit equal to the base, or for a resumed session to the prior Revision's final commit, is
   declared `unchanged`; any other is `delivered`. Either way the delivery pushes its own delivery ref.
5. The final commit, the history's size and SHA-256 and the declaration are written to the ledger as the
   frozen plan before the first network write. A restart before that prepares again; after it, the same
   plan is reused.

## Push and upload

Git that uses the Node's git-access identity never runs in the checkout. Each push uses a private
transport repository under `<repository_root>/.ora-transport/` that borrows only the checkout's object
store (nested alternates and shared directories are refused, and alternate ref discovery is disabled).
The certificate, key, CA and the delivery execution's `Ora-Execution-Id` header are given only to each
HTTPS command, so checkout configuration cannot redirect it or install credential helpers.

1. Push the frozen commit to `delivery_ref`. Creating it, or finding it already at that commit,
   succeeds; a refused update is `failed{push_rejected}`, and exhausted network or 5xx failures are
   `failed{push_failed}`.
2. Try to fast-forward `workspace_ref` to the same commit, declaring the value the Node ledger last saw
   there. A replay succeeds; a stale value, a refusal or exhausted failures are only logged and never
   fail the delivery.
3. Upload the history: without a valid grant (none yet, expiring within 5 s, refused with 403, or lost by
   a restart) the Node sends `upload_grant_needed` with the frozen SHA-256 of the history, repeats it
   every 2 s and again at once on every new connection, and the delivery stays `Running`. The PUT
   streams the frozen file with every grant header unchanged (including `If-None-Match: *` and
   `x-amz-checksum-sha256`) and never follows redirects. 2xx and 412 count as uploaded; three failed
   attempts end the delivery as `failed{upload_failed}`. Grants live only in memory and are never logged.

Transient Git Server failures get three attempts with 5 and 30 second pauses; 401, 403 and other 4xx
answers except 429, and refused ref updates are not retried and fail as `push_rejected`. Each push attempt may run 30 minutes, independent of `network_timeout_ms`, and
aborts below 1 KiB/s for a minute. Cloud verifies the delivery ref and the history outside its
transaction before registering the Revision.

The terminal `revision_result` (sequence 1) is replayed until acknowledged; the frozen files are removed
once it is durable and leftovers are swept at the next start. Shutdown, Controller disconnection and
cancellation never write a failure; the delivery resumes from its frozen plan after restart.

## Scheduled-artifact uploads

A scheduled run declares its artifacts under `.ora-scheduled-artifacts/`, and Cloud stores that
declaration without ever seeing the Workspace. `deliver_scheduled_artifacts` is therefore its own
execution, run after the session ends and before the Workspace can be released:

- `controlled_deliver_scheduled_artifacts` is the production entry: its runtime permit must match this
  Node, incarnation, operation and execution, checked at admission like every other work kind. The
  input names only the ended session; the Node reads the declaration back from that session's stored
  terminal result, so a resent execution cannot choose a different set of files.
- A missing declaration fails as `declaration_unavailable`; a checkout or reserved directory that
  cannot be read fails as `workspace_unavailable`.
- Cloud owns every object key. The Node asks for grants by declaration index with the declared size
  and digest (`scheduled_artifact_grant_needed`), and Cloud answers one grant per requested artifact in
  the same order (`scheduled_artifact_grants`). Grants live only in memory. Each grant is applied to
  the declaration index it was signed for and any index the upload no longer has outstanding is
  ignored, so a reply crossing a retry or a repeated request cannot fail the exchange.
- Cloud can withdraw an upload's grants for good (`scheduled_artifact_grants_refused`): the run left
  `delivering`, its frozen window has no usable grant left, or no object store is configured. That
  answer cannot change for the execution, so the upload stops asking and settles what it stored
  instead of holding its slot open.
- The Controller dispatches this execution only to a Node that advertises `RevisionDelivery`, because
  it reuses that delivery's memory-only grant relay; the gate is deliberate rather than a missing
  capability.
- This upload may run next to the Revision delivery of the same run, because Cloud dispatches both
  once the session ended. The Node keeps a session exclusive with every run execution and each
  delivery family exclusive with itself, so a second Revision delivery or a second upload is still
  refused with `resource_conflict`.
- Each file is measured again and uploaded to the key its grant names; 2xx and 412 count as stored. A
  measured size or digest that no longer matches the declaration fails that artifact before any PUT,
  because Cloud signed the declared values into the grant. A failed artifact does not stop the
  others. When any artifact was stored, the terminal result reports that subset as delivered so Cloud
  can register successes and record the remaining refusals. Only an attempt that stored nothing ends
  as `failed{upload_failed}`. Empty files use the same measured size/checksum and grant path. The
  Controller preserves the manifest's omitted-refusal count, walk-truncation flag and
  `candidate_limit` reason when forwarding the session terminal result.
- The terminal `scheduled_artifacts_result` (sequence 1) is replayed until acknowledged and shares the
  delivery outbox. A restart reports the attempt it cannot resume as `failed{interrupted}`; Cloud
  receives that original terminal fact and settles the run's delivery against its frozen deadline.

## Restoring a prior Revision

A session with `prior_revision` (its identity, final and base commits, `git_repository_url` and
`delivery_ref`) restores it after resolving the checkout and checking the plugin, and before the checkout
is handed to the workload user or any plugin starts. The repository must be the one the checkout was
cloned from; any other `git_repository_url` ends the session as `agent_failed{prior_revision_unavailable}`
without contacting the Git Server.

1. As the Node, with the session execution's identity, fetch `delivery_ref` into a private transport
   repository that borrows the checkout's objects, negotiating only from the clone's base commit. The
   fetched commit must equal the prior final commit.
2. As the workload user, take the objects from the transport repository into the checkout and move the
   Workspace's local branch to the prior final commit. The clone's base, the repository configuration
   and `refs/remotes/ora/**` are untouched.
3. If the prior base commit is not in the history of the current snapshot, one fixed English note naming
   the restored commit, its base and the branch, and recommending `ora-git fetch upstream`, is appended to
   the first turn.

The fetch and the import share one absolute deadline, retries included. It defaults to five minutes and is
configured in the Node service file as `"restore": { "deadline_ms": 300000 }`. The value must be a positive
integer representing a finite, representable duration; invalid values refuse service startup. Waiting for
the restore Git lock, retries and late host exchanges never reset this cutoff. A missing delivery ref, a 403
or another 4xx other than 401 and 429 ends the session as `agent_failed{prior_revision_base_unavailable}`:
the prior Revision's Git content is out of reach for good. A 401 only says the Node's own git-access
identity did not authenticate, so it ends the session as `agent_failed{prior_revision_unavailable}`,
which a later run may retry, without a retry within the deadline since the same material cannot pass.
Any other fetch, verification or checkout failure, or the deadline, also ends it as
`agent_failed{prior_revision_unavailable}`. No plugin process is
started in either case. A Node stop or an `EndSession` during the restore stops it before its next Git
command and waits for the running one, so the session ends only once the restore no longer writes the
checkout. A restart during a restore ends the session `interrupted`. A transport repository is removed
only once every original command of its restore proved that it stopped; startup cleanup therefore keeps
the repository of every session whose restore is not settled, instead of clearing it before recovery
runs.

Each supervised restore Git command, at either identity, receives the earlier of its own command
deadline and the restore cutoff. No new command starts after expiry; a pending host exchange is
bounded by that same cutoff, so it cannot defer the close request until a longer transport timeout.
Cleanup and original Scope closure use their independent cleanup budget after execution expires. A
late ancestor check cannot turn an expired restore into success.

**Stop confirmation.** Every restore Git intent — the Node's transport repository creation, Git Server
fetch and `commit_of`, and the workload user's import — is durably associated with its session before
host admission. The first stop request is persisted before Close; cancellation observes command boundaries
and does not abandon the blocking worker. Original Scope `Closed` plus either original Run direct-process
exit or host-authoritative `NotStarted` permits a terminal result, transport cleanup, delivery or checkout
reuse. Non-launch proof requires matching original Scope/Run identities, `LaunchFact::NotStarted`,
`DirectProcessState::NotStarted` and completed cleanup together. The host rejects any non-launch report
that contradicts its persisted confirmed launch, including after restart. A record that the host cannot
find (`UnknownIdentity`) remains unknown and never grants release.

Host-authoritative non-launch comes from a platform-confirmed spawn failure or cancellation before
guardian acceptance. The latter requires the current host binding to fence old requests and no Start
after Stop; an unknown guardian Run without those conditions is not proof. Linux guardian containment
is BestEffort; cancellation reports `BestEffortComplete`, which does not claim Strong quiescence.
See the [approved NotStarted extension](https://github.com/ora-space/specs/blob/main/decisions/node/revision/20261021-stop-confirmation-accepts-not-started.md).

If proof is unavailable, the original identities, inputs, terminal intention and checkout conflict
remain durable. `stop_confirmation_ms` defaults to ten minutes, must be positive, finite and greater
than `deadline_ms`, and is frozen at the first stop request. Retry, restart, reconnection and changed
configuration cannot renew it. At expiry the session stops querying host evidence and the existing
execution-status response exposes `stop_unconfirmed`, including original Scope/Run, first-request and
bound timestamps and the selected containment. Expiry emits no terminal and grants no cleanup or reuse.
Startup handover and later execution queries can retry closing and querying those same identities;
late proof settles the original execution once. A pre-upgrade restore that was still in flight remains
unknown. Completed old terminals stay as history and do not fence later admissions, and neither does a
running old session whose sequence counter is non-zero: v9 and v10 ran the restore synchronously before
the Agent started, only Agent Thread events advance that counter, and the Controller's acknowledgements
delete the outbox rows without resetting it. Sessions that were accepted but never started keep a
pending phase, because no actor existed that could have run a restore. The rows that remain
`legacy_unknown` are exactly the restores that may have been running when the Node was upgraded. They
have no Node-side exit: the residual-reclamation contract (specs#126) releases them once real evidence
or an operator's explicit confirmation covers the original stop.

**Partial restore isolation.** The ledger records restore completion before plugin admission. A prior-
input session whose restore never completed and whose Agent was never admitted can be delivered only
after stop confirmation. It reads HEAD alone, accepts only the clone base or prior final commit, and
returns `unchanged`; it neither probes mixed worktree files nor creates a snapshot, Revision ref or
bundle. Unknown stop rejects delivery before even that HEAD read. Transport repositories whose original
process ownership is unknown are retained; cleanup failure is logged independently of process evidence.

These semantics implement the Node portions of the
[approved restore deadline ADR](https://github.com/ora-space/specs/blob/main/decisions/node/revision/20261018-complete-restore-deadline-and-cancel-confirmation.md).

## Git Server access material

A deployment that enables the platform Git Server adds a top-level `git_access` section with absolute
`ca_cert`, `client_cert` and `client_key` paths for a client certificate issued for the runtime's
tenant, Workspace and generation, separate from the WSS and model-access identities. The entrypoint
keeps these files in the root-only `/run/ora`. The Node validates the material at startup and refuses to
start when it is unusable, and a Cloud-mode Node (`control.target` set) without the section refuses to
start with `git_access_required`. The certificate carries identity only: the Git Server decides what each
request may read or write from Cloud's current state for the execution in its `Ora-Execution-Id`
header, and execution inputs supply the server address. The same material serves
[clone](repository-clone.md), delivery, restore and [ora-git](ora-git.md).

## Limits

- Agent plugins run outside host Scopes, so "process scope closed" means the session's result is written
  and the session runtime stopped its plugin tree.
- Delivery Git Runs are host-managed but not journaled as business mutations; an interrupted preparation
  simply prepares again.
- Repository configuration can still select clean or smudge filters that `git add` runs, under the
  workload identity like the agent itself.
- Parsing the borrowed checkout objects in the transport repository has no resource limits of its own.
- A `revision-restores/` directory left in the Node home by an older Node is not removed.
