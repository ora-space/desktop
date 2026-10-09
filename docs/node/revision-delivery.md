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
   produce `revision.bundle`, copied next to the history.
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

## Known limits

- Agent plugins run outside host Scopes, so "process scope closed" means the session's terminal
  result is written and its plugin process tree was stopped by the session runtime.
- Delivery Git Runs are host-managed but not journaled as business mutations; an interrupted
  preparation is simply prepared again.
- Repository configuration can still select clean/smudge filters that `git add` runs; they run under
  the workload identity, like the Agent itself.
