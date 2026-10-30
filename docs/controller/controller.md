# Controller

English | [中文](controller.zh.md)

`ora-controller` (`apps/ora-controller`) carries work between an authority and the Nodes that
execute it. It owns no business decisions and never executes Git itself. It runs in one of two
deployment forms chosen once in configuration: **SQLite**, a local single-node coordinator, or
**cloud**, a stateless client that dials out to Ora Cloud and drives Workspace sandboxes. In neither
form does the executable open a listener. The executable is Linux-only. Agent session relay is in
[Agent relay](agent-relay.md); the Cloud contract in [Cloud contract](cloud-contract.md).

## Coordination store

Coordination logic reaches persistent state only through `CoordinationStore`: one atomic,
asynchronous business operation per method (`take_over_node_event`, `record_queried_result`,
`original_dispatch`, `pending_dispatches`, `result`, plus `serve` for the adapter's own work), never
a transaction, connection or table. One adapter is chosen at deployment time and neither is a
fallback for the other.

- **`SqliteStore`** keeps `ora-controller.sqlite3` in the private `home_directory`. Its operations
  run on the blocking pool so fsync never blocks the async runtime. The database has application id
  `0x4f524143`, schema version 1 and exact schema and integrity checks, and an OS lease on the sibling
  `ora-controller.sqlite3.lock` (beside the database so SQLite's own locks never collide with it on
  macOS or Windows). A different ControllerId or an unknown existing file is rejected; there is no
  HOME-derived location, reset or automatic rebinding. It reconciles, takes over and reports the
  clone dispatches its database already holds; it has no intake of new work.
- **`CloudStore`** opens no database and takes no file lease: every operation is a call on the
  [Cloud contract](cloud-contract.md), committed by Cloud in PostgreSQL. Acceptance belongs to
  Cloud's public API.

There is no local clone intake: a Workspace clone takes a Git Server snapshot input that only Cloud
can fix, so a deployment without Cloud cannot produce a valid one
([snapshot clone input and execution ref grants](https://github.com/ora-space/specs/blob/main/decisions/cloud/controller-integration/20261016-snapshot-clone-input-and-execution-ref-grants.md)).
`result(execution_id)` reads a durable terminal `ExecutionOutcome` (`ready { path,
commit }` or `failed { reason, retained_path }` plus the Node incarnation); absence is not proof of
failure. `pending_dispatches(node)` lists executions without a durable result, which a reconnected
session keeps querying. Query completion and event delivery share one takeover transaction, and only
a received event produces an Ack, after its receipt commits. Duplicate content is idempotent;
conflicting content is rejected without acknowledgement.

Optional Agent previews use connection-owned `session/previews.rs`: one latest snapshot per
execution, up to 64 concurrent workers, bounded downstream calls and cancellation with the connection.
Successful reliable terminal takeover removes that execution's snapshot and cancels its worker;
late previews cannot recreate its relay, while other live executions remain independent. The limit
does not accumulate across completed executions on a long-lived connection. This
path writes no durable coordination data, produces no Node Ack and cannot fail reliable Thread
takeover. A store advertises `supports_thread_previews=true` only when it implements the downstream
publisher. `CloudStore` advertises this capability; `SqliteStore` keeps it disabled. The Cloud adapter
maps settled assistant text, truncation and history positions, and provides an explicit preview
publisher. The publisher validates the Node envelope and registered session, rejects ended executions,
and sends one replacement snapshot under the current lease. It never retries through the durable
submission loop or produces an acknowledgement. Preview relay requires both Controller and Node to negotiate the capability; older Nodes retain the durable path.

## Running the executable

```text
ora-controller --config /absolute/path/controller.json [--single-node]
```

`ControllerRuntime::open(RuntimeConfig)` supports embedding, and `run(shutdown)` owns reconnect loops
without installing signal handlers.
`Service::start(DeploymentConfig, NodeHosting)` composes the runtime and an optionally hosted Node.
Invalid flags and configuration fail before any lease is taken.

```json
{
  "controller": {
    "home_directory": "/home/node/controller",
    "persistence": { "kind": "sqlite" },
    "controller_id": "deployment-controller",
    "protected_state_directories": ["/home/node/state", "/home/node/process"],
    "nodes": [
      { "node_id": "deployment-node", "endpoint": { "kind": "ipc", "path": "/home/node/state/control.sock" } }
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

- Declare every Node, process host and guardian state root in `protected_state_directories`; IPC
  socket parents are protected too, and any overlap with Controller state is rejected. Configure the
  Node's owner to this ControllerId.
- A Node in a sandbox is reached through its platform WebSocket router:
  `{ "kind": "websocket", "url": "wss://…", "headers": { "ate-target-actor": "atespace/sandbox-id" } }`.
  Headers are sent verbatim and carry vendor addressing and platform credentials; the handshake still
  verifies `node_id`.
- `--single-node` starts the configured Node, which must use an `ipc` endpoint whose
  `control.controller_id` and `control.listen` match, in its own process group, and waits up to
  `ready_timeout_ms` for the endpoint. The process host and guardian are prerequisites it does not
  start. If the hosted Node exits on its own, the Controller exits with failure.
- Normal shutdown stops Node sessions, then the adapter's own work (in the cloud form a bounded lease
  release), then the hosted Node (`SIGTERM`, waiting `stop_timeout_ms`, never `SIGKILL`), then the
  SQLite lease. Accepted Node executions are never cancelled by the Controller stopping.

Each failed or lost Node session is logged with its class (unreachable, unknown sandbox, refused,
busy, protocol, identity mismatch, silent Node, disconnected) and retried after `reconnect_ms`
without failing or re-creating any execution. The session protocol is in
[Control session](../node/control-session.md).

## Cloud form

```json
"management_tls": {
  "certificate_file": "/run/ora-management/controller.pem",
  "private_key_file": "/run/ora-management/controller-key.pem",
  "ca_file": "/run/ora-management/ca.pem"
},
"persistence": {
  "kind": "cloud",
  "endpoint": "https://cloud:8443",
  "claim_interval_ms": 1000,
  "substrate": {
    "effects_url": "https://sandbox-server:18000",
    "router_url": "wss://sandbox-server:18000/ora-node/v1",
    "atespace": "ora",
    "request_timeout_ms": 30000
  }
}
```

The executable refuses a cloud configuration without `management_tls` or with a non-`https://`
endpoint, and a `substrate` whose effects URL is not `https://` or whose router is not `wss://`.
Cloud authenticates the Controller by its client certificate; every call also carries the
`controller_id` as `x-ora-controller-id` metadata, which Cloud records as the lease and submission
holder. The channel connects lazily, so an unreachable Cloud is an unavailable authority at call time,
not a startup failure. HTTP/2 keepalive PINGs every 30 seconds (closing a connection whose PING goes
unanswered for 10 seconds) make a silently dropped connection visible.

### Lease, claims and dispatch

The adapter's `serve` task acquires Cloud's global lease and renews it every ten seconds. Every write
carries the lease epoch and a stable submission identity; only a lost reply is retransmitted, with
the same identity, so Cloud replays its recorded response instead of applying an effect twice. Cloud's
verdicts are mapped once, inside the adapter, to conflict (never retried as is), unavailable (nothing
committed), unknown (reply lost, same identity only) and stale eligibility (the lease is dropped and
re-acquired). While the lease is not held nothing is claimed, dispatched or acknowledged, and the
Controller never falls back to writing locally.

Claimed work is validated as a Node command and registered with `RecordDispatch` before the Node
session delivers it, in batches of at most 16 so a long queue never delays lease renewal. Every work
item names the Workspace sandbox it runs in (Agent sessions and Revision deliveries); Cloud hands out no
tenant-level clones, and an item without a target is logged once and left queued in Cloud. Without a
`substrate` section, claims start only after a session with the static Node completed its handshake in
this process, proving the configured `node_id` is the Node behind the endpoint; a dispatch names its
Node for good, so work registered under a wrong id would wait forever.

While holding the lease the Controller keeps one `Watch` stream open:

| Stream | Entered when | Claims |
|---|---|---|
| none | Start; the stream ended with an error; the lease epoch was dropped | Every `claim_interval_ms`, retrying the stream on each tick |
| live | Cloud sent its response headers | On opening, on every `WorkAvailable`/`OperationAvailable`, and after every renewal |
| drained | Cloud sent `Drain` or ended the stream cleanly | Never; the lease is kept and sessions continue |

Signals only accelerate claiming; ownership is decided when `RecordDispatch` commits, and the renewal
backstop picks up work whose signal was dropped. A drain means the Cloud instance is stopping: the
Controller keeps retrying and leaves the drain when a new stream opens, or falls back to periodic
claiming if a serving Cloud refuses the stream with anything but `UNAVAILABLE`.

### Workspace sandboxes

With `substrate` configured, the Controller also drives Cloud's Workspace operations (ADR
[Controller drives Workspace sandboxes](https://github.com/ora-space/specs/blob/main/decisions/controller/node-management/0-controller-drives-workspace-sandboxes.md)).
It claims one operation at a time with `ClaimOperation` (Cloud hands back the oldest runnable one and
decides the step order), records each step's effect plan, executes it and advances or defers the
operation with the claimed version:

| Step | What the Controller does |
|---|---|
| sandbox | Executes the ensure effect and advances with its evidence (`sandboxInstanceId`, `nodeId`) |
| node | Opens a Node session through `router_url` with `ate-target-actor: <atespace>/<externalId>` and waits for Cloud to list it connected |
| clone | Forwards the snapshot input Cloud froze (`clone_input`) to `RecordDispatch` and the Node verbatim, and advances once it is ready |
| plugin | Dispatches the frozen plugin install or removal plan, see [Node plugins](../node/plugins.md) |
| quiesce | Closes the sandbox's dispatch gate and reports idle only when nothing is pending or unresolved |
| terminate | Stops the Node session, then executes the terminate effect |
| cleanup | Executes the data-delete effect |

`effects_url` is the Substrate effect API (`GET`/`PUT /effects/{id}`). The Controller reads an effect
before writing it and re-reads the same id after a timeout or server error, so an effect whose reply
was lost is never sent twice under a new identity. A failed effect defers with `external_failure`, a
timed-out one with `substrate_timeout`, and an unconfirmed termination blocks with
`termination_unconfirmed`. The Controller never builds a clone input from Project or Workspace fields:
a clone step whose snapshot carries no `clone_input`, or one the Node protocol refuses (see
[Repository clone](../node/repository-clone.md#input)), blocks with `external_failure` without a
dispatch. A ready clone is taken over only when its commit equals the input's `base_commit`; otherwise
it is neither taken over nor acknowledged.

An unavailable Node receives one nonrenewing two-second handshake grace per ordinary operation
round, shared by node readiness, quiesce and clone disconnections. A missing target defers immediately;
a ready target proceeds immediately. The Controller submits `node_unavailable` without choosing a
retry delay: Cloud durably counts unavailable rounds and schedules 5/10/20/40/60-second backoff,
resetting the count only on real progress. Sessions and the global lease remain active.

An already registered clone keeps its original execution, Node and frozen input across rounds.
Each round polls for at most five seconds on the monotonic clock, then submits `clone_result_pending`;
Cloud schedules its next claim one second later. Reconnects and pending replies renew neither budget.
Known results are handled first. With no known result thirty minutes after the execution's durable
Cloud `created_at`, the Controller requests `blocked/clone_result_timeout`, which Cloud rechecks against
its durable registration and concurrent results. Cloud automatically takes over the original
execution's late known success or failure; the latter retains the existing clone failure budget.
Node-reported Unknown remains `clone_result_unknown` under the existing unresolved-outcome rules.
These waits do not authorize redispatch, cleanup or termination, and force-stop keeps its separate task.

Sandbox Node targets come only from Cloud's sandbox rows; a sandbox whose NodeId collides with a
static Node is refused, and the Controller never recreates a sandbox on its own. After a handshake it
registers the Node incarnation with `RegisterNode`, reports it connected every ten seconds, reports it
disconnected once when the session is lost (not when the Controller stopped it), and ends a previous
incarnation before registering a new one. When it first holds a lease epoch it calls
`ListLiveSandboxes`, starts a session for every listed sandbox and stops any it holds that is not
listed, so a restarted Controller reconnects before claiming operations. Routing details are in
[Sandbox routing](../operations/sandbox-routing.md).
