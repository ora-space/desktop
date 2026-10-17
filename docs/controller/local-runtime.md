# Local Controller runtime

English | [中文](local-runtime.zh.md)

`ora-controller` owns durable clone intent and result takeover. It does not execute Git, replace
Backend writers, or act as Cloud authority, and its executable opens no listener in any form. Linux sessions use the existing
[Node IPC](../node/local-ipc.md) and length-prefixed JSON messages, without application credentials.

## Acceptance and persistence

Coordination logic reaches persistent state only through the `CoordinationStore` interface: one atomic
business operation per method (`take_over_node_event`, `record_queried_result`, `original_dispatch`,
`pending_dispatches`, `result`, plus `serve` for the adapter's own coordination with its authority),
asynchronous, exposing no transaction, connection or table. Two adapters implement it and one is chosen
at deployment time, never as a fallback for the other: `SqliteStore::open(home, controller_id)` for
local deployments, whose operations run on the blocking pool so SQLite's fsync never occupies the
async runtime that hosts Node sessions and the embedding caller; and `CloudStore::open(&config)` for cloud
deployments, whose operations are calls on the
[Controller–Cloud contract](../protocols/controller-cloud-contract.md) committed by Cloud in PostgreSQL.
The Controller has no local clone intake: a Workspace clone takes a Git Server snapshot input that
only Cloud can fix, so a deployment without Cloud cannot produce a valid one. The former
`CloneIntake` interface (`accept_request`, `operations`, `operation`) and `ControllerHandle` were
removed
([ADR](../../specs/decisions/cloud/controller-integration/20261016-snapshot-clone-input-and-execution-ref-grants.md)
D4); the SQLite adapter keeps reconciling, taking over and reporting the clone dispatches its database
already holds.
`result(execution_id)` reports the terminal fact as an `ExecutionOutcome` (Node incarnation plus
`ready{path, commit}` or `failed{reason, retained_path}`), the shape both authorities persist; the local
database keeps the full wire result.

`result(execution_id)` reads a durable terminal result; absence is not proof of failure. `pending_dispatches(node)` lists only the executions
without a durable result: they are what a reconnected session keeps querying, while replayed events of
completed executions are still verified against `original_dispatch`.

The explicitly injected private directory contains `ora-controller.sqlite3`, independent of Node and
process state. Application ID `0x4f524143`, schema version 1, exact schema/integrity checks and an OS lease on
the sibling `ora-controller.sqlite3.lock` protect reopening; the lease lives beside the database so
SQLite's own file locks never collide with it on macOS or Windows. A different ControllerId or unknown existing file is rejected. No HOME-derived
storage location, database reset, task import or automatic Controller rebinding is provided.

`clone_operations` stores the dispatched input and the immutable terminal result. `clone_receipts` stores exact
Node event identities/content. Query completion and event delivery use the same takeover transaction;
only a received event produces an Ack after its receipt commits. Duplicate content is idempotent;
conflicting input/result/request association is rejected without acknowledgement. Historical Node
incarnations are retained, while query reporters and heartbeats must match the current session.

## Independent executable

`ControllerRuntime::open(RuntimeConfig)` supports embedding; `run(shutdown)` owns reconnect loops
without installing signal handlers. `Service::start(DeploymentConfig, NodeHosting)` composes the sole runtime owner and an optionally hosted
Node for the executable and for tests, and opens no listener. The executable has no intake of its own:
in the SQLite form it reconciles and takes over the dispatches its database already holds and logs
`ora-controller coordinating locally` once composed.

Build `cargo build -p ora-controller -p ora-node -p ora-process-host -p ora-process-guardian`.
Deployment state lives in one configuration file; per-process composition is given on the command line:

```text
ora-controller --config /absolute/path/controller.json [--single-node]
```

`--single-node` starts the configured Node from the `single_node` section and stops it on normal
shutdown; see below. Invalid flags and configuration are rejected before the database lease is taken.

`persistence` selects the persistence adapter at deployment time; a running Controller never switches,
and neither adapter is a fallback for the other. `{ "kind": "sqlite" }` uses the SQLite database and file
lease inside `home_directory`. The cloud form opens no database, takes no file lease and only dials
out to Cloud:

```json
"persistence": {
  "kind": "cloud",
  "endpoint": "http://127.0.0.1:8082",
  "claim_interval_ms": 1000
}
```

`endpoint` is Cloud's gRPC address; the channel connects lazily, so an unreachable Cloud is an
unavailable authority at call time rather than a start-up failure. Controllers are not
authenticated at this stage: every call carries `controller_id` as `x-ora-controller-id` metadata,
which Cloud records as the lease and submission holder (it must be printable ASCII). Without a
`substrate` section `nodes` must contain exactly one Node; with one it may be empty (see
[Workspace sandboxes](#workspace-sandboxes)). The adapter's
`serve` task acquires Cloud's global lease, renews it every ten seconds, claims accepted work,
validates it as a Node command and registers the dispatch with `RecordDispatch` before the existing
Node session delivers it. Every work item names the Workspace sandbox it runs in (Agent sessions and
Revision deliveries); Cloud no longer hands out tenant-level clones, and an item without a target is
logged once and left queued in Cloud without being registered. One claim registers work until Cloud has
none left or a step fails, in batches of at most 16 so a long queue never delays the lease renewal. Every write carries
the lease epoch and a stable submission identity; only a lost reply is retransmitted, with the same
identity, so Cloud replays its recorded response instead of applying the effect twice. Cloud's
verdicts are mapped once, inside the adapter, to the interface's error classes: conflict (never
retried as-is), unavailable (nothing committed, retry later), unknown (reply lost, same identity
only), stale eligibility (the lease is forgotten and re-acquired by the next renewal). While the
lease is not held nothing is claimed, dispatched or acknowledged, and the Controller never falls back
to writing locally.

Without a `substrate` section claims still start only after a session with the static Node completed
its handshake in this process (a static Node no longer receives any work: this gating outlived the
tenant clones it was made for), which proves that the configured `node_id` is the Node behind the endpoint. A dispatch
names its Node for good (the Node may already have run it, so it is never moved to another
identity); registering work under a misconfigured `node_id` would leave it pending forever. Until
the handshake succeeds the work stays queued with Cloud and the Controller logs once that it is
waiting; the first successful handshake triggers a claim at once. The proof is not withdrawn when
the Node disconnects later: work registered then simply waits for it to return
([ADR](../../specs/decisions/controller/node-management/20260926-static-node-identity-proven-before-dispatch.md)).

While it holds the lease, the Controller keeps one `Watch` stream open, and the stream's state decides
when it claims:

| Stream  | Entered when                                                            | Claims                                                                        |
| ------- | ----------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| none    | start; the stream ends with an error status; the lease epoch is dropped | every `claim_interval_ms`, after trying to reopen the stream on every tick    |
| live    | the stream opens (Cloud has sent its response headers)                  | once on opening, on every `WorkAvailable`, and once after every lease renewal |
| drained | Cloud sends `Drain` or ends the stream cleanly                          | never; the lease is kept and Node sessions continue                           |

`claim_interval_ms` is therefore the claim cadence without a stream only; with a live stream an idle
Controller queries Cloud once per renewal instead of every interval. Signals only accelerate
claiming: ownership is still decided when `RecordDispatch` commits, and the renewal backstop picks up
work whose signal Cloud dropped from a full subscriber buffer. A drain means the Cloud instance is
stopping; the Controller keeps retrying the stream on both ticks and leaves the drain when a new
stream opens, or when a serving Cloud refuses the stream with anything but `UNAVAILABLE`, in which
case it falls back to periodic claiming rather than pausing for good. The Cloud channel sends HTTP/2
keepalive PINGs every 30 seconds, also when idle, and closes a connection whose PING goes unanswered
for 10 seconds, so a silently dropped connection breaks the stream instead of leaving it looking
live. Cloud accepts that cadence only from `ora-space/cloud` `34d8067` (cloud#29) on; an older Cloud
answers it with `GOAWAY` after about two minutes of idleness.

### Workspace sandboxes

With a `substrate` section the cloud form also drives Cloud's Workspace operations (ADR
[Controller drives Workspace sandboxes](../../specs/decisions/controller/node-management/0-controller-drives-workspace-sandboxes.md)):

```json
"persistence": {
  "kind": "cloud",
  "endpoint": "http://127.0.0.1:8082",
  "claim_interval_ms": 1000,
  "substrate": {
    "effects_url": "http://127.0.0.1:8090",
    "router_url": "ws://127.0.0.1:8091/node",
    "atespace": "ora",
    "request_timeout_ms": 30000
  }
}
```

`effects_url` is the Substrate sandbox server's effect API (`GET`/`PUT /effects/{id}`). The
Controller reads an effect before writing it and, after a timeout or an unexpected server error,
reads the same id again, so an effect whose reply was lost is never sent twice under a new identity.
There is no provider abstraction beyond that HTTP boundary.

On every claim tick (and on `OperationAvailable`) the Controller claims one Workspace operation at a
time with `ClaimOperation`, records the step's effect plan, executes it and advances or defers the
operation with the claimed version. Cloud hands back the oldest runnable operation, so operations run
one after another. The steps are:

| Step      | What the Controller does                                                                                                            |
| --------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| sandbox   | executes the ensure effect and advances with its evidence (`sandboxInstanceId`, `nodeId`)                                           |
| node      | opens a Node session through `router_url` with `ate-target-actor: <atespace>/<externalId>` and waits for Cloud to list it connected |
| clone     | forwards the snapshot input Cloud froze (`clone_input`) to `RecordDispatch` and the Node verbatim, and advances once it is ready    |
| quiesce   | closes the sandbox's dispatch gate and reports idle only when no dispatch is pending or unresolved                                  |
| terminate | stops the Node session first, then executes the terminate effect                                                                    |
| cleanup   | executes the data-delete effect                                                                                                     |

Node targets come only from Cloud's sandbox rows; a sandbox whose NodeId collides with a static Node
is refused. The Controller never recreates a sandbox
on its own. After a handshake it registers the Node incarnation with `RegisterNode`, reports it
connected every ten seconds, reports it disconnected once when the session is lost (not when the Controller stopped it), and ends a previous
incarnation before registering a new one. A failed effect defers the operation with
`external_failure`, a timed-out one with `substrate_timeout`, and an unconfirmed termination blocks
it with `termination_unconfirmed`. The Controller never builds a clone input from the Project or
Workspace fields: a clone step whose snapshot carries no `clone_input`, or one the Node protocol
refuses (a URL that is not plain `https`, a ref outside its namespace, a snapshot ref not named after
`base_commit`), blocks with `external_failure` without a dispatch. A ready clone is taken over only
when its commit equals the input's `base_commit`; otherwise it is neither taken over nor acknowledged.

Sessions follow Cloud's record. When a lease epoch is first held, the Controller calls
`ListLiveSandboxes`, starts a session for every listed sandbox and stops any it holds that is not
listed, so a restarted Controller reconnects before any operation is claimed; each claimed
operation's snapshot then keeps its own Project's sandboxes in line. A failed list does not hold
operations back and is retried on the next wake.

```json
{
  "controller": {
    "home_directory": "/home/node/controller",
    "persistence": { "kind": "sqlite" },
    "controller_id": "deployment-controller",
    "protected_state_directories": ["/home/node/state", "/home/node/process"],
    "nodes": [
      {
        "node_id": "deployment-node",
        "endpoint": { "kind": "ipc", "path": "/home/node/state/control.sock" }
      }
    ],
    "session": { "io_timeout_ms": 10000, "query_interval_ms": 1000 },
    "reconnect_ms": 1000,
    "timezone": "Asia/Shanghai"
  },
  "single_node": {
    "node_executable": "/opt/ora/bin/ora-node",
    "node_config": "/home/node/config/node.json",
    "ready_timeout_ms": 30000,
    "stop_timeout_ms": 30000
  }
}
```

A Node in a sandbox is reached through its platform WebSocket router instead:

```json
{
  "node_id": "sandbox-node",
  "endpoint": {
    "kind": "websocket",
    "url": "wss://router.example/ora-node/v1",
    "headers": { "ate-target-actor": "atespace/sandbox-id" }
  }
}
```

Headers are sent verbatim; vendor addressing and platform credentials live only there, and the
handshake still verifies `node_id`. A `ws://`/`wss://` URL and valid header names and values are checked
before any state opens. Each failed or lost session is logged with its class (unreachable, unknown
sandbox, refused, busy, protocol, identity mismatch, silent Node, disconnected) and retried after
`reconnect_ms` without failing or re-creating any execution. "Busy" is only recognizable over WebSocket, where the Node closes with code
`4409`; an IPC Node can only close the socket, so an occupied IPC Node is logged as a protocol failure.
Over WebSocket a Node closing with `1002` or `4403` is logged as a protocol failure or identity
mismatch, and the Controller itself closes with the matching code (`1001` on stop, `1002`, `4403`,
`4408`, or `1011` for a persistence failure) so the Node and any router see why the session ended. Declare all Node/host/guardian state roots in `protected_state_directories`; configured endpoint
IPC socket parents are also protected. Overlap with Controller state is rejected before opening its database. The
executable recovers already accepted records; its configuration file and stdin are not business command
channels. Deploy host and Node separately unless hosting the Node, and configure Node's owner to match
this ControllerId.

With `--single-node`, `nodes` must contain exactly one Node. Before opening state, the
executable requires an `ipc` endpoint, reads `node_config` read-only and refuses to start when its
`control.controller_id` or `control.listen` (kind `ipc` and the same path) does not match, or when something already accepts connections on the endpoint. It then
starts `node_executable <node_config>` in its own process group (no new session), waits up to
`ready_timeout_ms` for the endpoint, and only then reports the composition up. Process host and guardian are
prerequisites: the executable neither deploys nor starts them. Controller death alone signals nothing to
the Node, so an accepted clone keeps running; a group-level stop from an operator or launcher reaches
both. If the hosted Node exits on its own, the Controller shuts down and exits with failure rather than
holding work it cannot dispatch.

Normal shutdown stops in order: Node sessions, the
adapter's own coordination (in the cloud form: a bounded attempt to release the lease, after the
sessions so nothing writes under a lease about to be released), the hosted Node (`SIGTERM`, waiting up
to `stop_timeout_ms`; never escalated to `SIGKILL`), then the database lease in the SQLite form.
Accepted Node executions are never cancelled by this process stopping.

The Cloud-facing contract is defined by the Cloud repository's
proto and the Controller dials out as its client (see the
[Controller–Cloud contract](../protocols/controller-cloud-contract.md)); it exposes no service to Cloud.

## Verification and remaining scope

Real SQLite tests, driven through the `CoordinationStore` interface on dispatches written into the
database directly, cover durability, exclusive ownership, transaction failure, query/event ordering,
duplicate takeover, conflicting facts and the retirement of completed executions from periodic queries.
The Cloud adapter's verdict mapping, same-identity retransmission and message translation are unit
tested, and so are the transitions of the stream state. `apps/ora-controller/tests/cloud.rs` drives
the runtime against an in-memory Cloud that serves the real contract through the test-only server
stubs of `ora-controller-proto` (`test-server` feature) and a fake static Node over IPC, covering
signal-driven claiming, claims held back until the Node's handshake and while the Node presents
another identity, fallback and reopening after a broken stream, drains, a refused stream after a
drain, a stale epoch on opening, refusing work without a target, and cancelling the stream and
releasing the lease on shutdown. The runtime and executable tests cover that the cloud form opens no local state and stays up while
Cloud is unreachable. Its behavior against a real Cloud (lease, claim, dispatch, takeover, restart
without a second clone) was verified end to end by hand; integration now runs in the cluster
repository's Compose stack (Gateway → Cloud → Controller → Sandbox Server → Node), and none of it is an
automated test yet. `apps/ora-controller/tests/workspaces.rs` drives
Workspace operations against a fake Cloud, a fake Substrate effect server and a fake Node: creating
and stopping a Workspace runs every step, registers the Node before the clone, reports idle and stops
the session before terminating without reporting that stop as a lost connection; the frozen clone
input reaches `RecordDispatch` and the Node verbatim; a clone step without that input, or with one
the Node protocol refuses, blocks without a dispatch; a ready clone at another commit is not taken
over; and quiesce reports busy while a dispatch has no result. The Substrate client is unit tested for querying a timed-out effect
by the same id and for not resending a completed effect. Framed-session tests cover bounded Unknown retransmission
and rejection of a wrong Node identity or missing clone capability before dispatch.
The independent Controller–Node–host/guardian test performs real HTTPS clone, intercepts Ack, kills
Controller after durable takeover, restarts it offline, then checks original result, exact Ack, cleared
Node outbox and one mutation Run. Node's own IPC tests additionally cover Node restart and event replay.

A separate child process runs production `run_session` and the real SQLite owner with an injected
pre-commit barrier. The parent observes that no Ack escaped, sends SIGKILL while the takeover transaction
is open, and reopens the store to verify rollback and unchanged intent. The normal Controller executable
then takes over the replayed Node result with HTTPS refusing access. The barrier is a persistence test
dependency (`WritePoint::Commit`), not a deployment option or protocol extension.

These are not complete Client/UI, Cloud, multi-Controller or hostile-peer guarantees. Exhaustive queue
pressure, all crash boundaries and all deployment combinations remain tracked in the approved ADR's
core test cases. The existing Backend entry and Worktree coordination are unchanged.
