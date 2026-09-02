# @ora-space/plugin-sdk

The Ora plugin SDK runs JavaScript plugins as persistent Deno processes. A
plugin registers its complete method set before calling `run()`:

```ts
import { createPlugin } from "@ora-space/plugin-sdk";

const plugin = createPlugin();
plugin.registerMethod("example.echo", (input) => input);
await plugin.run();
```

Methods receive JSON values and may return a value or a promise. Registration is
immutable once `run()` begins; duplicate method names and late registration are
rejected. Ora invokes independent requests concurrently and correlates responses
by their JSON-RPC request IDs.

## Process contract

The SDK reserves stdout for Ora's binary protocol. Each frame starts with a
four-byte big-endian length, followed by the one-byte JSON-RPC frame type and a
UTF-8 JSON payload. Frames larger than 16 MiB and malformed host messages stop
the plugin.

When the default Deno transport starts, the SDK redirects all `console` methods
to stderr so normal plugin diagnostics cannot corrupt stdout. Plugins receive no
Deno permissions unless the Ora host grants them when launching the process; ui
plugins receive none at all and reach their data through the storage client
below.

`run()` sends a single `ora/register` notification, serves host traffic until it
receives `ora/shutdown` or stdin closes, then waits for current handlers to
settle before returning.

## Notifications

Registration declares both directions. `registerMethod` lists what Ora may call;
`declareEmit` lists what the plugin may send unprompted. Ora rejects any
plugin-sent method outside that whitelist and terminates the process, so an
undeclared `notify()` is a defect rather than a dropped message.
`onNotification` handles host-sent notifications, which never produce a
response; an unhandled one is logged rather than treated as fatal.

Throw `PluginMethodError` from a handler to control the JSON-RPC error code Ora
sees; a plain `Error` becomes `-32603`.

## Host requests and storage

`plugin.request(method, params, { timeoutMs })` sends a JSON-RPC request to Ora
and resolves with its result. Host methods need no declaration; Ora answers
`method_not_found` for anything it does not serve. Failures reject with
`HostRequestError`, whose `kind` is the host's `data.kind` when present,
`method_not_found`, `timeout` (default 30 s), or `transport` (the process
stopped first).

`createStorage(plugin)` (also available as `ui.storage` from `defineUiPlugin`)
wraps the `ora/storage/*` methods. Paths are logical, slash-separated, and
relative to the plugin's private data directory; Ora resolves them by the
calling plugin's identity and refuses absolute paths, `..`, symlinks, and the
host-owned `web-profile/` directory.

```ts
const entries = await storage.list("downloads"); // [{ name, kind, sizeBytes }]
const bytes = await storage.read("downloads/skill.zip"); // Uint8Array, ≤ 8 MiB
await storage.write("index.json", new TextEncoder().encode("{}")); // atomic
await storage.remove("index.json"); // file or directory tree
```

Storage errors carry `kind` `invalid_path`, `not_found`, `too_large`, `io`, or
`invalid_params`.

## Host-managed child processes

`createHostProcesses(plugin)` wraps `ora/childprocess/*`: instead of a plugin
spawning its own subprocess (which on Deno needs `--allow-run`), it asks Ora to
spawn, own, and best-effort kill one on its behalf. Ora tears down every process
a plugin spawned this way the moment that plugin generation stops for any
reason, on top of whatever a caller's own `kill()` requests.

```ts
import { createHostProcesses } from "@ora-space/plugin-sdk";

const processes = createHostProcesses(plugin); // before plugin.run()
const acp = await processes.spawn({
  command: "opencode",
  args: ["acp", "--cwd", cwd],
  cwd,
});
acp.stdout; // ReadableStream<Uint8Array> — this plugin owns any line framing
acp.stderr; // ReadableStream<Uint8Array>
await acp.write(new TextEncoder().encode(line)); // to the process's stdin
await acp.closeStdin(); // signals EOF without killing it
await acp.kill(); // best-effort tree-wide termination
const { code, signal } = await acp.exited;
```

A plugin can also run an executable its own package ships, by naming a
package-relative `packageCommand` instead of a `command`; Ora joins it onto that
plugin's install root, so the plugin never learns a host path and `cwd` stays
free to be the workspace the child runs in. The two fields are mutually
exclusive.

`spawn` failures carry `kind` `invalid_command` (empty command),
`program_not_found` (the OS could not resolve the executable — distinct from any
other spawn failure, which is `io`), `package_command_missing` (this package
carries nothing at that path), `invalid_package_command` (it carries something
there that cannot be run, or the path is not a portable package-relative one),
or `invalid_params`; `write`, `closeStdin`, and `kill` against an already-exited
process's id fail with `not_found`.

## UI plugins

`defineUiPlugin` builds a plugin that serves Ora's ui contract with the
`ora/ui/*` wire names, translating snake_case params into camelCase objects so
plugin code never spells a method name. It always declares `ora/ui/push`;
`ora/ui/download_completed` and `ora/ui/request` are registered only when the
matching handler is present, which is how Ora rejects an incomplete plugin at
the handshake (a `remote_site` surface requires `onDownloadCompleted`, a `panel`
surface requires `onRequest`).

```ts
import { defineUiPlugin } from "@ora-space/plugin-sdk";

const ui = defineUiPlugin({
  onSurfaceOpened: (
    session,
  ) => {/* session.surfaceId, .surfaceInstanceId, .pluginGeneration */},
  onSurfaceClosed: (session) => {},
  onDownloadCompleted: async ({ session, download }) => {
    // download.path is "downloads/<fileName>", readable through storage
    const bytes = await ui.storage.read(download.path);
  },
  onRequest: ({ session, payload }) => ({ echo: payload }),
});
await ui.run();
```

`ui.push(session, payload)` sends `ora/ui/push` to the panel page of `session`;
delivery is best-effort and Ora drops pushes whose `pluginGeneration` is not the
current process. `ui.plugin` exposes the underlying `Plugin`.

## Agent plugins

`defineAgent` builds a plugin that serves Ora's agent contract — `agent/start`,
`agent/stop`, `agent/list_models`, and the `agent/acp` notification in both
directions. Ora validates that whole contract when the handshake completes and
refuses a plugin whose declaration is incomplete, so the helper registers all of
it up front.

```ts
import {
  AGENT_NOT_INSTALLED,
  defineAgent,
  PluginMethodError,
  SKILL_DIRECTORY_V1,
} from "@ora-space/plugin-sdk";

let send;
const plugin = defineAgent({
  start: (context, sender) => {
    send = sender; // spawn the agent CLI here and own its lifetime
  },
  stop: () => {/* terminate the CLI this plugin spawned */},
  listModels: ({ cwd }) => [{ id: "opus", displayName: "Opus", default: true }],
  onAcp: (frame) => {/* forward the frame to the CLI */},
  effects: {
    resources: [{
      workspaceRelativePath: ".agents/skills",
      materializationFormat: SKILL_DIRECTORY_V1,
      coordination: "quiesce_before_mutation",
    }],
    coordinate: async ({ targetId, resourceIds }) => {
      // Quiesce every instance that could consume this exact Resource set and retain the barrier.
      return { targetId, resourceIds, state: "safe_to_mutate" };
    },
    reactivate: async ({ targetId, resourceIds }) => {
      // Reinitialize affected instances after verification, then release the retained barrier.
      return { targetId, resourceIds, state: "reactivated" };
    },
    verifyReady: async ({
      targetId,
      generation,
      consumerRevisionId,
      projectionDigest,
    }) => {
      // Confirm the Agent can consume this exact immutable Target projection.
      return { targetId, generation, consumerRevisionId, projectionDigest };
    },
  },
});
await plugin.run();
```

The plugin spawns and owns its agent process. Ora never touches that process's
stdio; it only sees `agent/acp` frames, whose payloads it passes through without
parsing. Throw `new PluginMethodError(AGENT_NOT_INSTALLED, ...)` from `start`
when the CLI is absent — Ora treats that as expected local configuration and
retries quietly instead of reporting a fault. Throw `AGENT_UNUSABLE` instead
when the CLI this package ships cannot run at all: that failure repeats on every
attempt, so Ora reports it once and stops retrying that agent.

### Model discovery

`listModels` is called on demand — when a user opens a chat surface or a workflow
inspector — never as part of bringing the agent up, and it receives the
Workspace directory the models are being listed for. Ora keeps no copy of the
answer: the plugin owns the catalog and decides when its own cache is stale.
Returning an empty list is a valid answer for an agent that has no models to
offer before a session exists; its models then arrive with the session's ACP
`config_options` instead.

Most agents only expose their models through ACP session configuration, which
means discovery has to run one. Do that on a **separate, one-shot agent
process**, and ask the host to start it:

```ts
listModels: async ({ cwd }) => {
  const probe = await spawnAgentProcess(processes, {
    packageCommand: "bin/opencode",
    command: "opencode",
  }, { args: ["acp", "--cwd", cwd], cwd });
  // initialize → session/new(cwd) → read config_options → end the process
}
```

Two constraints, both load-bearing:

- **Not the connection you gave Ora.** `listModels` runs before Ora's own ACP
  `initialize`, Ora's `initialize` is what declares the client capability that
  decides whether the agent reports a model selector at all, and any request you
  inject returns down the same pipe Ora is reading. A second process avoids all
  three, and its probe session disappears with it — no `session/delete` needed.
- **Not a process you spawn yourself.** Every plugin child process goes through
  `ora/childprocess/spawn` (`createHostProcesses`), because the host owns the OS
  handles, terminates process trees, and reclaims everything a plugin generation
  left behind. Discovery is the likeliest thing to fail halfway — a missing CLI,
  a timed-out handshake — and a failure outside that ownership leaves an orphan
  agent process behind. A sandboxed plugin also cannot compute the host path of
  its own bundled executable, which is why `packageCommand` exists.

Effect locators are always Workspace-relative; Ora supplies and validates the
absolute Workspace root when it coordinates a mutation. The canonical Plugin ID
becomes the persisted consumer identity, so plugin code cannot claim another
consumer's state. Both coordination callbacks must be idempotent because Ora may
retry after either side has completed but before the corresponding durable
status update is visible.

### Bundled CLI or the user's own

An agent package is published one of two ways: with the CLI bundled (a
`[[targets]]` release, one package per target triple) or without it, resolving
whatever the user installed from PATH (a universal `url`/`sha256` release). The
same plugin source serves both — it cannot know at build time which package it
ended up in — so name both programs and let `spawnAgentProcess` resolve them:

```ts
import { spawnAgentProcess } from "@ora-space/plugin-sdk";

const acp = await spawnAgentProcess(processes, {
  packageCommand: Deno.build.os === "windows"
    ? "bin/opencode.exe"
    : "bin/opencode",
  command: "opencode",
}, { args: ["acp", "--cwd", cwd], cwd });
```

The bundled path is tried first. It falls through to the PATH lookup on exactly
one condition — Ora answering that this package carries no such file — and
raises `AGENT_UNUSABLE` for any other failure of a bundled executable, so a
broken package is never masked by a PATH lookup that happens to succeed. A PATH
lookup that finds nothing raises `AGENT_NOT_INSTALLED`.

`command` also takes several spellings, tried in order, for a CLI whose
installers disagree about what lands on PATH — a native `tool.exe` against the
`tool.cmd` shim npm and bun write, which Ora's PATH lookup will not find from
the bare name:

```ts
command: ["codeagent.exe", "codeagent.cmd", "codeagent"];
```

Only "not on PATH" moves to the next spelling; a candidate that started and then
failed is raised as-is, so a real fault is never buried under the next attempt.

A plugin whose CLI is only ever distributed on its own has no bundled form to
discover, and says so by leaving `packageCommand` out entirely:

```ts
const acp = await spawnAgentProcess(processes, {
  command: ["codeagent.exe", "codeagent.cmd", "codeagent"],
}, { args: ["acp"], cwd });
```

That starts at the PATH lookup and asks the host nothing about the package.
Naming a path the package is known not to carry would reach the same CLI, but it
claims a bundled executable may exist there — and one that turns out to exist
and not run fails the agent outright rather than falling back.
