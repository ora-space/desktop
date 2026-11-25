import {
  type AcpSender,
  AGENT_DISCOVERY_CLEANUP_RESERVE_MS,
  AGENT_NOT_INSTALLED,
  defineAgent,
  PluginMethodError,
  waitForAgentCommands,
} from "../src/mod.ts";
import {
  AGENT_DISCOVERY_BUDGET_MS,
  decodeFrames,
  encodeFrame,
  type JsonValue,
  type PluginTransport,
} from "../src/protocol/index.ts";

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
  // A default TransformStream has a zero readable high-water mark, so a write's promise only
  // resolves once a reader pulls it. Unbounded queuing here decouples writes from reads the way a
  // real stdio pipe does, so a plugin can await `notify` before the harness reads the response.
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

Deno.test("a generic Agent must apply the exact model context before start succeeds", async () => {
  for (const acknowledges of [false, true]) {
    let received: unknown;
    const plugin = defineAgent({
      start: (context) => {
        received = context.modelContext;
        if (acknowledges && context.modelContext) {
          const frozen = context.modelContext.configuration;
          return {
            adapterVersion: frozen.adapterVersion,
            declarationFingerprint: frozen.declarationFingerprint,
            configurationRevision: frozen.configurationRevision,
          };
        }
      },
      stop: () => {},
      discover: () => ({ models: [] }),
      onAcp: () => {},
    });
    const harness = createTransportHarness();
    const run = plugin.run(harness.transport);
    await harness.responses.next();
    const modelContext = {
      configuration: {
        pluginId: "official/minimal",
        pluginVersion: "1.0.0",
        target: "x86_64-unknown-linux-gnu",
        packageSha256: "a".repeat(64),
        schemaVersion: 2,
        declarationFingerprint: "b".repeat(64),
        adapterVersion: 1,
        configurationRevision: 7,
        settingsJson:
          '{"style":"literal $(text)","budget":2.5,"verbose":false,"counter":9007199254740993,"ratio":0.123456789012345678901}',
        protocol: "anthropic-messages",
        model: {
          id: "vendor/model",
          name: "Model",
          contextWindow: 128000,
          maxTokens: 8192,
        },
      },
      proxyBaseUrl: "https://gateway.invalid/runtime/anthropic/v1",
      token: "synthetic-runtime-token",
    };
    await harness.send({
      jsonrpc: "2.0",
      id: 1,
      method: "agent/start",
      params: {
        cwd: "/private/execution",
        hostVersion: "0.0.0",
        modelContext,
      },
    });
    const response = (await harness.responses.next()).value;
    assertEquals(received, modelContext);
    assertEquals(
      response,
      acknowledges
        ? {
          jsonrpc: "2.0",
          id: 1,
          result: {
            protocol: "acp",
            acpVersion: 1,
            modelApplied: {
              adapterVersion: 1,
              declarationFingerprint: "b".repeat(64),
              configurationRevision: 7,
            },
          },
        }
        : {
          jsonrpc: "2.0",
          id: 1,
          error: {
            code: -32602,
            message: "invalid or unapplied Agent model context",
          },
        },
    );
    await harness.send({ jsonrpc: "2.0", method: "ora/shutdown", params: {} });
    await run;
  }
});

Deno.test("serves the whole agent contract over one run loop", async () => {
  const received: JsonValue[] = [];
  const discoveryContexts: unknown[] = [];
  let send: AcpSender | undefined;
  const plugin = defineAgent({
    start: (_context, sender) => {
      send = sender;
    },
    stop: () => {},
    discover: ({ deadline: _, ...context }) => {
      discoveryContexts.push(context);
      return {
        models: [{ id: "opus", displayName: "Opus" }],
        // Returned even when not wanted, which must not put it on the wire.
        commands: [{ name: "review", description: "Review changes" }],
      };
    },
    onAcp: (frame) => {
      received.push(frame);
    },
  });
  const harness = createTransportHarness();
  const run = plugin.run(harness.transport);

  assertEquals((await harness.responses.next()).value, {
    jsonrpc: "2.0",
    method: "ora/register",
    params: {
      methods: [
        "agent/start",
        "agent/stop",
        "agent/discover",
        "agent/list_models",
      ],
      emits: ["agent/acp"],
    },
  });

  await harness.send({
    jsonrpc: "2.0",
    id: 1,
    method: "agent/start",
    params: { cwd: "/home/user", hostVersion: "0.8.0" },
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
  await harness.send({
    jsonrpc: "2.0",
    id: 2,
    method: "agent/list_models",
    params: { cwd: "/home/user/project" },
  });
  assertEquals((await harness.responses.next()).value, {
    jsonrpc: "2.0",
    id: 2,
    result: { models: [{ id: "opus", displayName: "Opus", default: false }] },
  });
  const discoverMessage = (id: number, includeCommands: boolean) => ({
    jsonrpc: "2.0" as const,
    id,
    method: "agent/discover",
    params: { cwd: "/home/user/project", includeCommands },
  });
  const models = [{ id: "opus", displayName: "Opus", default: false }];
  await harness.send(discoverMessage(20, true));
  assertEquals((await harness.responses.next()).value, {
    jsonrpc: "2.0",
    id: 20,
    result: {
      models,
      commands: [{ name: "review", description: "Review changes" }],
    },
  });
  await harness.send(discoverMessage(21, false));
  assertEquals((await harness.responses.next()).value, {
    jsonrpc: "2.0",
    id: 21,
    result: { models },
  });
  // Discovery happens outside any session, so the directory the host resolved is the only thing
  // telling a plugin which project it is answering for. The old name never asks for commands.
  assertEquals(discoveryContexts, [
    { cwd: "/home/user/project", includeCommands: false },
    { cwd: "/home/user/project", includeCommands: true },
    { cwd: "/home/user/project", includeCommands: false },
  ]);
  assertEquals(received, [{ jsonrpc: "2.0", id: 7, method: "initialize" }]);

  await send?.({ jsonrpc: "2.0", id: 7, result: { protocolVersion: 1 } });
  assertEquals((await harness.responses.next()).value, {
    jsonrpc: "2.0",
    method: "agent/acp",
    params: { jsonrpc: "2.0", id: 7, result: { protocolVersion: 1 } },
  });

  await harness.send({
    jsonrpc: "2.0",
    id: 3,
    method: "agent/stop",
    params: {},
  });
  assertEquals((await harness.responses.next()).value, {
    jsonrpc: "2.0",
    id: 3,
    result: {},
  });
  await harness.send({ jsonrpc: "2.0", method: "ora/shutdown" });
  await run;
});

Deno.test("reports a missing agent with the code Ora retries quietly", async () => {
  const plugin = defineAgent({
    start: () => {
      throw new PluginMethodError(
        AGENT_NOT_INSTALLED,
        "claude-agent-acp is not installed",
      );
    },
    stop: () => {},
    discover: () => ({ models: [] }),
    onAcp: () => {},
  });
  const harness = createTransportHarness();
  const run = plugin.run(harness.transport);
  await harness.responses.next();

  await harness.send({
    jsonrpc: "2.0",
    id: 1,
    method: "agent/start",
    params: { cwd: "/home/user", hostVersion: "0.8.0" },
  });

  assertEquals((await harness.responses.next()).value, {
    jsonrpc: "2.0",
    id: 1,
    error: {
      code: AGENT_NOT_INSTALLED,
      message: "claude-agent-acp is not installed",
    },
  });
  await harness.send({ jsonrpc: "2.0", method: "ora/shutdown" });
  await run;
});

Deno.test("keeps unknown commands apart from an empty report", async () => {
  for (const commands of [undefined, []]) {
    const plugin = defineAgent({
      start: () => {},
      stop: () => {},
      discover: () => ({ models: [], commands }),
      onAcp: () => {},
    });
    const harness = createTransportHarness();
    const run = plugin.run(harness.transport);
    await harness.responses.next();
    await harness.send({
      jsonrpc: "2.0",
      id: 1,
      method: "agent/discover",
      params: { cwd: "/home/user/project", includeCommands: true },
    });
    assertEquals((await harness.responses.next()).value, {
      jsonrpc: "2.0",
      id: 1,
      result: commands === undefined
        ? { models: [] }
        : { models: [], commands },
    });
    await harness.send({ jsonrpc: "2.0", method: "ora/shutdown", params: {} });
    await run;
  }
});

Deno.test("gives discover the host's deadline, before which cleanup keeps its time", async () => {
  const { now } = Date;
  const { setTimeout: schedule } = globalThis;
  const asked = 1_000_000;
  // Less than a whole budget away: the request spent the rest of it on its way here.
  const stated = asked + AGENT_DISCOVERY_BUDGET_MS / 2;
  const discover = (deadline: number) => ({
    method: "agent/discover",
    params: { cwd: "/p", includeCommands: true, deadline },
  });
  const requests: {
    request: { method: string; params: JsonValue };
    expected: number;
    allowed: number;
  }[] = [
    { request: discover(stated), expected: stated, allowed: 100 },
    { request: discover(asked - 1), expected: asked, allowed: 0 },
    {
      request: { method: "agent/list_models", params: { cwd: "/p" } },
      expected: asked + AGENT_DISCOVERY_BUDGET_MS,
      allowed: 100,
    },
  ];
  for (const { request, expected, allowed } of requests) {
    let deadline: number | undefined;
    let waited: number | undefined;
    const plugin = defineAgent({
      start: () => {},
      stop: () => {},
      discover: async (context) => {
        deadline = context.deadline;
        // `session/new` answers a tenth of a second before the reserve begins.
        Date.now = () => expected - AGENT_DISCOVERY_CLEANUP_RESERVE_MS - allowed;
        globalThis.setTimeout = ((fire: () => void, delay: number) => {
          waited = delay;
          return schedule(fire, 0);
        }) as typeof setTimeout;
        await waitForAgentCommands(context, new Promise(() => {}));
        return { models: [] };
      },
      onAcp: () => {},
    });
    const harness = createTransportHarness();
    const run = plugin.run(harness.transport);
    await harness.responses.next();
    try {
      Date.now = () => asked;
      await harness.send({ jsonrpc: "2.0", id: 1, ...request });
      await harness.responses.next();
    } finally {
      Date.now = now;
      globalThis.setTimeout = schedule;
    }
    assertEquals([deadline, waited], [expected, allowed]);
    await harness.send({ jsonrpc: "2.0", method: "ora/shutdown", params: {} });
    await run;
  }
});
