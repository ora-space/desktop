import {
  createDenoTransport,
  decodeFrames,
  encodeFrame,
  INTERNAL_ERROR,
  JSON_RPC_VERSION,
  type JsonRpcNotification,
  type JsonRpcRequest,
  type JsonValue,
  METHOD_NOT_FOUND,
  PLUGIN_METHODS,
  type PluginEffectResource,
  type PluginRegistrationParams,
  type PluginTransport,
  type RequestId,
  SKILL_DIRECTORY_V1,
  OPENCODE_MCP_CONFIG_V1,
  CLAUDE_MCP_CONFIG_V1,
} from "./protocol/index.ts";

export type MethodHandler = (
  input: JsonValue,
) => JsonValue | Promise<JsonValue>;

export type NotificationHandler = (
  params: JsonValue,
) => void | Promise<void>;

type PluginState = "registering" | "running" | "stopped";

/** How long a host request may stay unanswered before it fails with `timeout`. */
export const DEFAULT_HOST_REQUEST_TIMEOUT_MS = 30_000;

/** Options of one plugin-to-host request. */
export interface HostRequestOptions {
  timeoutMs?: number;
}

/**
 * The one materialization format Ora's agent Consumer adapter accepts.
 *
 * Exported as a value because the host compares it exactly and abandons the agent for the rest of
 * the process when it does not match — a typo here is not a degraded mode, it is a plugin that
 * never starts, reported once as an invalid Effect declaration.
 */
export { CLAUDE_MCP_CONFIG_V1, OPENCODE_MCP_CONFIG_V1, SKILL_DIRECTORY_V1 };

export type EffectResourceDeclaration =
  & Omit<
    PluginEffectResource,
    "materializationFormat"
  >
  & {
    /** Narrowed to the built-in formats the host accepts. */
    materializationFormat:
      | typeof SKILL_DIRECTORY_V1
      | typeof OPENCODE_MCP_CONFIG_V1
      | typeof CLAUDE_MCP_CONFIG_V1;
  };

/**
 * A host method failed, or could not be completed.
 *
 * `kind` is the stable classification to branch on: it is the host's `data.kind` when the host
 * answered with one (storage reports `invalid_path`, `not_found`, `too_large`, `io`,
 * `invalid_params`), `method_not_found` for a method this host does not serve, `timeout` when no
 * answer arrived in time, and `transport` when the connection ended first. `code` is the raw
 * JSON-RPC code for the first two.
 */
export class HostRequestError extends Error {
  readonly kind: string;
  readonly code: number | undefined;
  readonly data: JsonValue;

  constructor(
    kind: string,
    message: string,
    code?: number,
    data: JsonValue = null,
  ) {
    super(message);
    this.name = "HostRequestError";
    this.kind = kind;
    this.code = code;
    this.data = data;
  }
}

interface PendingHostRequest {
  resolve(result: JsonValue): void;
  reject(error: HostRequestError): void;
  timer: ReturnType<typeof setTimeout>;
}

/** Stores a plugin's immutable capability registry and serves host traffic. */
export class Plugin {
  readonly #methods = new Map<string, MethodHandler>();
  readonly #emits = new Set<string>();
  readonly #effectResources: EffectResourceDeclaration[] = [];
  readonly #notificationHandlers = new Map<string, NotificationHandler>();
  readonly #pendingHostRequests = new Map<number, PendingHostRequest>();
  #nextHostRequestId = 1;
  #state: PluginState = "registering";
  #writer: FrameWriter | undefined;

  /** Registers one uniquely named method before the plugin starts serving. */
  registerMethod(name: string, handler: MethodHandler): void {
    this.#assertRegistering();
    if (name.length === 0) {
      throw new Error("Plugin method names cannot be empty");
    }
    if (this.#methods.has(name)) {
      throw new Error(`Plugin method ${name} is already registered`);
    }
    this.#methods.set(name, handler);
  }

  /**
   * Declares one method this plugin may send to the host unprompted.
   *
   * The declaration is part of the same immutable registration as `registerMethod`, so the host
   * knows the plugin's whole behaviour before it serves anything.
   */
  declareEmit(name: string): void {
    this.#assertRegistering();
    if (name.length === 0) {
      throw new Error("Emitted method names cannot be empty");
    }
    this.#emits.add(name);
  }

  /** Declares one runtime-consumed Effect Resource before registration is sent. */
  declareEffectResource(resource: EffectResourceDeclaration): void {
    this.#assertRegistering();
    if (
      resource.workspaceRelativePath.length === 0 ||
      resource.materializationFormat.length === 0
    ) {
      throw new Error("Effect Resource locator and format cannot be empty");
    }
    this.#effectResources.push({ ...resource });
  }

  /** Handles one host-sent notification, which never produces a response. */
  onNotification(name: string, handler: NotificationHandler): void {
    this.#assertRegistering();
    if (this.#notificationHandlers.has(name)) {
      throw new Error(`Notification ${name} already has a handler`);
    }
    this.#notificationHandlers.set(name, handler);
  }

  /**
   * Sends one declared notification to the host while the plugin is running.
   *
   * Only methods declared through `declareEmit` may be sent: the host rejects anything outside
   * that whitelist and terminates the process, so an undeclared method is a defect here rather
   * than a message the host quietly drops.
   */
  async notify(method: string, params: JsonValue): Promise<void> {
    if (!this.#emits.has(method)) {
      throw new Error(`Plugin method ${method} was not declared in emits`);
    }
    if (this.#writer === undefined) {
      throw new Error("A plugin can only notify the host while running");
    }
    await this.#writer.write({ jsonrpc: JSON_RPC_VERSION, method, params });
  }

  /**
   * Sends one request to the host and resolves with its `result`.
   *
   * Host methods (such as `ora/storage/*`) need no declaration: the host decides what it
   * serves and answers `method_not_found` otherwise. Requests are correlated by a plugin-local
   * numeric id and bounded by `timeoutMs`; a process shutdown rejects everything still pending,
   * so a caller never waits on a connection that is gone.
   */
  request(
    method: string,
    params: JsonValue,
    options: HostRequestOptions = {},
  ): Promise<JsonValue> {
    const writer = this.#writer;
    if (writer === undefined) {
      return Promise.reject(
        new HostRequestError(
          "transport",
          "A plugin can only call the host while running",
        ),
      );
    }
    const id = this.#nextHostRequestId;
    this.#nextHostRequestId += 1;
    const timeoutMs = options.timeoutMs ?? DEFAULT_HOST_REQUEST_TIMEOUT_MS;
    return new Promise<JsonValue>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.#pendingHostRequests.delete(id);
        reject(
          new HostRequestError(
            "timeout",
            `Host request ${method} timed out after ${timeoutMs} ms`,
          ),
        );
      }, timeoutMs);
      this.#pendingHostRequests.set(id, { resolve, reject, timer });
      writer.write({ jsonrpc: JSON_RPC_VERSION, id, method, params }).catch(
        (error) => {
          const pending = this.#pendingHostRequests.get(id);
          if (pending === undefined) {
            return;
          }
          this.#pendingHostRequests.delete(id);
          clearTimeout(pending.timer);
          reject(
            new HostRequestError(
              "transport",
              error instanceof Error
                ? error.message
                : "Host request write failed",
            ),
          );
        },
      );
    });
  }

  /** Announces the capability registry and serves host traffic until shutdown or EOF. */
  async run(transport: PluginTransport = createDenoTransport()): Promise<void> {
    if (this.#state !== "registering") {
      throw new Error("A plugin can only run once");
    }
    this.#state = "running";
    if (transport.redirectConsole) {
      redirectConsoleToStderr();
    }

    const writer = new FrameWriter(transport.writable);
    this.#writer = writer;
    const registration = {
      methods: [...this.#methods.keys()],
      emits: [...this.#emits],
      ...(this.#effectResources.length === 0
        ? {}
        : { effectResources: this.#effectResources }),
    } satisfies PluginRegistrationParams;
    await writer.write({
      jsonrpc: JSON_RPC_VERSION,
      method: PLUGIN_METHODS.register,
      params: registration,
    });

    const inFlight = new Set<Promise<void>>();
    const track = (operation: Promise<void>) => {
      inFlight.add(operation);
      // Supplying both continuations observes transport failures without creating a rejected
      // promise from `finally`; the host will invalidate the process when stdout closes.
      void operation.then(
        () => inFlight.delete(operation),
        () => inFlight.delete(operation),
      );
    };
    try {
      for await (const message of decodeFrames(transport.readable)) {
        if (isShutdownNotification(message)) {
          break;
        }
        if (this.#settleHostResponse(message)) {
          continue;
        }
        const notification = this.#matchNotification(message);
        if (notification !== undefined) {
          track(notification);
          continue;
        }
        track(this.#dispatch(parseRequest(message), writer));
      }
      await Promise.allSettled(inFlight);
    } finally {
      this.#state = "stopped";
      this.#writer = undefined;
      this.#rejectPendingHostRequests();
      await writer.close();
    }
  }

  /** Routes a host response to its pending request; reports whether the message was one. */
  #settleHostResponse(message: unknown): boolean {
    if (
      !isRecord(message) || message.jsonrpc !== JSON_RPC_VERSION ||
      "method" in message ||
      typeof message.id !== "number"
    ) {
      return false;
    }
    const pending = this.#pendingHostRequests.get(message.id);
    if (pending === undefined) {
      // A response to a timed-out request is late, not hostile; the host already answered the
      // id it was given, so there is nothing left to correlate.
      return true;
    }
    this.#pendingHostRequests.delete(message.id);
    clearTimeout(pending.timer);
    if ("error" in message) {
      const error = isRecord(message.error) ? message.error : {};
      const data = (error.data ?? null) as JsonValue;
      const kind = isRecord(data) && typeof data.kind === "string"
        ? data.kind
        : error.code === METHOD_NOT_FOUND
        ? "method_not_found"
        : "host";
      pending.reject(
        new HostRequestError(
          kind,
          typeof error.message === "string"
            ? error.message
            : "Host request failed",
          typeof error.code === "number" ? error.code : undefined,
          data,
        ),
      );
    } else {
      pending.resolve((message.result ?? null) as JsonValue);
    }
    return true;
  }

  /** Fails every in-flight host request once the connection can no longer answer. */
  #rejectPendingHostRequests(): void {
    for (const [id, pending] of this.#pendingHostRequests) {
      this.#pendingHostRequests.delete(id);
      clearTimeout(pending.timer);
      pending.reject(
        new HostRequestError(
          "transport",
          "Plugin stopped before the host answered",
        ),
      );
    }
  }

  /** Runs the handler for a host notification, or reports that this was not a notification. */
  #matchNotification(message: unknown): Promise<void> | undefined {
    if (
      !isRecord(message) || message.jsonrpc !== JSON_RPC_VERSION ||
      "id" in message
    ) {
      return undefined;
    }
    if (typeof message.method !== "string") {
      return undefined;
    }
    const handler = this.#notificationHandlers.get(message.method);
    if (handler === undefined) {
      // Notifications have no response channel, so an unhandled one can only be reported. Failing
      // the process here would let a host that learned a new method take working plugins down.
      console.warn(`Ignoring unhandled host notification ${message.method}`);
      return Promise.resolve();
    }
    return Promise.resolve(handler((message.params ?? null) as JsonValue));
  }

  /** Executes one handler and maps expected method failures into JSON-RPC responses. */
  async #dispatch(request: JsonRpcRequest, writer: FrameWriter): Promise<void> {
    const handler = this.#methods.get(request.method);
    if (handler === undefined) {
      await writer.write(
        errorResponse(
          request.id,
          METHOD_NOT_FOUND,
          `Unknown plugin method ${request.method}`,
        ),
      );
      return;
    }

    try {
      const result = await handler(request.params ?? null);
      await writer.write({
        jsonrpc: JSON_RPC_VERSION,
        id: request.id,
        result: result ?? null,
      });
    } catch (error) {
      await writer.write(
        errorResponse(
          request.id,
          error instanceof PluginMethodError ? error.code : INTERNAL_ERROR,
          error instanceof Error ? error.message : "Plugin method failed",
        ),
      );
    }
  }

  #assertRegistering(): void {
    if (this.#state !== "registering") {
      throw new Error("Plugin capabilities cannot change after run() starts");
    }
  }
}

/**
 * Carries a specific JSON-RPC error code from a method handler to the host.
 *
 * Ora distinguishes expected conditions from faults by code, so a handler that throws this instead
 * of a plain `Error` controls how the host reacts.
 */
export class PluginMethodError extends Error {
  readonly code: number;

  constructor(code: number, message: string) {
    super(message);
    this.name = "PluginMethodError";
    this.code = code;
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
  if (!isRecord(message) || message.jsonrpc !== JSON_RPC_VERSION) {
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
    message.jsonrpc === JSON_RPC_VERSION &&
    message.method === PLUGIN_METHODS.shutdown &&
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
  return { jsonrpc: JSON_RPC_VERSION, id, error: { code, message } };
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
