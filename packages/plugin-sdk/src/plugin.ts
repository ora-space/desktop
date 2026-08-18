import {
  createDenoTransport,
  decodeFrames,
  encodeFrame,
  type JsonRpcNotification,
  type JsonRpcRequest,
  type JsonValue,
  type PluginTransport,
  type RequestId,
} from "./protocol.ts";

export type MethodHandler = (
  input: JsonValue,
) => JsonValue | Promise<JsonValue>;

type PluginState = "registering" | "running" | "stopped";

/** Stores a plugin's immutable method registry and serves host requests. */
export class Plugin {
  readonly #methods = new Map<string, MethodHandler>();
  #state: PluginState = "registering";

  /** Registers one uniquely named method before the plugin starts serving. */
  registerMethod(name: string, handler: MethodHandler): void {
    if (this.#state !== "registering") {
      throw new Error("Plugin methods cannot be registered after run() starts");
    }
    if (name.length === 0) {
      throw new Error("Plugin method names cannot be empty");
    }
    if (this.#methods.has(name)) {
      throw new Error(`Plugin method ${name} is already registered`);
    }
    this.#methods.set(name, handler);
  }

  /** Announces the method registry and serves requests until shutdown or EOF. */
  async run(transport: PluginTransport = createDenoTransport()): Promise<void> {
    if (this.#state !== "registering") {
      throw new Error("A plugin can only run once");
    }
    this.#state = "running";
    if (transport.redirectConsole) {
      redirectConsoleToStderr();
    }

    const writer = new FrameWriter(transport.writable);
    await writer.write({
      jsonrpc: "2.0",
      method: "ora/register",
      params: { methods: [...this.#methods.keys()] },
    });

    const inFlight = new Set<Promise<void>>();
    try {
      for await (const message of decodeFrames(transport.readable)) {
        if (isShutdownNotification(message)) {
          break;
        }
        const request = parseRequest(message);
        const operation = this.#dispatch(request, writer);
        inFlight.add(operation);
        // Supplying both continuations observes transport failures without creating a rejected
        // promise from `finally`; the host will invalidate the process when stdout closes.
        void operation.then(
          () => inFlight.delete(operation),
          () => inFlight.delete(operation),
        );
      }
      await Promise.allSettled(inFlight);
    } finally {
      this.#state = "stopped";
      await writer.close();
    }
  }

  /** Executes one handler and maps expected method failures into JSON-RPC responses. */
  async #dispatch(request: JsonRpcRequest, writer: FrameWriter): Promise<void> {
    const handler = this.#methods.get(request.method);
    if (handler === undefined) {
      await writer.write(
        errorResponse(
          request.id,
          -32601,
          `Unknown plugin method ${request.method}`,
        ),
      );
      return;
    }

    try {
      const result = await handler(request.params ?? null);
      await writer.write({
        jsonrpc: "2.0",
        id: request.id,
        result: result ?? null,
      });
    } catch (error) {
      await writer.write(
        errorResponse(
          request.id,
          -32603,
          error instanceof Error ? error.message : "Plugin method failed",
        ),
      );
    }
  }
}

/** Creates a fresh plugin in its registration state. */
export function createPlugin(): Plugin {
  return new Plugin();
}

class FrameWriter {
  readonly #writer: WritableStreamDefaultWriter<Uint8Array>;
  #tail: Promise<void> = Promise.resolve();

  constructor(writable: WritableStream<Uint8Array>) {
    this.#writer = writable.getWriter();
  }

  /** Queues one whole frame after all earlier writes have completed. */
  write(message: JsonValue): Promise<void> {
    const operation = this.#tail.then(() =>
      this.#writer.write(encodeFrame(message))
    );
    // Keeping a fulfilled tail lets later writes proceed while each caller still observes its
    // own failure through the returned operation.
    this.#tail = operation.catch(() => undefined);
    return operation;
  }

  /** Flushes queued frames and releases the underlying stdout writer. */
  async close(): Promise<void> {
    await this.#tail;
    this.#writer.releaseLock();
  }
}

/** Validates the host request shape before any plugin handler sees it. */
function parseRequest(message: unknown): JsonRpcRequest {
  if (!isRecord(message) || message.jsonrpc !== "2.0") {
    throw new Error("Host message is not JSON-RPC 2.0");
  }
  if (
    (typeof message.id !== "number" && typeof message.id !== "string") ||
    typeof message.method !== "string"
  ) {
    throw new Error("Host request has an invalid id or method");
  }
  return message as unknown as JsonRpcRequest;
}

/** Recognizes the only lifecycle notification accepted after registration. */
function isShutdownNotification(
  message: unknown,
): message is JsonRpcNotification {
  return (
    isRecord(message) &&
    message.jsonrpc === "2.0" &&
    message.method === "ora/shutdown" &&
    !("id" in message)
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function errorResponse(
  id: RequestId,
  code: number,
  message: string,
): JsonValue {
  return { jsonrpc: "2.0", id, error: { code, message } };
}

let consoleRedirected = false;

/** Protects the stdout protocol channel from every standard console method. */
function redirectConsoleToStderr(): void {
  if (consoleRedirected) {
    return;
  }
  consoleRedirected = true;
  const encoder = new TextEncoder();
  const write = (level: string, values: unknown[]) => {
    const rendered = values
      .map((value) => (typeof value === "string" ? value : Deno.inspect(value)))
      .join(" ");
    Deno.stderr.writeSync(encoder.encode(`[plugin:${level}] ${rendered}\n`));
  };
  console.debug = (...values: unknown[]) => write("debug", values);
  console.info = (...values: unknown[]) => write("info", values);
  console.log = (...values: unknown[]) => write("log", values);
  console.warn = (...values: unknown[]) => write("warn", values);
  console.error = (...values: unknown[]) => write("error", values);
}
