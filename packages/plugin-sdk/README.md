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
`agent/stop`, `agent/listModels`, and the `agent/acp` notification in both
directions. Ora validates that whole contract when the handshake completes and
refuses a plugin whose declaration is incomplete, so the helper registers all of
it up front.

```ts
import {
  AGENT_NOT_INSTALLED,
  defineAgent,
  PluginMethodError,
} from "@ora-space/plugin-sdk";

let send;
const plugin = defineAgent({
  start: (context, sender) => {
    send = sender; // spawn the agent CLI here and own its lifetime
  },
  stop: () => {/* terminate the CLI this plugin spawned */},
  listModels: () => [{ id: "opus", displayName: "Opus", default: true }],
  onAcp: (frame) => {/* forward the frame to the CLI */},
  effects: {
    surfaces: [{
      workspaceRelativePath: ".agents/skills",
      materializationFormat: "skill_directory.v1",
      coordination: "wait_for_idle_and_restart",
    }],
    waitForIdle: async ({ surfaceKey, workspaceRoot, relativePath }) => {
      // Return waiting_for_idle while any affected instance is serving a turn. Once ready is
      // returned, keep new turns behind the surfaceKey barrier until restart.
      return "ready";
    },
    restart: async ({ surfaceKey, generation }) => {
      // Restart every affected instance, then release the idempotent barrier for this generation.
    },
  },
});
await plugin.run();
```

The plugin spawns and owns its agent process. Ora never touches that process's
stdio; it only sees `agent/acp` frames, whose payloads it passes through without
parsing. Throw `new PluginMethodError(AGENT_NOT_INSTALLED, ...)` from `start`
when the CLI is absent — Ora treats that as expected local configuration and
retries quietly instead of reporting a fault.

Effect locators are always Workspace-relative; Ora supplies and validates the
absolute Workspace root when it coordinates a mutation. The canonical Plugin ID
becomes the persisted consumer identity, so plugin code cannot claim another
consumer's state. Both coordination callbacks must be idempotent because Ora may
retry after either side has completed but before the corresponding durable
status update is visible.
