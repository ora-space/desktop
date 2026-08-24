import {
  createPlugin,
  createStorage,
  HostRequestError,
  type JsonValue,
} from "../src/mod.ts";
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
  // Unbounded queuing lets a plugin await a request frame write before the harness reads it,
  // the way a real stdio pipe would.
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
  "storage calls become ora/storage/* requests correlated by id",
  async () => {
    const plugin = createPlugin();
    const storage = createStorage(plugin);
    const harness = createTransportHarness();
    const run = plugin.run(harness.transport);
    await harness.responses.next();

    const write = storage.write("state/index.json", new Uint8Array([1, 2, 3]));
    const read = storage.read("downloads/skill.zip");
    const list = storage.list("downloads");
    const remove = storage.remove("state");

    assertEquals((await harness.responses.next()).value, {
      jsonrpc: "2.0",
      id: 1,
      method: "ora/storage/write",
      params: { path: "state/index.json", bytes_base64: "AQID" },
    });
    assertEquals((await harness.responses.next()).value, {
      jsonrpc: "2.0",
      id: 2,
      method: "ora/storage/read",
      params: { path: "downloads/skill.zip" },
    });
    assertEquals((await harness.responses.next()).value, {
      jsonrpc: "2.0",
      id: 3,
      method: "ora/storage/list",
      params: { path: "downloads" },
    });
    assertEquals((await harness.responses.next()).value, {
      jsonrpc: "2.0",
      id: 4,
      method: "ora/storage/remove",
      params: { path: "state" },
    });

    // Answer out of order to prove correlation is by id, not arrival.
    await harness.send({
      jsonrpc: "2.0",
      id: 3,
      result: {
        entries: [{ name: "skill.zip", kind: "file", size_bytes: 3 }],
      },
    });
    await harness.send({
      jsonrpc: "2.0",
      id: 2,
      result: { bytes_base64: "emlw" },
    });
    await harness.send({ jsonrpc: "2.0", id: 1, result: {} });
    await harness.send({ jsonrpc: "2.0", id: 4, result: {} });

    assertEquals(await list, [{
      name: "skill.zip",
      kind: "file",
      sizeBytes: 3,
    }]);
    assertEquals([...(await read)], [122, 105, 112]);
    assertEquals(await write, undefined);
    assertEquals(await remove, undefined);

    await harness.send({ jsonrpc: "2.0", method: "ora/shutdown" });
    await run;
  },
);

Deno.test(
  "host errors carry the kind, timeouts and shutdown reject pending requests",
  async () => {
    const plugin = createPlugin();
    const storage = createStorage(plugin);
    const harness = createTransportHarness();
    const run = plugin.run(harness.transport);
    await harness.responses.next();

    const escaped = storage.read("../outside").catch((error) => error);
    const timedOut = plugin
      .request("ora/storage/list", { path: "" }, { timeoutMs: 5 })
      .catch((error) => error);
    const orphaned = storage.list("downloads").catch((error) => error);
    await harness.responses.next();
    await harness.responses.next();
    await harness.responses.next();
    await harness.send({
      jsonrpc: "2.0",
      id: 1,
      error: {
        code: -32602,
        message: "path escapes the plugin data directory",
        data: { kind: "invalid_path" },
      },
    });
    const escapedError = await escaped;
    const timedOutError = await timedOut;
    await harness.send({ jsonrpc: "2.0", method: "ora/shutdown" });
    await run;
    const orphanedError = await orphaned;

    assertEquals(
      [escapedError, timedOutError, orphanedError].map((error) => [
        error instanceof HostRequestError,
        error.kind,
        error.code,
      ]),
      [
        [true, "invalid_path", -32602],
        [true, "timeout", undefined],
        [true, "transport", undefined],
      ],
    );
  },
);
