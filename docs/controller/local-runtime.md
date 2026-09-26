# Local Controller runtime

English | [中文](local-runtime.zh.md)

`ora-controller` owns durable local clone intent and result takeover, and its executable hosts the
transitional clone API that [minicloud](../minicloud/runtime.md) calls. It does not execute Git, replace
Backend writers, or act as Cloud authority. Linux sessions use the existing
[Node IPC](../node/local-ipc.md) and length-prefixed JSON messages, without application credentials.

## Acceptance and persistence

Coordination logic reaches persistent state only through the `CoordinationStore` interface: one atomic
business operation per method (`take_over_node_event`, `record_queried_result`, `original_dispatch`,
`pending_dispatches`, `result`, plus `serve` for the adapter's own coordination with its authority),
asynchronous, exposing no transaction, connection or table. Two adapters implement it and one is chosen
at deployment time, never as a fallback for the other: `SqliteStore::open(home, controller_id)` for
local deployments, whose operations run on the blocking pool so SQLite's fsync never occupies the
async runtime that hosts Node sessions and the API; and `CloudStore::open(&config)` for cloud
deployments, whose operations are calls on the
[Controller–Cloud contract](../protocols/controller-cloud-contract.md) committed by Cloud in PostgreSQL.
Accepting caller requests and cataloguing accepted operations (`accept_request`, `operations`,
`operation`) is the separate `CloneIntake` interface that only the SQLite adapter implements: in a
cloud deployment acceptance belongs to Cloud's public API, so the JSON surface is not composed at all.
`result(execution_id)` reports the terminal fact as an `ExecutionOutcome` (Node incarnation plus
`ready{path, commit}` or `failed{reason, retained_path}`), the shape both authorities persist; the local
catalogue keeps the full wire result for presentation.

The command returned by `accept_request(request_id, spec)` contains stable operation/execution IDs.
Acceptance writes the complete input and target Node before returning; repeating a request returns the
original command, while changed input is rejected. `result(execution_id)` reads a durable terminal
result; absence is not proof of failure. `pending_dispatches(node)` lists only the executions
without a durable result: they are what a reconnected session keeps querying, while replayed events of
completed executions are still verified against `original_dispatch`.

The explicitly injected private directory contains `ora-controller.sqlite3`, independent of Node and
process state. Application ID `0x4f524143`, schema version 1, exact schema/integrity checks and an OS lease on
the sibling `ora-controller.sqlite3.lock` protect reopening; the lease lives beside the database so
SQLite's own file locks never collide with it on macOS or Windows. A different ControllerId or unknown existing file is rejected. No HOME-derived
storage location, database reset, task import or automatic Controller rebinding is provided.

`clone_operations` stores acceptance and the immutable terminal result. `clone_receipts` stores exact
Node event identities/content. Query completion and event delivery use the same takeover transaction;
only a received event produces an Ack after its receipt commits. Duplicate content is idempotent;
conflicting input/result/request association is rejected without acknowledgement. Historical Node
incarnations are retained, while query reporters and heartbeats must match the current session.

## Independent executable

`ControllerRuntime::open(RuntimeConfig)` supports embedding. `handle()` exposes durable clone
acceptance, operation listing and lookup; `run(shutdown)` owns reconnect loops without installing signal
handlers. Missing lookup is distinct from an accepted operation without a terminal result. The library
depends on no listener; `Service::start(DeploymentConfig, Transport, NodeHosting)` composes the API
listener, the sole runtime owner and an optionally hosted Node for the executable and for tests.

Build `cargo build -p ora-controller -p ora-node -p ora-process-host -p ora-process-guardian`.
Deployment state lives in one configuration file; per-process composition is given on the command line:

```text
ora-controller --config /absolute/path/controller.json [--single-node]
               [--transport tcp|unix] [--host 127.0.0.1] [--port 4820] [--socket /path/api.sock]
```

| Flag                        | Rule                                                                                                                                                                                                    |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--transport tcp` (default) | `--host` defaults to `127.0.0.1`, `--port` to `4820`. A non-loopback host is accepted with a warning: the API has no authentication, so loopback is a deployment restriction, not a security guarantee. |
| `--transport unix`          | Requires `--socket`, an absolute path directly inside `home_directory`, created with the same private-socket rules as the Node endpoint. `--host`/`--port` are rejected.                                |
| `--single-node`             | Starts the configured Node from the `single_node` section and stops it on normal shutdown; see below.                                                                                                   |

Invalid flag combinations and configuration are rejected before the database lease is taken.

`persistence` selects the persistence adapter at deployment time; a running Controller never switches,
and neither adapter is a fallback for the other. `{ "kind": "sqlite" }` uses the SQLite database and file
lease inside `home_directory` and serves the JSON surface, so the `api` section is required. The cloud
form opens no database, takes no file lease and serves no JSON surface, so `api` must be absent and
the listener flags are refused:

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
Node session delivers it through its periodic status query. One claim registers work until Cloud has
none left or a step fails, in batches of at most 16 so a long queue never delays the lease renewal. Every write carries
the lease epoch and a stable submission identity; only a lost reply is retransmitted, with the same
identity, so Cloud replays its recorded response instead of applying the effect twice. Cloud's
verdicts are mapped once, inside the adapter, to the interface's error classes: conflict (never
retried as-is), unavailable (nothing committed, retry later), unknown (reply lost, same identity
only), stale eligibility (the lease is forgotten and re-acquired by the next renewal). While the
lease is not held nothing is claimed, dispatched or acknowledged, and the Controller never falls back
to writing locally.

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
| clone     | dispatches the latest clone through the Node session and advances once its result is ready                                          |
| quiesce   | closes the sandbox's dispatch gate and reports idle only when no dispatch is pending or unresolved                                  |
| terminate | stops the Node session first, then executes the terminate effect                                                                    |
| cleanup   | executes the data-delete effect                                                                                                     |

Node targets come only from Cloud's sandbox rows; static `nodes` stay tenant clone targets, and a
sandbox whose NodeId collides with a static Node is refused. The Controller never recreates a sandbox
on its own. After a handshake it registers the Node incarnation with `RegisterNode`, reports it
connected every ten seconds, reports it disconnected once when the session is lost (not when the Controller stopped it), and ends a previous
incarnation before registering a new one. A failed effect defers the operation with
`external_failure`, a timed-out one with `substrate_timeout`, and an unconfirmed termination blocks
it with `termination_unconfirmed`. A clone whose ref the Node protocol refuses (such as `HEAD`)
blocks the step without a dispatch.

Sessions follow Cloud's record. When a lease epoch is first held, the Controller calls
`ListLiveSandboxes`, starts a session for every listed sandbox and stops any it holds that is not
listed, so a restarted Controller reconnects before any operation is claimed; each claimed
operation's snapshot then keeps its own Project's sandboxes in line. A failed list does not hold
operations back and is retried on the next wake. Cloud requires a concrete Project default branch,
so `HEAD` reaches the clone step only for Projects created before that rule.

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
  "api": { "node_id": "deployment-node" },
  "single_node": {
    "node_executable": "/opt/ora/bin/ora-node",
    "node_config": "/home/node/config/node.json",
    "ready_timeout_ms": 30000,
    "stop_timeout_ms": 30000
  }
}
```

`api.node_id` names the configured Node that accepted clones are dispatched to; callers never choose a
Node. A Node in a sandbox is reached through its platform WebSocket router instead:

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

With `--single-node`, `nodes` must contain exactly the `api.node_id` Node. Before opening state, the
executable requires an `ipc` endpoint, reads `node_config` read-only and refuses to start when its
`control.controller_id` or `control.listen` (kind `ipc` and the same path) does not match, or when something already accepts connections on the endpoint. It then
starts `node_executable <node_config>` in its own process group (no new session), waits up to
`ready_timeout_ms` for the endpoint, and only then binds the API. Process host and guardian are
prerequisites: the executable neither deploys nor starts them. Controller death alone signals nothing to
the Node, so an accepted clone keeps running; a group-level stop from an operator or launcher reaches
both. If the hosted Node exits on its own, the Controller shuts down and exits with failure rather than
accepting undispatchable requests.

Normal shutdown stops in order: API admission (bounded wait for in-flight requests), Node sessions, the
adapter's own coordination (in the cloud form: a bounded attempt to release the lease, after the
sessions so nothing writes under a lease about to be released), the hosted Node (`SIGTERM`, waiting up
to `stop_timeout_ms`; never escalated to `SIGKILL`), then the database lease in the SQLite form.
Accepted Node executions are never cancelled by this process stopping.

The JSON surface is the transitional clone API documented under
[minicloud](../minicloud/runtime.md#http-interface); its DTOs live in `ora-contracts::controller_api`.
It exists only with SQLite persistence. The Cloud-facing contract is defined by the Cloud repository's
proto and the Controller dials out as its client (see the
[Controller–Cloud contract](../protocols/controller-cloud-contract.md)); it exposes no service to Cloud.

## Verification and remaining scope

Real SQLite tests, driven through the `CoordinationStore` and `CloneIntake` interfaces, cover acceptance, exclusive ownership, transaction failure, query/event ordering,
duplicate takeover, conflicting facts and the retirement of completed executions from periodic queries.
The Cloud adapter's verdict mapping, same-identity retransmission and message translation are unit
tested, and so are the transitions of the stream state. `apps/ora-controller/tests/cloud.rs` drives
the runtime against an in-memory Cloud that serves the real contract through the test-only server
stubs of `ora-controller-proto` (`test-server` feature), covering signal-driven claiming, work
accepted before the stream opened, fallback and reopening after a broken stream, drains, a refused
stream after a drain, a stale epoch on opening, batching, and cancelling the stream and releasing the
lease on shutdown. The runtime and executable tests cover that the cloud form opens no local state, serves no
JSON surface, refuses a JSON section or listener flags, and stays up while Cloud is unreachable. Its
behavior against a real Cloud (lease, claim, dispatch, takeover, restart without a second clone) is
verified end to end with the [minicloud cloud form](../minicloud/runtime.md#cloud-persistence-mode) and is not yet an automated test. `apps/ora-controller/tests/workspaces.rs` drives
Workspace operations against a fake Cloud, a fake Substrate effect server and a fake Node: creating
and stopping a Workspace runs every step, registers the Node before the clone, reports idle and stops
the session before terminating without reporting that stop as a lost connection; an unclonable ref blocks the clone step without a dispatch; and quiesce reports busy
while a dispatch has no result. The Substrate client is unit tested for querying a timed-out effect
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
