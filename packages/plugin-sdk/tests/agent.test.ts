import {
  type AcpSender,
  AGENT_NOT_INSTALLED,
  defineAgent,
  PluginMethodError,
} from "../src/mod.ts";
import {
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

Deno.test("serves the whole agent contract over one run loop", async () => {
  const received: JsonValue[] = [];
  const effectCalls: unknown[] = [];
  const discoveryContexts: unknown[] = [];
  let send: AcpSender | undefined;
  const plugin = defineAgent({
    start: (_context, sender) => {
      send = sender;
    },
    stop: () => {},
    listModels: (context) => {
      discoveryContexts.push(context);
      return [{ id: "opus", displayName: "Opus" }];
    },
    onAcp: (frame) => {
      received.push(frame);
    },
    effects: {
      resources: [{
        workspaceRelativePath: ".agents/skills",
        materializationFormat: "ora/skill-directory.v1",
        coordination: "quiesce_before_mutation",
      }],
      coordinate: (context) => {
        effectCalls.push(context);
        return { barrier: "held" };
      },
      reactivate: (context) => {
        effectCalls.push(context);
        return { restarted: true };
      },
      verifyReady: (context) => {
        effectCalls.push(context);
        return { loaded: true };
      },
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
        "agent/list_models",
        "effect/coordinate",
        "effect/reactivate",
        "effect/verify_ready",
      ],
      emits: ["agent/acp"],
      effectResources: [{
        workspaceRelativePath: ".agents/skills",
        materializationFormat: "ora/skill-directory.v1",
        coordination: "quiesce_before_mutation",
      }],
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
  // Discovery happens outside any session, so the directory the host resolved is the only thing
  // telling a plugin which project it is answering for.
  assertEquals(discoveryContexts, [{ cwd: "/home/user/project" }]);
  assertEquals(received, [{ jsonrpc: "2.0", id: 7, method: "initialize" }]);

  const coordination = {
    targetId: "target-1",
    resourceIds: ["resource-1"],
  };
  await harness.send({
    jsonrpc: "2.0",
    id: 4,
    method: "effect/coordinate",
    params: coordination,
  });
  assertEquals((await harness.responses.next()).value, {
    jsonrpc: "2.0",
    id: 4,
    result: { barrier: "held" },
  });
  await harness.send({
    jsonrpc: "2.0",
    id: 5,
    method: "effect/reactivate",
    params: coordination,
  });
  assertEquals((await harness.responses.next()).value, {
    jsonrpc: "2.0",
    id: 5,
    result: { restarted: true },
  });
  const readiness = {
    targetId: "target-1",
    generation: 7,
    consumerRevisionId: "consumer-revision-1",
    projectionDigest: "sha256:projection",
  };
  await harness.send({
    jsonrpc: "2.0",
    id: 6,
    method: "effect/verify_ready",
    params: readiness,
  });
  assertEquals((await harness.responses.next()).value, {
    jsonrpc: "2.0",
    id: 6,
    result: { loaded: true },
  });
  assertEquals(effectCalls, [coordination, coordination, readiness]);

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
