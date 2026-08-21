import {
  defineUiPlugin,
  type JsonValue,
  type SurfaceSession,
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

const SESSION_PARAMS = {
  surface_id: "market",
  surface_instance_id: 7,
  plugin_generation: 3,
};

Deno.test(
  "registers the ui contract and maps snake_case params to camelCase",
  async () => {
    const events: JsonValue[] = [];
    let lastSession: SurfaceSession | undefined;
    const ui = defineUiPlugin({
      onSurfaceOpened: (session) => {
        lastSession = session;
        events.push(["opened", session as unknown as JsonValue]);
      },
      onSurfaceClosed: (session) => {
        events.push(["closed", session as unknown as JsonValue]);
      },
      onDownloadCompleted: (event) => {
        events.push(["download", event as unknown as JsonValue]);
      },
      onRequest: ({ session, payload }) => ({
        echo: payload,
        instance: session.surfaceInstanceId,
      }),
    });
    const harness = createTransportHarness();
    const run = ui.run(harness.transport);

    assertEquals((await harness.responses.next()).value, {
      jsonrpc: "2.0",
      method: "ora/register",
      params: {
        methods: ["ora/ui/download_completed", "ora/ui/request"],
        emits: ["ora/ui/push"],
      },
    });

    await harness.send({
      jsonrpc: "2.0",
      method: "ora/ui/surface_opened",
      params: SESSION_PARAMS,
    });
    await harness.send({
      jsonrpc: "2.0",
      id: 1,
      method: "ora/ui/download_completed",
      params: {
        ...SESSION_PARAMS,
        download: {
          id: 12,
          page_url: "https://www.skillhub.cn/skills/abc",
          source_url: "https://cdn.skillhub.cn/abc.zip",
          file_name: "abc.zip",
          path: "downloads/abc.zip",
          size_bytes: 10240,
          completed_at: "2026-08-20T16:30:00+08:00",
        },
      },
    });
    assertEquals((await harness.responses.next()).value, {
      jsonrpc: "2.0",
      id: 1,
      result: {},
    });
    await harness.send({
      jsonrpc: "2.0",
      id: 2,
      method: "ora/ui/request",
      params: { ...SESSION_PARAMS, payload: { type: "increment" } },
    });
    assertEquals((await harness.responses.next()).value, {
      jsonrpc: "2.0",
      id: 2,
      result: { payload: { echo: { type: "increment" }, instance: 7 } },
    });

    await ui.push(lastSession!, { count: 1 });
    assertEquals((await harness.responses.next()).value, {
      jsonrpc: "2.0",
      method: "ora/ui/push",
      params: { ...SESSION_PARAMS, payload: { count: 1 } },
    });
    await harness.send({
      jsonrpc: "2.0",
      method: "ora/ui/surface_closed",
      params: SESSION_PARAMS,
    });
    await harness.send({ jsonrpc: "2.0", method: "ora/shutdown" });
    await run;

    const session = {
      surfaceId: "market",
      surfaceInstanceId: 7,
      pluginGeneration: 3,
    };
    assertEquals(events, [
      ["opened", session],
      [
        "download",
        {
          session,
          download: {
            id: 12,
            pageUrl: "https://www.skillhub.cn/skills/abc",
            sourceUrl: "https://cdn.skillhub.cn/abc.zip",
            fileName: "abc.zip",
            path: "downloads/abc.zip",
            sizeBytes: 10240,
            completedAt: "2026-08-20T16:30:00+08:00",
          },
        },
      ],
      ["closed", session],
    ]);
  },
);

Deno.test(
  "registers only the methods it can serve and rejects malformed params",
  async () => {
    const ui = defineUiPlugin({
      onRequest: () => null,
    });
    const harness = createTransportHarness();
    const run = ui.run(harness.transport);

    assertEquals((await harness.responses.next()).value, {
      jsonrpc: "2.0",
      method: "ora/register",
      params: { methods: ["ora/ui/request"], emits: ["ora/ui/push"] },
    });
    await harness.send({
      jsonrpc: "2.0",
      id: 1,
      method: "ora/ui/request",
      params: { surface_id: "counter", payload: 1 },
    });
    assertEquals((await harness.responses.next()).value, {
      jsonrpc: "2.0",
      id: 1,
      error: {
        code: -32602,
        message:
          "ora/ui/request requires surface_id, surface_instance_id, and plugin_generation",
      },
    });
    await harness.send({ jsonrpc: "2.0", method: "ora/shutdown" });
    await run;
  },
);
