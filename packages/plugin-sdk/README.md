# @ora-space/plugin-sdk

The Ora plugin SDK is the in-process library for Ora plugins: it implements the
plugin side of Ora's stdio protocol and ships the base classes and bridges each
plugin family needs. It is published to JSR as `@ora-space/plugin-sdk` (with an
npm mirror) from this directory, which is the only source of truth — plugins
depend on a released version and never vendor these files.

```jsonc
// deno.json in a plugin package
{
  "imports": {
    "@ora-space/plugin-sdk": "jsr:@ora-space/plugin-sdk@^1.0.0",
    "@ora-space/plugin-sdk/": "jsr:@ora-space/plugin-sdk@^1.0.0/"
  },
  "lock": true
}
```

## Entry points

| Specifier                       | Contents                                                                                                                       |
| ------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| `@ora-space/plugin-sdk`         | Protocol frames, `Plugin` (methods, notifications, emits, contracts), `PluginMethodError`, `SDK_VERSION`, `PLUGIN_API_VERSION` |
| `@ora-space/plugin-sdk/agent`   | The agent contract: `defineAgent`, `AgentPlugin` base class, `runAgentPlugin`, wire-name route tables, `AGENT_NOT_INSTALLED`   |
| `@ora-space/plugin-sdk/acp`     | `AcpProcessBridge` (owns an ACP child process and re-frames NDJSON ⇄ Ora frames), NDJSON codec, command-candidate helpers      |
| `@ora-space/plugin-sdk/testing` | `HostSimulator`: launches a plugin with Deno and drives it exactly like the Ora host                                           |

## Core

```ts
import { createPlugin } from "@ora-space/plugin-sdk";

const plugin = createPlugin();
plugin.registerMethod("example.echo", (input) => input);
await plugin.run();
```

A plugin registers its complete capability set before `run()`: methods it
serves, notifications it handles (`onNotification`), notifications it may emit
(`declareEmit`), and the contract versions it implements (`declareContract`).
Registration is immutable once `run()` begins. `run()` sends one `ora/register`
notification carrying `methods`, `emits`, `sdkVersion`, and `contracts` (always
including `pluginApi`), serves requests until `ora/shutdown` or stdin EOF, then
waits for in-flight handlers to settle.

Throw `PluginMethodError(code, message)` from a handler to control the JSON-RPC
error code the host sees; any other error maps to `-32603`.

## Agent plugins

```ts
import { AgentPlugin, runAgentPlugin } from "@ora-space/plugin-sdk/agent";
import { AcpProcessBridge, spawnPipedProcess } from "@ora-space/plugin-sdk/acp";

class MyAgent extends AgentPlugin {
  #send: AcpSender | undefined;
  readonly #bridge = new AcpProcessBridge({
    spawn: (cwd) => spawnPipedProcess("my-acp-server", [], cwd),
    onAcpFrame: (frame) => void this.#send?.(frame),
    logTag: "my-agent",
  });
  override onStart = async (ctx, send) => {
    this.#send = send;
    await this.#bridge.start(ctx.cwd);
  };
  override onStop = () => this.#bridge.stop();
  override onAcp = (frame) => this.#bridge.forwardAcp(frame);
  override onListModels = () => [];
}
await runAgentPlugin(new MyAgent(), { pluginId: "my.agent" });
```

`AgentPlugin` declares the required APIs as `abstract` so an incomplete plugin
fails to compile; `runAgentPlugin` flattens the instance into a wire-name keyed
dispatch table (fields such as `override onStart = …` count), redirects every
`console` method to stderr before activation, and adapts the instance onto
`defineAgent`, which owns the registration handshake and response shapes.

## Process contract

Stdout is reserved for Ora's binary protocol: a four-byte big-endian length, one
frame-type byte (`0x01`), and a UTF-8 JSON payload, at most 16 MiB. All
`console` methods are redirected to stderr when the Deno transport starts.
Plugins receive only the Deno permissions the host grants at launch.

## Testing a plugin

```ts
import { HostSimulator } from "@ora-space/plugin-sdk/testing";

const host = await HostSimulator.launch({
  entrypoint: import.meta.resolve("../src/main.ts"),
});
await host.request("agent/start", { cwd: Deno.cwd(), hostVersion: "0.9.0" });
const init = await host.acpRequest(1, "initialize", { protocolVersion: 1 });
await host.request("agent/stop");
await host.shutdown();
```

## Development

- `deno task check` / `deno task lint` / `deno task test` / `deno task format`
- `deno task publish:dry-run` validates the JSR package locally.

## Releasing

Bump `version` in both `deno.json` and `package.json` and the `SDK_VERSION`
constant in `src/version.ts` to the same value, then push a tag
`plugin-sdk/vX.Y.Z`. The `plugin-sdk-publish` workflow verifies the three agree,
runs the checks, publishes to JSR via OIDC, and publishes the npm mirror.
