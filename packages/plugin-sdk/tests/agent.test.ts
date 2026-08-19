import {
  AGENT_CONTRACT_VERSION,
  AGENT_NOT_INSTALLED,
  AgentPlugin,
  defineAgent,
  runAgentPlugin,
} from "../src/agent/mod.ts";
import {
  decodeFrames,
  encodeFrame,
  type JsonValue,
  PLUGIN_API_VERSION,
  PluginMethodError,
  type PluginTransport,
  SDK_VERSION,
} from "../src/mod.ts";

/** Compares JSON-compatible values without a Node compatibility dependency. */
function assertEquals(actual: unknown, expected: unknown): void {
  const actualJson = JSON.stringify(actual);
  const expectedJson = JSON.stringify(expected);
  if (actualJson !== expectedJson) {
    throw new Error(`Expected ${expectedJson}, received ${actualJson}`);
  }
}

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

Deno.test("defineAgent registers the full agent contract", async () => {
  const frames: JsonValue[] = [];
  let sender: ((frame: JsonValue) => Promise<void>) | undefined;
  const plugin = defineAgent({
    start: (_context, send) => {
      sender = send;
    },
    stop: () => {},
    listModels: () => [{ id: "a/b", displayName: "b" }],
    onAcp: (frame) => {
      frames.push(frame);
    },
  });
  const harness = createTransportHarness();
  const run = plugin.run(harness.transport);

  assertEquals((await harness.responses.next()).value, {
    jsonrpc: "2.0",
    method: "ora/register",
    params: {
      methods: ["agent/start", "agent/stop", "agent/listModels"],
      emits: ["agent/acp"],
      sdkVersion: SDK_VERSION,
      contracts: {
        pluginApi: PLUGIN_API_VERSION,
        agent: AGENT_CONTRACT_VERSION,
      },
    },
  });

  await harness.send({
    jsonrpc: "2.0",
    id: 1,
    method: "agent/start",
    params: { cwd: "/tmp", hostVersion: "0.9.0" },
  });
  assertEquals((await harness.responses.next()).value, {
    jsonrpc: "2.0",
    id: 1,
    result: { protocol: "acp", acpVersion: 1 },
  });

  await harness.send({
    jsonrpc: "2.0",
    method: "agent/acp",
    params: { jsonrpc: "2.0", id: 7, method: "initialize" },
  });
  // The in-memory transport has no buffer, so the emitted frame resolves only once it is read.
  const emitted = sender!({ jsonrpc: "2.0", id: 7, result: {} });
  assertEquals((await harness.responses.next()).value, {
    jsonrpc: "2.0",
    method: "agent/acp",
    params: { jsonrpc: "2.0", id: 7, result: {} },
  });
  await emitted;
  assertEquals(frames, [{ jsonrpc: "2.0", id: 7, method: "initialize" }]);

  await harness.send({
    jsonrpc: "2.0",
    id: 2,
    method: "agent/listModels",
    params: {},
  });
  assertEquals((await harness.responses.next()).value, {
    jsonrpc: "2.0",
    id: 2,
    result: { models: [{ id: "a/b", displayName: "b", default: false }] },
  });

  await harness.send({ jsonrpc: "2.0", method: "ora/shutdown" });
  await run;
});

Deno.test("agent/start rejects an empty cwd and surfaces not-installed", async () => {
  const plugin = defineAgent({
    start: () => {
      throw new PluginMethodError(AGENT_NOT_INSTALLED, "missing cli");
    },
    stop: () => {},
    listModels: () => [],
    onAcp: () => {},
  });
  const harness = createTransportHarness();
  const run = plugin.run(harness.transport);
  await harness.responses.next();

  await harness.send({
    jsonrpc: "2.0",
    id: 1,
    method: "agent/start",
    params: { cwd: "  ", hostVersion: "0.9.0" },
  });
  assertEquals((await harness.responses.next()).value, {
    jsonrpc: "2.0",
    id: 1,
    error: { code: -32602, message: "agent/start requires a non-empty cwd" },
  });
  await harness.send({
    jsonrpc: "2.0",
    id: 2,
    method: "agent/start",
    params: { cwd: "/tmp", hostVersion: "0.9.0" },
  });
  assertEquals((await harness.responses.next()).value, {
    jsonrpc: "2.0",
    id: 2,
    error: { code: AGENT_NOT_INSTALLED, message: "missing cli" },
  });
  await harness.send({ jsonrpc: "2.0", method: "ora/shutdown" });
  await run;
});

Deno.test("runAgentPlugin refuses a class that lacks a required route", async () => {
  // Bypass the compile-time guarantee on purpose: the runtime check is what protects a plugin
  // built with a looser TypeScript configuration or plain JavaScript.
  const Broken = class extends AgentPlugin {
    onStart() {}
    onAcp() {}
    onListModels = undefined as unknown as () => never;
  };
  let message = "";
  try {
    await runAgentPlugin(new Broken(), { pluginId: "test.broken" });
  } catch (error) {
    message = error instanceof Error ? error.message : String(error);
  }
  assertEquals(
    message,
    "Agent plugin does not implement onListModels, required for agent/listModels",
  );
});
