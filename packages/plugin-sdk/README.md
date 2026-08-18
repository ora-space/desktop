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
Deno permissions unless the Ora host grants them when launching the process.

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
});
await plugin.run();
```

The plugin spawns and owns its agent process. Ora never touches that process's
stdio; it only sees `agent/acp` frames, whose payloads it passes through without
parsing. Throw `new PluginMethodError(AGENT_NOT_INSTALLED, ...)` from `start`
when the CLI is absent — Ora treats that as expected local configuration and
retries quietly instead of reporting a fault.
