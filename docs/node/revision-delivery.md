# Revision delivery

English | [中文](revision-delivery.zh.md)

The Linux Node saves an ended Agent session's checkout and history as a Revision
(`revision_delivery` capability, ADR `node/revision/0`). It advertises the capability whenever it
has the `clone` deployment section, because delivery runs Git in checkouts created by clone, with
clone's hardened Git environment.

## Admission

- `controlled_deliver_revision` is the production entry: the runtime permit must match this Node,
  incarnation, operation and execution, and is checked at admission. Bare `deliver_revision` is
  accepted only on private IPC in a home that never enabled runtime control.
- The input is persisted before any work and the delivery is recorded as already started
  (`Running`). Repeating the same input only reports state; different input under the same
  identity is a protocol error.
- Ledger: table `revision_deliveries` (schema v9, identity kind `deliver_revision`) with its
  single terminal event in `revision_outbox`. Deliveries share no table with sessions, so session
  recovery never sees them. Status queries answer `Running` until the terminal result, then the
  stored `revision` result.

## Preparation (once)

1. The session must have its terminal result and no live actor; otherwise
   `failed{session_not_settled}`. The checkout is the session's own clone and must still be at the
   delivery's base commit; otherwise `failed{checkout_unavailable}`. A missing session JSONL is
   `failed{history_unavailable}`.
2. The sealed JSONL is copied into `<home>/revision-deliveries/<sha256(execution)>/history.jsonl`.
3. The snapshot uses a scratch index in the checkout's Git directory (writable by the workload
   identity, unlike the Node home): `read-tree HEAD`, `add -A`, `write-tree`. If the tree equals
   `HEAD^{tree}` the final commit is `HEAD`; otherwise `commit-tree` adds one commit authored by the
   session's git identity and committed by `Ora <revision@ora.invalid>`. Hooks, fsmonitor and
   signing are disabled; the real index, worktree files and branches are not changed.
   `update-ref` points the Revision ref at the final commit.
4. If the final commit differs from the base, `bundle create <ref> ^<base>` and `bundle verify`
   produce `revision.bundle`, copied next to the history. When the input carries a
   `prior_revision` (a resumed session) and the final commit equals its `final_commit`, the
   delivery is `unchanged` at that commit and no bundle is made: the prior Revision's bundle
   already holds it. Any other new commit is bundled against the clone's base as usual, so it
   carries the prior Revision's commits too.
5. Size and SHA-256 of every object and the declaration (`delivered` or `unchanged`) are written to
   the ledger as the frozen plan **before the first PUT**. A restart before that prepares again;
   after it, the same files and declaration are reused and nothing is snapshotted again.

## Upload

- Grants live only in memory and are never logged or written to the ledger. Without a valid grant
  (none yet, expired within 5 s, refused with 403, or lost by a restart) the Node sends
  `upload_grant_needed` with the frozen SHA-256 of every object still to upload, repeats it every
  2 s while waiting, and sends it again at once on every new Controller connection. The delivery
  stays `Running` while waiting, without a deadline.
- Each PUT streams the frozen file with every grant header unchanged (including `If-None-Match: *`
  and `x-amz-checksum-sha256`) and never follows redirects. 2xx and 412 count as uploaded; Cloud
  verifies the declaration. Every other outcome is an attempt; the third failed attempt of one
  object ends the delivery with `failed{upload_failed}`.
- The terminal `revision_result` (sequence 1) is replayed until acknowledged. The frozen files are
  removed once it is durable; leftovers are swept at the next start.
- Shutdown, Controller disconnect and lifecycle cancellation never write a failure: the delivery
  resumes after restart.

## Resuming a prior Revision

A new run of the same Issue resumes the Issue's latest Revision (ADR
`node/revision/20261010-restore-prior-revision-before-session`). The Node advertises
`revision_restore` exactly when it advertises both `agent_session` and `revision_delivery`, since
restore runs delivery Git in session checkouts; the Controller sends a session with
`prior_revision` only to such a Node.

The session driver restores after resolving the checkout and checking the plugin, and before the
checkout is handed back to the workload user or any plugin starts. A failed restore ends the session
as `agent_failed` with `prior_revision_unavailable` or `prior_revision_base_unavailable`; no plugin
process ever existed. A Node restart during a restore ends the session `interrupted`, like any
session, and the delivery decision below stays exact for either state of the checkout.

### Download

- The Node sends `download_grant_needed` (no sequence), repeats it every 2 s while waiting, and
  again at once on every new Controller connection. The answer is `download_grant{granted}` or
  `download_grant{refused}`; a refusal fails the restore at once. Grants are memory-only and never
  logged; only a grant for the session's own `prior_revision.bundle.key` is accepted.
- Grant wait and download share one 5-minute deadline. The bundle is fetched with one presigned
  `GET` (headers verbatim, redirects never followed, no URL in errors) into
  `<home>/revision-restores/<sha256(execution)>/prior.bundle` (directories `0700`, file `0600`),
  bounded by the declared size, and must match the declared size and SHA-256. Network errors, 429
  and 5xx are retried 3 times; a 403 drops the grant and asks for a fresh one once; any other
  answer fails. The directory is removed when the restore ends and the root is cleared at start.

### Git steps

All commands run like delivery Git (host-managed, as the workload user, hooks/fsmonitor/signing
disabled, clone's environment and protocol policy), on a blocking thread:

1. The bundle header must be v2 or v3 (only `@object-format` capabilities) with exactly one head,
   under `refs/ora/revisions/`, pointing at `prior_revision.final_commit`; otherwise unavailable.
2. Every prerequisite (`-<oid>`) must exist (`cat-file -e <oid>^{commit}`). Missing ones are fetched
   once with `fetch --no-tags origin <oids>`. Only when the remote answers that it has no such object
   (`not our ref`, an unadvertised object, no such remote ref), or the fetch succeeds and a commit is
   still missing, is the result `base_unavailable`; an unreachable remote or refused access is
   `unavailable`, which Cloud retries on the next run instead of refusing the Revision.
3. The verified bundle is copied into the checkout's Git directory (owned by the workload user,
   removed afterwards), then `bundle verify`, then `fetch --no-tags <bundle> <head>:<head>` (the
   file transport is allowed for this one command), and the final commit must resolve.
4. `checkout --force -B <branch> <final_commit>` on the clone's branch; `origin/<branch>` is not
   touched, so the worktree and index equal the prior final commit.
5. If a prerequisite is not an ancestor of `refs/remotes/origin/<branch>`, the remote history was
   rewritten: one fixed English text block naming the restored commit, its base commit and
   `origin/<branch>` is appended to the first turn, so it also reaches the session history and
   Thread.

## Known limits

- Agent plugins run outside host Scopes, so "process scope closed" means the session's terminal
  result is written and its plugin process tree was stopped by the session runtime.
- Delivery Git Runs are host-managed but not journaled as business mutations; an interrupted
  preparation is simply prepared again.
- Repository configuration can still select clean/smudge filters that `git add` runs; they run under
  the workload identity, like the Agent itself.
- The permanent `base_unavailable` relies on the remote's wording for a missing object; a server
  that phrases it differently is reported as the retryable `unavailable`.
