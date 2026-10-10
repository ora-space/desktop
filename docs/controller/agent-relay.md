# Cloud Agent session relay

English | [中文](agent-relay.zh.md)

In Cloud mode, `ora-controller` carries Agent session work, commands, Revision deliveries, their
upload grants and the download grants of resumed sessions between Cloud and the Workspace's Node. Cloud owns the IssueRun phase and durable
receipts; the Controller keeps only connection-local queues. Local SQLite mode remains clone-only.

## Dispatch and authority

`ClaimWork` session items name their Workspace sandbox and Node. The Controller waits for that
sandbox's verified handshake and `agent_session` capability, then records the exact input in Cloud
while holding the sandbox admission gate. A reconnect wakes queued work. The Node operation ID is
read back from the registered record; it is not assumed to equal the IssueRun ID.

Pending executions are queried. An `Unknown` status permits at most one original-command
retransmission per connection, using a fresh execution permit and `ControlledStartAgentSession`.
A missing/closed permission cannot authorize a start. Nonterminal Agent executions count as
unfinished work when quiesce determines whether the sandbox is idle.

## Revision delivery

`DeliverRevision` work items take the same claim, register and dispatch path. Registration
requires the target Node's verified handshake with the `revision_delivery` capability; a Node
without it leaves the item queued in Cloud. The registered input is rebuilt with the target Node
added (Cloud's spec carries none) and sent only as `ControlledDeliverRevision` under a fresh
execution permit. Each family has its own pending list, so a delivery record never reaches clone
polling or clone reconciliation, and an unfinished delivery keeps quiesce from reporting idle.

The terminal `RevisionResult` is taken over with `TakeOverNodeEvent` and acknowledged only after
Cloud committed it; Cloud verifies the declared objects inside that takeover. A Completed status
reply settles nothing, because Cloud accepts a delivery result only together with the Node's
sequenced receipt; the Node keeps replaying the envelope until it is acknowledged. The Controller
never writes `RevisionFailed` itself and never reports `VERIFICATION_FAILED`, which only Cloud
records. Shutdown, lease loss and connection loss leave the registered delivery without a result;
a new process resumes it through status queries and, for an `Unknown` answer, the original
command.

## Upload grants

Grants are requested from `GrantRevisionUpload` only with the SHA-256 digests the Node froze, so
Cloud signs `x-amz-checksum-sha256` together with `If-None-Match: *`:

- When the Node sends `UploadGrantNeeded`, its checksum map is forwarded unchanged.
- When a new connection first sees the delivery `Accepted` or `Running`, it requests again with the
  digests that Node last reported to this process, which covers reconnects.
- Nothing is requested right after dispatch. The Node has not frozen its objects yet, so such a
  grant could not be checksum-bound, and the Node asks as soon as it needs one. For the same
  reason a restarted Controller, which knows no digests, waits for the Node's request.

Returned grants are relayed as `UploadGrant` with headers and expiry verbatim, only for requested
keys. An unavailable Cloud or lost lease keeps the request and retries it later; a `CONFLICT`
(the delivery has a result or its run is no longer delivering) drops it without failing the
delivery or the connection. Grants live only in the relay's memory on their way to the transport;
they are not persisted or logged, and the frame is never formatted into diagnostics. The process
memory keeps only the digests, which are not credentials.

## Resumed sessions and download grants

A session input with `prior_revision` resumes the Issue's latest Revision (restore contract
`cloud/controller-integration/20261010-revision-restore-contract`). Its prior Revision maps into the
Node session spec with its bundle object (an input without one is refused), and into the delivery
spec without it (an input with one is refused). Such a session is registered only when the target
Node's live handshake also advertised `revision_restore`; otherwise it stays queued in Cloud like
any capability miss, so it never runs on a fresh clone of a Node that cannot restore. An unchanged
delivery result is accepted when its final commit is the base or the input's prior final commit.

The Node asks with `DownloadGrantNeeded` while its restore waits, and again on every new
connection; the Controller asks `GrantRevisionDownload` only then and keeps nothing across
connections. Repeated requests for one execution coalesce. The returned grant is relayed as
`DownloadGrant{granted}` with headers and expiry verbatim, only as a `GET` of the session's own
bundle key. `NOT_FOUND` and `ABORTED` (`CONFLICT`: the session has a result, its run stopped, or
the input names no bundle) reach the Node as `DownloadGrant{refused}`, which fails its restore;
an unavailable Cloud, a lost reply or a stale lease (`FAILED_PRECONDITION`) keeps the request and
retries it, so an outage never fails a restore. Download grants, like upload grants, are never
persisted or logged.

## Events and recovery

Each execution has its own ordered takeover worker. Batches contain at most 64 records, wait at
most 100 ms from the first record, and flush after roughly 1 MiB of encoded payload to stay within
gRPC message limits. Cloud must confirm the entire submitted batch before any exact event ACK is
sent. The terminal envelope follows all preceding batches; a Completed status query does not
settle a session because it carries no sequence proving those preceding records were delivered.

A definitely unavailable write retries after 250 ms without ACK. Lost-response writes use the
existing RPC adapter's bounded retries with one submission ID. Conflict or exhausted uncertain
writes close the connection without ACK; reconnect replays from Node's durable outbox. Merely
clearing an in-memory queue would not trigger Node replay and could leave a full window stuck.

Thread workers, command delivery, ordinary reconciliation and polling run outside the transport
frame pump. A slow Thread request does not stop another execution's relay or transport heartbeats.
Connection teardown cancels its workers; no local durable copy of Cloud state is created.

## Commands

`ThreadCommandAvailable` wakes delivery, with periodic polling as a backstop. Commands must match
the registered run/execution and the current sandbox/runtime scope. Only the first command of a
run is eligible; Node must first report that it knows the execution. After transmission, a matching
`SessionCommandAccepted` or `SessionCommandRejected(session_ended)` permits
`RecordThreadCommandDelivered`. A send alone never does. Retries preserve the command ID and
payload. After restart, Cloud supplies the unconfirmed head again; Node performs deduplication.

## Validation and remaining work

`cargo test -p ora-controller --lib --tests` includes real generated gRPC and WebSocket transport
tests with in-memory Cloud and Node fixtures in `tests/workspaces/agents.rs`,
`tests/workspaces/deliveries.rs` and `tests/workspaces/restores.rs`. They exercise ordered takeover, independent slow executions,
outage/conflict/lost-response recovery, Controller restart, command retry/order/rejection,
capability gating, large records, quiesce responsibility, permit-gated delivery dispatch,
checksum-bound grant relay with verbatim headers, grant refresh on reconnect, refused grants,
ACK after commit, the `revision_restore` gate for resumed sessions, download grant relay,
refusal and outage retry, re-requests after reconnect, and the absence of grant URLs and signatures
from logs. These tests do not
establish real PostgreSQL, object storage, production Node or browser Thread integration.

The Controller session ADR is still `proposed`; this implementation supplies reviewable behavior
without claiming design approval or updating approved core-test evidence. Node process-scope
containment, the production Node delivery and the full M2/M3 Compose/browser acceptance still
require their own evidence.
