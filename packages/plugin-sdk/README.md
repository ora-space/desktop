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

`run()` sends a single `ora/register` notification, serves requests until it
receives `ora/shutdown` or stdin closes, then waits for current handlers to
settle before returning.
