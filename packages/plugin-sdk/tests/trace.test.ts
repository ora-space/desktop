import {
  createPlugin,
  createTraceClient,
  type JsonValue,
  MAX_TRACE_CHUNK_BYTES,
} from "../src/mod.ts";
import {
  decodeFrames,
  encodeFrame,
  type PluginTransport,
} from "../src/protocol/index.ts";

function assertEquals(actual: unknown, expected: unknown): void {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(
      `Expected ${JSON.stringify(expected)}, received ${
        JSON.stringify(actual)
      }`,
    );
  }
}

function harness(): {
  transport: PluginTransport;
  send(message: JsonValue): Promise<void>;
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

Deno.test("trace client binds every request to its invocation context", async () => {
  const plugin = createPlugin();
  plugin.registerMethod("ready", () => null);
  const trace = createTraceClient(plugin, { id: "ctx-1" });
  const wire = harness();
  const run = plugin.run(wire.transport);
  await wire.responses.next();

  const listed = trace.list();
  const listRequest = (await wire.responses.next()).value as { id: number };
  assertEquals(listRequest, {
    jsonrpc: "2.0",
    id: 1,
    method: "ora/trace/list",
    params: { context_id: "ctx-1" },
  });
  await wire.send({
    jsonrpc: "2.0",
    id: listRequest.id,
    result: {
      traces: [{
        trace_id: "trace-1",
        provider_id: "claude-code",
        format: "ora/trace.claude-code-jsonl.v1",
        size_bytes: 3,
        modified_at_ms: 5,
        cursor: "5:3",
        label: "Dashboard test",
        is_current: true,
      }],
    },
  });
  assertEquals(await listed, [{
    traceId: "trace-1",
    providerId: "claude-code",
    format: "ora/trace.claude-code-jsonl.v1",
    sizeBytes: 3,
    modifiedAtMs: 5,
    cursor: "5:3",
    label: "Dashboard test",
    isCurrent: true,
  }]);

  const reading = trace.read("trace-1", 1, 2, "5:3");
  const readRequest = (await wire.responses.next()).value as { id: number };
  assertEquals(readRequest, {
    jsonrpc: "2.0",
    id: 2,
    method: "ora/trace/read",
    params: {
      context_id: "ctx-1",
      trace_id: "trace-1",
      offset: 1,
      max_bytes: 2,
      cursor: "5:3",
    },
  });
  await wire.send({
    jsonrpc: "2.0",
    id: readRequest.id,
    result: {
      bytes_base64: "AgM=",
      offset: 1,
      next_offset: 3,
      eof: true,
      cursor: "5:3",
    },
  });
  const chunk = await reading;
  assertEquals({ ...chunk, bytes: [...chunk.bytes] }, {
    bytes: [2, 3],
    offset: 1,
    nextOffset: 3,
    eof: true,
    cursor: "5:3",
  });

  await wire.send({ jsonrpc: "2.0", method: "ora/shutdown" });
  await run;
});

Deno.test("trace client rejects oversized reads before host I/O", () => {
  const trace = createTraceClient(createPlugin(), { id: "ctx-1" });
  const request = trace.read("trace-1", 0, MAX_TRACE_CHUNK_BYTES + 1);
  request.catch(() => undefined);
  return request.then(
    () => {
      throw new Error("expected oversized trace read to fail");
    },
    (error) => {
      if (!(error instanceof Error) || !error.message.includes("maxBytes")) {
        throw error;
      }
    },
  );
});
