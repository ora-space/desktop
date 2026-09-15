import { createLogger, createPlugin, type PluginLogSink } from "../src/mod.ts";
import {
  decodeFrames,
  encodeFrame,
  PLUGIN_LOG_ENVELOPE_V1_PREFIX,
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

/** Captures encoded lines so a test can inspect exactly what would reach stderr. */
function captureSink(): { sink: PluginLogSink; lines: string[] } {
  const lines: string[] = [];
  return { sink: { write: (line) => lines.push(line) }, lines };
}

/** Strips the envelope prefix and newline of one captured line and parses the payload. */
function payloadOf(line: string): Record<string, unknown> {
  if (!line.startsWith(PLUGIN_LOG_ENVELOPE_V1_PREFIX) || !line.endsWith("\n")) {
    throw new Error(`Not one v1 envelope line: ${JSON.stringify(line)}`);
  }
  const body = line.slice(PLUGIN_LOG_ENVELOPE_V1_PREFIX.length, -1);
  if (body.includes("\n")) {
    throw new Error("Envelope body must not contain a raw newline");
  }
  return JSON.parse(body);
}

Deno.test("each level writes one v1 envelope line with its fields", () => {
  const { sink, lines } = captureSink();
  const logger = createLogger(sink);
  logger.trace("t");
  logger.debug("d", { target: "db" });
  logger.info("multi\nline", { method: "query", context: { ms: 12 } });
  logger.warn("w");
  logger.error("e", { error: new Error("boom") });

  const payloads = lines.map(payloadOf);
  assertEquals(payloads.map((payload) => payload.level), [
    "TRACE",
    "DEBUG",
    "INFO",
    "WARN",
    "ERROR",
  ]);
  assertEquals(payloads[1], { level: "DEBUG", message: "d", target: "db" });
  assertEquals(payloads[2], {
    level: "INFO",
    message: "multi\nline",
    method: "query",
    context: { ms: 12 },
  });
  const error = payloads[4].error as Record<string, unknown>;
  assertEquals([error.name, error.message, typeof error.stack], [
    "Error",
    "boom",
    "string",
  ]);
});

Deno.test("child loggers layer defaults without touching the parent", () => {
  const { sink, lines } = captureSink();
  const parent = createLogger(sink, { context: { app: "x" } });
  const child = parent.child({ target: "sync", context: { job: 1 } });
  child.info("from child", { context: { step: 2 } });
  parent.info("from parent");

  assertEquals(lines.map(payloadOf), [
    {
      level: "INFO",
      message: "from child",
      target: "sync",
      context: { app: "x", job: 1, step: 2 },
    },
    { level: "INFO", message: "from parent", context: { app: "x" } },
  ]);
});

Deno.test("hostile values never throw and degrade to bounded descriptions", () => {
  const { sink, lines } = captureSink();
  const logger = createLogger(sink);
  const circular: Record<string, unknown> = { name: "loop" };
  circular.self = circular;
  const throwingGetter = Object.defineProperty({}, "boom", {
    enumerable: true,
    get() {
      throw new Error("getter failed");
    },
  });
  const throwingToJson = {
    toJSON() {
      throw new Error("toJSON failed");
    },
  };

  logger.info("values", {
    context: {
      circular,
      big: 10n,
      fn: () => 1,
      getter: throwingGetter,
      toJson: throwingToJson,
      nested: { deep: { deeper: { deepest: true } } },
    },
    error: "not an error object",
  });
  logger.error("cause chain", {
    error: new Error("outer", { cause: new Error("inner") }),
  });
  // Ten 10k-character values stay individually under the string bound but together push the
  // record past the host's limit, which is what forces the context to be dropped.
  logger.info("x".repeat(20_000), {
    context: Object.fromEntries(
      Array.from(
        { length: 10 },
        (_, index) => [`k${index}`, "y".repeat(10_000)],
      ),
    ),
  });

  const [values, chain, oversized] = lines.map(payloadOf);
  const context = values.context as Record<string, unknown>;
  assertEquals(
    [
      (context.circular as Record<string, unknown>).self,
      context.big,
      typeof context.fn,
      (context.getter as Record<string, unknown>).boom,
      // `toJSON` is never invoked; the method is rendered like any other own property.
      typeof (context.toJson as Record<string, unknown>).toJSON,
      values.error,
    ],
    [
      "[circular]",
      "10n",
      "string",
      "[unserializable: Error: getter failed]",
      "string",
      { name: "NonError", message: "not an error object" },
    ],
  );
  assertEquals(
    ((chain.error as Record<string, unknown>).cause as Record<string, unknown>)
      .message,
    "inner",
  );
  const message = oversized.message as string;
  assertEquals(
    [
      message.length < 20_000,
      message.endsWith("…[truncated]"),
      oversized.context,
    ],
    [true, true, { truncated: true }],
  );
  assertEquals(
    lines.every((line) =>
      new TextEncoder().encode(line).byteLength <= 64 * 1024
    ),
    true,
  );
});

Deno.test("a failing sink is absorbed rather than thrown into plugin code", () => {
  const logger = createLogger({
    write() {
      throw new Error("EPIPE");
    },
  });
  logger.error("still fine");
});

/** Creates paired in-memory streams and opts into the console redirect. */
function createTransportHarness(): {
  transport: PluginTransport;
  send: (message: unknown) => Promise<void>;
  responses: AsyncGenerator<unknown>;
} {
  const hostInput = new TransformStream<Uint8Array>();
  const pluginOutput = new TransformStream<Uint8Array>();
  const inputWriter = hostInput.writable.getWriter();
  return {
    transport: {
      readable: hostInput.readable,
      writable: pluginOutput.writable,
      redirectConsole: true,
    },
    send: (message) =>
      inputWriter.write(
        encodeFrame(message as Parameters<typeof encodeFrame>[0]),
      ),
    responses: decodeFrames(pluginOutput.readable),
  };
}

Deno.test(
  "console.* maps onto the plugin logger and stdout carries only protocol frames",
  async () => {
    const { sink, lines } = captureSink();
    const plugin = createPlugin({ logSink: sink });
    plugin.registerMethod("example.noisy", (input) => {
      console.debug("d");
      console.info("i");
      console.log("l", { n: 1 });
      console.warn("w\nsecond line");
      console.error("e");
      plugin.logger.info("direct");
      return input;
    });
    const harness = createTransportHarness();
    const run = plugin.run(harness.transport);
    await harness.responses.next();
    await harness.send({
      jsonrpc: "2.0",
      id: 1,
      method: "example.noisy",
      params: "ok",
    });
    const response = (await harness.responses.next()).value;
    await harness.send({ jsonrpc: "2.0", method: "ora/shutdown" });
    await run;

    assertEquals(response, { jsonrpc: "2.0", id: 1, result: "ok" });
    assertEquals(
      lines.map(payloadOf).map((payload) => [payload.level, payload.message]),
      [
        ["DEBUG", "d"],
        ["INFO", "i"],
        ["INFO", "l { n: 1 }"],
        ["WARN", "w\nsecond line"],
        ["ERROR", "e"],
        ["INFO", "direct"],
      ],
    );
  },
);
