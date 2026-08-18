import { type AcpSender, defineAgent } from "../src/mod.ts";
import {
  decodeFrames,
  encodeFrame,
  type JsonValue,
  type PluginTransport,
} from "../src/protocol.ts";

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

Deno.test("step1: register only then shutdown", async () => {
  const plugin = defineAgent({
    start: () => {},
    stop: () => {},
    listModels: () => [],
    onAcp: () => {},
  });
  const harness = createTransportHarness();
  const run = plugin.run(harness.transport);
  await harness.responses.next();
  await harness.send({ jsonrpc: "2.0", method: "ora/shutdown" });
  await run;
});

Deno.test("step2: start then shutdown", async () => {
  const plugin = defineAgent({
    start: () => {},
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
  await harness.responses.next();

  await harness.send({ jsonrpc: "2.0", method: "ora/shutdown" });
  await run;
});

Deno.test("step3: start + acp notification forward then shutdown", async () => {
  const received: JsonValue[] = [];
  const plugin = defineAgent({
    start: () => {},
    stop: () => {},
    listModels: () => [],
    onAcp: (frame) => {
      received.push(frame);
    },
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
  await harness.responses.next();

  await harness.send({
    jsonrpc: "2.0",
    method: "agent/acp",
    params: { jsonrpc: "2.0", id: 7, method: "initialize" },
  });

  await harness.send({ jsonrpc: "2.0", method: "ora/shutdown" });
  await run;
});

Deno.test("step4: direct notify via captured sender then shutdown", async () => {
  let send: AcpSender | undefined;
  const plugin = defineAgent({
    start: (_context, sender) => {
      send = sender;
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
  await harness.responses.next();

  await send?.({ jsonrpc: "2.0", id: 7, result: { protocolVersion: 1 } });
  await harness.responses.next();

  await harness.send({ jsonrpc: "2.0", method: "ora/shutdown" });
  await run;
});
