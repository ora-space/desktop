# Cloud Agent session relay

English | [中文](agent-relay.zh.md)

In Cloud mode, `ora-controller` carries Agent session work and commands between Cloud and the
Workspace's Node. Cloud owns the IssueRun phase and durable receipts; the Controller keeps only
connection-local queues. Local SQLite mode remains clone-only.

## Dispatch and authority

`ClaimWork` session items name their Workspace sandbox and Node. The Controller waits for that
sandbox's verified handshake and `agent_session` capability, then records the exact input in Cloud
while holding the sandbox admission gate. A reconnect wakes queued work. The Node operation ID is
read back from the registered record; it is not assumed to equal the IssueRun ID.

Pending executions are queried. An `Unknown` status permits at most one original-command
retransmission per connection, using a fresh execution permit and `ControlledStartAgentSession`.
A missing/closed permission cannot authorize a start. Nonterminal Agent executions count as
unfinished work when quiesce determines whether the sandbox is idle.

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
tests with in-memory Cloud and Node fixtures in `tests/workspaces/agents.rs`. They exercise
ordered takeover, independent slow executions, outage/conflict/lost-response recovery, Controller
restart, command retry/order/rejection, capability gating, large records and quiesce responsibility.
These tests do not establish real PostgreSQL, production Node or browser Thread integration.

The Controller session ADR is still `proposed`; this implementation supplies reviewable behavior
without claiming design approval or updating approved core-test evidence. Revision delivery and
upload-grant relay remain a separate development slice. Node process-scope containment and the
full M2 Compose/browser acceptance still require their own evidence.
