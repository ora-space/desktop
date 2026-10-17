# Revision delivery over Git

English | [中文](revision-delivery.zh.md)

The Linux Node pushes an ended Agent session's final commit to Git Server and uploads only its
sealed session JSONL to object storage. Clone and git-access configuration enable delivery;
Agent sessions additionally enable restore. Cloud mode requires usable git-access material.

## Admission and preparation

The production `controlled_deliver_revision` entry checks a permit bound to the Node, incarnation,
operation and execution. Bare delivery is allowed only over private IPC before runtime control has
ever been enabled. The immutable input names the session, clone execution, snapshot baseline,
local Revision ref, Git repository URL, delivery ref, Workspace ref and history object key.
The Workspace ref must equal the clone's; an identical request replays state and a conflict fails.

The ledger records responsibility before work starts. Preparation requires a settled session and
its checkout, then copies the sealed JSONL under
`<home>/revision-deliveries/<sha256(execution)>/history.jsonl`. Workload Git uses a scratch index:
`read-tree HEAD`, `add -A`, `write-tree`. A clean tree keeps HEAD; remaining changes become a child
commit authored by the session identity and committed by `Ora <revision@ora.invalid>`. Hooks,
fsmonitor and signing are disabled. The real index, worktree and local branches remain untouched.
The local Revision ref retains the final commit.

Before network writes, the Node freezes that commit, history bytes/digest and result declaration.
Restart before freezing prepares again; restart afterwards reuses the same plan. A final commit
equal to the baseline or resumed final commit is `unchanged`; every delivery still creates its own
delivery ref and attempts the Workspace update. No bundle is generated, uploaded or downloaded.

## Push and upload

Git with the Node identity never runs in the checkout. A private transport repository borrows only
its object store, refuses nested alternates/shared directories and disables alternate ref discovery.
The certificate, key, CA and execution header are supplied only to each HTTPS command; checkout
configuration cannot redirect it or install credential helpers.

1. Push the frozen commit to `delivery_ref` under `refs/ora/deliveries/`. Creation and identical
   replay succeed; refusal is `push_rejected`; exhausted network failures are `push_failed`.
2. Try to fast-forward `workspace_ref`, declaring the Node ledger's old value. Replays succeed;
   stale leases, rejection or exhausted network failures are logged and never fail the delivery.
3. Request an in-memory upload grant for the history checksum and PUT the frozen JSONL with every
   grant header unchanged. No redirects are followed. 2xx and 412 proceed to Cloud verification;
   three failed upload attempts report `upload_failed`. A 403 invalidates the grant and asks again.

Transient Git failures use three attempts with 5/30-second delays. Each delivery push attempt may
run 30 minutes, independent of `network_timeout_ms`, with a 1 KiB/s low-speed abort after one
minute; 401/403 and refused ref updates are not retried. Shutdown/disconnect preserve the
frozen plan. The terminal sequence-1 event replays until ACK. Cloud verifies the delivery ref and
history outside its transaction before registering the Revision. Grants never reach the ledger.
Frozen files are removed after a durable terminal result, and leftovers are swept on startup.

## Restore before Agent launch

`prior_revision` supplies the Revision identity, final/base commits, Git repository URL and delivery
ref. The repository must equal the checkout's clone repository. The session's execution authorizes
fetching the ref into a private transport, negotiating only from the snapshot baseline. The fetched
commit must equal the prior final commit. Then workload Git takes the objects from the local
transport and checks out the Workspace's local branch at that final commit, before any plugin starts.
The clone baseline stays unchanged, and configuration and `refs/remotes/ora/**` are preserved.

If the prior base is absent from the current snapshot's history, the first turn gains a fixed hint
naming the final/base commits and recommending `ora-git fetch upstream`. A missing/refused delivery
ref is `prior_revision_base_unavailable`; transient fetch, verification or checkout failures are
`prior_revision_unavailable`. The fetch and the import share one five-minute deadline, retries
included, which runs out as `prior_revision_unavailable`. Either ends the session before launch. A
Node stop or an `EndSession` during the restore stops it before its next Git command and waits for
the running one, so the session ends (with the command's reason) only once the restore no longer
writes the checkout. Restart interrupts the session;
it never resumes an old Agent input. Transports are removed after use and swept at Node startup.

## Deployment and limits

Sandbox Server issues a distinct git-access client certificate for tenant, Workspace and generation.
The entrypoint stores material in root-only `/run/ora` tmpfs; workloads receive only their session
socket. The certificate carries identity, while Git Server derives permission from current Cloud state.

Snapshot filters still run as the workload user. Borrowed object parsing and resource limits remain
future hardening work. Delivery preparation is replayable rather than a journaled business mutation;
its durability boundary is the frozen plan. See [snapshot clone and collaboration](repository-clone.md).
