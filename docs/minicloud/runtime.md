# minicloud local runtime

English | [中文](runtime.zh.md)

## One-command development environment

Run `task run:minicloud` at the repository root. The launcher installs frontend dependencies, builds
debug binaries, then starts host, `ora-controller --single-node` (which starts Node itself), and Vite.
Open `http://127.0.0.1:5174`. Linux, Deno, Cargo, Node.js, Git and `setsid` are required; root is not.

Development configuration and runtime data live under `~/.ora/minicloud/<digest>/`, where `<digest>`
is derived from the checkout path and the `workspace` file records that path. Each checkout therefore
keeps separate state; a directory claimed by another checkout is rejected. The launcher writes nothing
into the repository:

- `config/node.json`, `controller.json`, `client.json`: Node and Controller deployment configuration
  (the Controller file names `node.json` in its `single_node` section), plus the Vite port and the
  Controller's loopback `controllerPort` (default 4820) that the launcher passes on the command line.
  An older `server.json` from the removed minicloud server is ignored.
- `config/clone.gitconfig`: noninteractive Git configuration; the default supports credential-free HTTPS repositories. Configure private-repository credentials explicitly.
- `node/`, `controller/`: databases and Node IPC; `p/`: host/guardian state (short to leave room for Unix socket names).
- `repositories/`: clones; `home/`: workload HOME; `bin/`: identifiable versioned guardians; `vite/`: Vite cache.

Repeated runs preserve configuration edits, databases and checkouts; failed recovery never falls back
to clearing state. Logs go to the terminal. Dependencies and build outputs retain the standard
repository `node_modules`/`target` locations. Use `deno run -A scripts/run-minicloud.ts --init-only`
to initialize without starting, or `--no-build` to skip installation and compilation. Restart after
changing ports; launcher-owned state paths must not point at a different deployment.

Ctrl+C or unexpected component exit stops Vite, then the Controller, which retires its Node and waits
for managed Git cleanup, then host and guardians belonging to this directory. Node shares the
Controller's process group, so the launcher's group stop reaches both even after a Controller crash.
Stop timeouts are reported before escalating signals; escaped workload descendants are not guaranteed to
terminate. Interrupted clones may remain
pending/unknown for recovery; they are never automatically recreated or deleted. An exclusive lock
rejects a second launcher for the same data directory.

Debug builds skip Unix permission-bit checks on trusted paths, allowing group-writable checkouts
without changing existing permissions. Owner, symlink, hard-link, type, directory-isolation and database
ownership checks remain enabled; release builds still enforce permission bits. State is keyed to the
home directory rather than the checkout because Unix socket paths are limited to 108 bytes; an
unusually long real home path is rejected, not shortened or redirected through a symlink.

## Cloud persistence mode

`task run:minicloud -- --cloud` starts the same host, `ora-controller --single-node` and Node, but the
Controller runs with [cloud persistence](../controller/local-runtime.md#independent-executable): Cloud
holds every durable fact and the Controller only calls out to it, so it opens no SQLite database and
no listener, and the minicloud frontend is not started. Requests enter through Cloud's tenant clone
API instead.

Cloud is started separately and must come from a revision that serves the clone API and accepts
unauthenticated Controllers. In the Cloud repository, copy `config.toml.template` to `config.toml`
and run `task setup` once (PostgreSQL DSN, Gateway keys, `.local/dev.env`, migrations), then run
`task run` for the Cloud server (HTTP `:8080`, Controller gRPC `:8082`) and `task run:gateway` for
its authentication Gateway (`:8081`); Cloud's `docs/gateway.md` describes both. The Controller
presents no credential at this stage: it names itself with `controller_id`, which Cloud records as
the lease holder.

State lives under `~/.ora/cloud/<digest>/`, apart from the local mode's directory: a Node's journal
belongs to one persistence authority, so work accepted by SQLite is never reported to Cloud and clone
destinations are never shared. The layout matches the local mode without `client.json` and `vite/`;
`config/controller.json` selects `persistence: cloud` with Cloud's gRPC endpoint and the claim
interval, and has no `api` section. Edit it to reach Cloud elsewhere. The
launcher refuses a `controller.json` whose persistence does not match the mode.

Startup warns once when Cloud's gRPC endpoint is unreachable but continues, because the Controller
keeps retrying Cloud. The Controller has no port to probe, so readiness means only that it is still
running two seconds after start, not that it holds Cloud's lease; later exits stop all components as
in the local mode. Clones are submitted like any Cloud client's requests: through the Gateway, with a
session from its development login and a tenant of that user. Nothing on this side holds a key or
token. With the Gateway on `:8081` and its default public origin `http://localhost:5173`, which
state-changing requests must name in `Origin`:

```bash
G=http://localhost:8081 O=http://localhost:5173 JAR=$(mktemp)
# Development login: start an attempt, post the identity form, follow the callback to the session cookie.
AUTH=$(curl -s -c "$JAR" -H "Origin: $O" -H 'content-type: application/json' -d '{"provider":"dev"}' "$G/auth/login" | sed -E 's/.*"authorizationUrl":"([^"]*)".*/\1/; s/\\u0026/\&/g')
CALLBACK=$(curl -s -o /dev/null -w '%{redirect_url}' -b "$JAR" -c "$JAR" -H "Origin: $O" -d "${AUTH#*\?}" -d source=dev -d subject=minicloud "$G/auth/dev/authorize")
curl -s -o /dev/null -b "$JAR" -c "$JAR" "${CALLBACK/#$O/$G}"
# Tenant creation is idempotent per user and key, so rerunning this returns the same tenant.
TID=$(curl -s -b "$JAR" -H "Origin: $O" -H 'content-type: application/json' -H 'Idempotency-Key: minicloud' -d '{"name":"minicloud","slug":"minicloud"}' "$G/api/v1/tenants" | sed -E 's/.*"tenant":\{"id":"([^"]*)".*/\1/')
curl -s -b "$JAR" -H "Origin: $O" -H 'content-type: application/json' -H 'Idempotency-Key: r1' -d '{"requestId":"r1","repository":"https://github.com/octocat/Hello-World","branch":"master"}' "$G/api/v1/tenants/$TID/clones"
```

`curl -s -b "$JAR" "$G/api/v1/tenants/$TID/clones"` lists operations and `.../clones/{operationId}`
reads one; `state` moves from `pending` to `succeeded` (`path`, `commit`) or `failed` (`reason`,
optional `retainedPath`). Resubmitting the same `requestId` returns the original operation instead of
cloning again. Work accepted while the Controller is down
stays pending until it is claimed. After a run whose Controller was killed rather than stopped, the
previous lease is not released, so work waits until Cloud's 30-second lease expires.

## Manual deployment

The API is served by the `ora-controller` executable itself; there is no separate minicloud server.
Start host/guardian and either start Node using its existing
[deployment configuration](../node/repository-clone.md) or let the Controller host it with
`--single-node`. Then run `ora-controller --config /absolute/path/controller.json --transport tcp
--host 127.0.0.1 --port 4820 [--single-node]`; the configuration file, flags and hosting rules are
documented in the [Controller runtime](../controller/local-runtime.md#independent-executable).

The Node owner must match this ControllerId. All state paths remain explicitly injected and protected
from overlap, and one Controller data directory has one running owner. minicloud only uses loopback:
the launcher passes `--host 127.0.0.1` and the executable defaults to it. This is a non-production
application, with no authentication or additional security infrastructure.

## HTTP interface

After `deno install`, run `deno task --filter @ora/minicloud-client dev` from the repository root and
open `http://127.0.0.1:5174`. Vite proxies `/api` to `http://127.0.0.1:4820`; set
`MINICLOUD_SERVER_URL` when using a different Controller port. The page uses the shared shadcn components
with React 19 and polls Controller operations. An unresolved submission is saved in tab session storage
before dispatch; after response loss or reload, “retry original request” reuses its identity and input.
This storage is not operation history. Closing the page aborts HTTP and polling, not the Node execution.

- `POST /api/clones`: `{ "requestId": "stable-client-id", "repository": "https://host/repo.git", "branch": "main" }`.
  Returns HTTP 202 with `requestId`, `operationId`, and `executionId` after durable acceptance.
- `GET /api/clones`: newest accepted operations first, including pending records while Node is offline.
- `GET /api/clones/{executionId}`: one operation; 404 means no accepted operation with that identity.

State is `pending`, `succeeded` (Node path and commit), or `failed` (known reason and retained path).
Pending makes no claim about whether Git is currently running. HTTP 400 means invalid input, 409 an
identity/input conflict and 503 temporary unavailability; HTTP errors are not terminal clone failures.
The browser DTOs are generated from `ora-contracts::controller_api`, separate from the Node wire
protocol; this JSON surface is transitional and is retired once the browser reaches Ora Cloud instead.

Repeat an uncertain submission with the same request identity and input. Closing a browser or stopping
the Controller does not cancel a Node execution. Restart with the same state directory to recover original facts.
There is no separate minicloud task database, cleanup command or automatic new-execution retry.

Real HTTP tests (`cargo test -p ora-controller`) cover acceptance, conflicts, invalid input, offline
listing, missing identities, exclusive ownership, composition rejection, the Unix-socket transport and
normal restart. Lower-level Controller crash tests remain separate from minicloud's own end-to-end
evidence.

Frontend checks: `deno task --filter @ora/minicloud-client lint`, `test`, and `build`.
`task test:minicloud` additionally runs real HTTP and Vite proxy → independent `ora-controller` →
Node → HTTPS Git tests, including Controller SIGKILL/restart and one mutation Run. It requires Linux and
installed frontend dependencies. The Vite case is explicitly opt-in for Rust-only CI runners.
The real chain also holds a SQLite writer lock to verify HTTP 503 without accepted intent, then releases
it and verifies a single Run. A real TCP proxy truncates the accepted response body; retry after
Controller restart preserves the original execution. Normal Controller shutdown during clone preserves
the pinned live Git process and eventually produces the same result with one Run. Entry tests reject
overlapping directories and preserve unknown files, and reuse intent accepted by a separate Controller
owner. The `--single-node` composition is verified separately: a Controller kill leaves its Node and Git
running, a live endpoint refuses a second hosting Controller, a replacement Controller takes over the
replayed result, and normal stop retires the hosted Node.
DOM tests verify UI behavior, automatic polling recovery and reload identity recovery. Full browser-engine
interaction acceptance is explicitly outside the agreed scope.
