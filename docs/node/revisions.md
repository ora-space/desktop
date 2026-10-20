# Revisions

English | [中文](revisions.zh.md)

When an Agent session ends, the Node saves its checkout and history as a **Revision**: a commit (as an
incremental bundle) plus the sealed session JSONL, uploaded to object storage through grants Cloud
signs. A later run of the same Issue can restore that Revision before its agent starts. The decisions
are the [internal snapshot and incremental bundle upload](https://github.com/ora-space/specs/blob/main/decisions/node/revision/0-internal-snapshot-and-incremental-bundle-upload.md)
and [restore prior Revision before session](https://github.com/ora-space/specs/blob/main/decisions/node/revision/20261010-restore-prior-revision-before-session.md)
ADRs. The Controller's side is in [Agent relay](../controller/agent-relay.md), the grant contract in
[Cloud contract](../controller/cloud-contract.md#revision-upload-checksums).

The Node advertises `revision_delivery` whenever it has the `clone` section, because delivery runs Git
in clone checkouts with clone's hardened Git environment, and `revision_restore` exactly when it
advertises both `agent_session` and `revision_delivery`.

## Admission

- `controlled_deliver_revision` is the production entry: its runtime permit must match this Node,
  incarnation, operation and execution, checked at admission. A bare `deliver_revision` is accepted
  only on private IPC in a home that never enabled runtime control.
- The input is persisted before any work and the delivery recorded as `Running`. Repeating the same
  input only reports state; different input under the same identity is a protocol error.
- Deliveries live in `revision_deliveries` (schema v9) with their single terminal event in
  `revision_outbox`, separate from sessions so session recovery never sees them.

## Preparation

Preparation happens once:

1. The session must have its terminal result and no live actor (`failed{session_not_settled}`), its
   checkout must still be at the delivery's base commit (`failed{checkout_unavailable}`), and its JSONL
   must exist (`failed{history_unavailable}`).
2. The sealed JSONL is copied into `<home>/revision-deliveries/<sha256(execution)>/history.jsonl`.
3. A scratch index in the checkout's Git directory snapshots the worktree (`read-tree HEAD`, `add -A`,
   `write-tree`). If the tree equals `HEAD^{tree}` the final commit is `HEAD`; otherwise `commit-tree`
   adds one commit authored by the session's Git identity and committed by `Ora <revision@ora.invalid>`.
   Hooks, fsmonitor and signing are disabled, and the real index, worktree files and branches are not
   touched. `update-ref` points the Revision ref at the final commit.
4. If the final commit differs from the base, `bundle create <ref> ^<base>` and `bundle verify` produce
   `revision.bundle`. For a resumed session whose final commit equals the prior Revision's, the delivery
   is `unchanged` at that commit and no bundle is made; any other new commit is bundled against the
   clone's base, so it carries the prior Revision's commits too.
5. Size and SHA-256 of every object and the declaration (`delivered` or `unchanged`) are written to the
   ledger as the frozen plan before the first upload. A restart before that prepares again; after it,
   the same files and declaration are reused.

## Upload

- Without a valid grant (none yet, expiring within 5 s, refused with 403, or lost by a restart) the Node
  sends `upload_grant_needed` with the frozen SHA-256 of every object still to upload, repeats it every
  2 s and again at once on every new connection. The delivery stays `Running` without a deadline.
  Grants live only in memory and are never logged.
- Each PUT streams the frozen file with every grant header unchanged (including `If-None-Match: *` and
  `x-amz-checksum-sha256`) and never follows redirects. 2xx and 412 count as uploaded; Cloud verifies
  the declaration. Three failed attempts on one object end the delivery as `failed{upload_failed}`.
- The terminal `revision_result` (sequence 1) is replayed until acknowledged; the frozen files are
  removed once it is durable and leftovers are swept at the next start.
- Shutdown, Controller disconnection and cancellation never write a failure; the delivery resumes after
  restart.

## Restoring a prior Revision

A session with `prior_revision` restores it after resolving the checkout and checking the plugin, and
before the checkout is handed to the workload user or any plugin starts. A failure ends the session as
`agent_failed` with `prior_revision_unavailable` or `prior_revision_base_unavailable`, with no plugin
process ever started. A restart during a restore ends the session `interrupted`.

**Download.** The Node sends `download_grant_needed`, repeating every 2 s and on every new connection,
and accepts only `download_grant { granted }` for its own `prior_revision.bundle.key`; `refused` fails the
restore. Download and every Git step below, including the ancestor check, share one absolute deadline.
It defaults to five minutes and is configured in the Node service file as
`"restore": { "deadline_ms": 300000 }`. The value must be a positive integer representing a finite,
representable duration; invalid values refuse service startup. Grant renewal, retries, backoff,
download completion and waiting for the restore Git lock never reset this cutoff.

The bundle is fetched with one presigned
`GET` (headers verbatim, no redirects, no URL in errors) into
`<home>/revision-restores/<sha256(execution)>/prior.bundle` (directories `0700`, file `0600`), bounded by
and checked against the declared size and SHA-256. Network errors, 429 and 5xx retry three times; a 403
drops the grant and asks for a new one once; anything else fails. The directory is removed afterwards.

**Git steps**, run like delivery Git (host-managed, as the workload user, hooks, fsmonitor and signing
disabled):

1. The bundle header must be v2 or v3 (only `@object-format` capabilities) with exactly one head under
   `refs/ora/revisions/` pointing at `prior_revision.final_commit`.
2. Every prerequisite must exist; missing ones are fetched once with `fetch --no-tags origin <oids>`. Only
   when the remote says it has no such object, or a fetched commit is still missing, is the result
   `base_unavailable`; an unreachable remote or refused access is `unavailable`, which Cloud retries on
   the next run.
3. The verified bundle is copied into the checkout's Git directory, verified and fetched
   (`fetch --no-tags <bundle> <head>:<head>`, with the file transport allowed for that one command).
4. `checkout --force -B <branch> <final_commit>` on the clone's branch; `origin/<branch>` is untouched.
5. If a prerequisite is not an ancestor of `refs/remotes/origin/<branch>`, the remote history was
   rewritten, and one fixed English note naming the restored commit, its base and `origin/<branch>` is
   appended to the first turn.

Each supervised Git command receives the earlier of its own command deadline and the restore cutoff.
No new command starts after expiry; a pending host exchange is bounded by that same cutoff, so it
cannot defer the close request until a longer transport timeout. Cleanup and original Scope closure
use their independent cleanup budget after execution expires. A late ancestor check cannot turn an
expired restore into success. Expiry is not proof that a process stopped: durable unconfirmed-stop
ownership, delivery isolation and the residual-evidence bound remain separate obligations of the
[approved restore deadline ADR](https://github.com/ora-space/specs/blob/main/decisions/node/revision/20261018-complete-restore-deadline-and-cancel-confirmation.md).

## Git Server access material

A deployment that enables the platform Git Server adds a top-level `git_access` section with absolute
`ca_cert`, `client_cert` and `client_key` paths for a client certificate issued for the runtime's
tenant, Workspace and generation, separate from the WSS and model-access identities. The entrypoint
keeps these files in the root-only `/run/ora`. The Node validates the material at startup and refuses to
start when it is unusable; the certificate grants no repository permission by itself and execution
inputs supply the server address. Delivery and restore still use object storage: the Git Server
delivery path is a `proposed` decision and is not implemented here.

## Limits

- Agent plugins run outside host Scopes, so "process scope closed" means the session's result is written
  and the session runtime stopped its plugin tree.
- Delivery Git Runs are host-managed but not journaled as business mutations; an interrupted preparation
  simply prepares again.
- Repository configuration can still select clean or smudge filters that `git add` runs, under the
  workload identity like the agent itself.
- `base_unavailable` relies on the remote's wording for a missing object; a server that words it
  differently is reported as the retryable `unavailable`.
