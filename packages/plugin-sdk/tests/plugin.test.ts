import { createPlugin } from "../src/mod.ts";
import {
  decodeFrames,
  encodeFrame,
  type JsonValue,
  type PluginTransport,
} from "../src/protocol.ts";

/** Compares JSON-compatible values without a Node compatibility dependency. */
function assertEquals(actual: unknown, expected: unknown): void {
  const actualJson = JSON.stringify(actual);
  const expectedJson = JSON.stringify(expected);
  if (actualJson !== expectedJson) {
    throw new Error(`Expected ${expectedJson}, received ${actualJson}`);
  }
}

/** Verifies a synchronous operation fails with the expected message. */
function assertThrows(operation: () => void, pattern: RegExp): void {
  try {
    operation();
  } catch (error) {
    if (error instanceof Error && pattern.test(error.message)) {
      return;
    }
    throw error;
  }
  throw new Error(`Expected operation to throw ${pattern}`);
}

/** Creates paired in-memory streams for exercising the SDK without global stdio. */
function createTransportHarness(): {
  transport: PluginTransport;
  send: (message: JsonValue) => Promise<void>;
  responses: AsyncGenerator<unknown>;
} {
  const hostInput = new TransformStream<Uint8Array>();
  const pluginOutput = new TransformStream<Uint8Array>();
  const inputWriter = hostInput.writable.getWriter();
  return {
    transport: {
      readable: hostInput.readable,
      writable: pluginOutput.writable,
      redirectConsole: false,
    },
    send: (message) => inputWriter.write(encodeFrame(message)),
    responses: decodeFrames(pluginOutput.readable),
  };
}

Deno.test(
  "registers once and serves repeated calls from one run loop",
  async () => {
    const plugin = createPlugin();
    plugin.registerMethod("example.echo", (input) => input);
    const harness = createTransportHarness();
    const run = plugin.run(harness.transport);

    assertEquals((await harness.responses.next()).value, {
      jsonrpc: "2.0",
      method: "ora/register",
      params: { methods: ["example.echo"], emits: [] },
    });
    await harness.send({
      jsonrpc: "2.0",
      id: 1,
      method: "example.echo",
      params: "abc",
    });
    assertEquals((await harness.responses.next()).value, {
      jsonrpc: "2.0",
      id: 1,
      result: "abc",
    });
    await harness.send({
      jsonrpc: "2.0",
      id: 2,
      method: "example.echo",
      params: "Ora",
    });
    assertEquals((await harness.responses.next()).value, {
      jsonrpc: "2.0",
      id: 2,
      result: "Ora",
    });
    await harness.send({ jsonrpc: "2.0", method: "ora/shutdown" });
    await run;
  },
);

Deno.test("rejects duplicate and late method registration", async () => {
  const plugin = createPlugin();
  plugin.registerMethod("example.echo", (input) => input);
  assertThrows(
    () => plugin.registerMethod("example.echo", (input) => input),
    /already registered/,
  );

  const harness = createTransportHarness();
  const run = plugin.run(harness.transport);
  await harness.responses.next();
  assertThrows(
    () => plugin.registerMethod("example.other", (input) => input),
    /cannot change/,
  );
  await harness.send({ jsonrpc: "2.0", method: "ora/shutdown" });
  await run;
});

Deno.test("maps handler failures to JSON-RPC errors", async () => {
  const plugin = createPlugin();
  plugin.registerMethod("example.fail", () => {
    throw new Error("expected failure");
  });
  const harness = createTransportHarness();
  const run = plugin.run(harness.transport);
  await harness.responses.next();

  await harness.send({
    jsonrpc: "2.0",
    id: 3,
    method: "example.fail",
    params: null,
  });
  assertEquals((await harness.responses.next()).value, {
    jsonrpc: "2.0",
    id: 3,
    error: { code: -32603, message: "expected failure" },
  });
  await harness.send({ jsonrpc: "2.0", method: "ora/shutdown" });
  await run;
});
