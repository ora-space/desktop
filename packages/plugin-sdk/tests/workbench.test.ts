import { defineWorkbenchPlugin, type JsonValue } from "../src/mod.ts";
import {
  decodeFrames,
  encodeFrame,
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

/** Creates paired in-memory streams for exercising the SDK without global stdio. */
function createTransportHarness(): {
  transport: PluginTransport;
  send: (message: JsonValue) => Promise<void>;
  responses: AsyncGenerator<unknown>;
} {
  const hostInput = new TransformStream<Uint8Array>();
  const pluginOutput = new TransformStream<Uint8Array>(
    undefined,
    undefined,
    new CountQueuingStrategy({ highWaterMark: Infinity }),
  );
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
  "registers only its methods and unpacks the host envelope into a call",
  async () => {
    let lastCall: JsonValue | undefined;
    const workbench = defineWorkbenchPlugin({
      methods: {
        "counter/get": (call) => {
          lastCall = call as unknown as JsonValue;
          return { value: 42 };
        },
      },
    });

    const harness = createTransportHarness();
    const run = workbench.run(harness.transport);

    // The registration declares exactly one method and no emit.
    assertEquals((await harness.responses.next()).value, {
      jsonrpc: "2.0",
      method: "ora/register",
      params: { methods: ["counter/get"], emits: [] },
    });

    await harness.send({
      jsonrpc: "2.0",
      id: 1,
      method: "counter/get",
      params: {
        surface: { instance_id: 7, generation: 3 },
        input: { city: "SH" },
      },
    });
    const response = (await harness.responses.next()).value;

    assertEquals(response, {
      jsonrpc: "2.0",
      id: 1,
      result: { value: 42 },
    });
    assertEquals(lastCall, {
      surface: { instanceId: 7, generation: 3 },
      input: { city: "SH" },
    });

    await harness.send({ jsonrpc: "2.0", method: "ora/shutdown" });
    await run;
  },
);

Deno.test("rejects a call missing its surface envelope", async () => {
  const workbench = defineWorkbenchPlugin({
    methods: { "counter/get": () => null },
  });

  const harness = createTransportHarness();
  const run = workbench.run(harness.transport);
  await harness.responses.next();

  await harness.send({
    jsonrpc: "2.0",
    id: 1,
    method: "counter/get",
    params: { input: {} },
  });
  const response = (await harness.responses.next()).value as {
    error?: { code: number };
  };

  assertEquals(response.error?.code, -32602);

  await harness.send({ jsonrpc: "2.0", method: "ora/shutdown" });
  await run;
});
