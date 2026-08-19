import {
  decodeFrames,
  encodeFrame,
  type JsonValue,
  type RequestId,
} from "../protocol.ts";

/** The permissions Ora grants an agent plugin process; the default for simulations. */
export const AGENT_PLUGIN_PERMISSIONS = [
  "--allow-run",
  "--allow-read",
  "--allow-env",
  "--allow-net",
] as const;

export interface HostSimulatorOptions {
  /** Plugin entrypoint, as a path or a module URL (`import.meta.resolve("../src/main.ts")`). */
  entrypoint: string | URL;
  /** Deno permission flags to launch with; defaults to what Ora grants an agent plugin. */
  permissions?: readonly string[];
  /** Deno executable; defaults to the one running the simulator. */
  denoPath?: string;
  /**
   * Explicit Deno config for the plugin process (`--config`), replacing the package's own
   * `deno.json`; useful to point SDK imports at a local checkout while developing both sides.
   */
  configPath?: string;
  /** Working directory for the plugin process. */
  cwd?: string;
  /** Extra environment for the plugin process. */
  env?: Record<string, string>;
  /** How long to wait for any single expected frame before failing. */
  timeoutMs?: number;
}

/** The registration the plugin announced in `ora/register`. */
export interface PluginRegistration {
  methods: string[];
  emits: string[];
  sdkVersion?: string;
  contracts?: Record<string, number>;
}

/** One JSON-RPC response as the host sees it. */
export interface HostResponse {
  id: RequestId;
  result?: JsonValue;
  error?: { code: number; message: string };
}

type Frame = Record<string, JsonValue>;

/**
 * Drives one plugin process exactly the way the Ora host does.
 *
 * The simulator launches the entrypoint with Deno, speaks Ora's binary frame protocol on its
 * stdio, and exposes the handshake, request/response, notification, and ACP pass-through flows as
 * awaitable steps. Frames that arrive while waiting for a specific one — typically streamed
 * `agent/acp` notifications — are kept in `passedFrames` so a test can assert on them later
 * without the wait desynchronizing.
 */
export class HostSimulator {
  readonly registration: PluginRegistration;
  readonly passedFrames: Frame[] = [];
  readonly #child: Deno.ChildProcess;
  readonly #writer: WritableStreamDefaultWriter<Uint8Array>;
  readonly #inbound: AsyncIterator<unknown>;
  readonly #timeoutMs: number;
  #nextId = 1;

  private constructor(
    child: Deno.ChildProcess,
    inbound: AsyncIterator<unknown>,
    registration: PluginRegistration,
    timeoutMs: number,
  ) {
    this.#child = child;
    this.#writer = child.stdin.getWriter();
    this.#inbound = inbound;
    this.registration = registration;
    this.#timeoutMs = timeoutMs;
  }

  /** Launches the plugin and waits for its `ora/register` handshake. */
  static async launch(options: HostSimulatorOptions): Promise<HostSimulator> {
    const entrypoint = options.entrypoint instanceof URL
      ? moduleUrlToPath(options.entrypoint)
      : options.entrypoint.startsWith("file:")
      ? moduleUrlToPath(new URL(options.entrypoint))
      : options.entrypoint;
    const child = new Deno.Command(options.denoPath ?? Deno.execPath(), {
      args: [
        "run",
        "--no-prompt",
        ...(options.configPath === undefined
          ? []
          : ["--config", options.configPath]),
        ...(options.permissions ?? AGENT_PLUGIN_PERMISSIONS),
        entrypoint,
      ],
      cwd: options.cwd,
      env: options.env,
      stdin: "piped",
      stdout: "piped",
      stderr: "inherit",
    }).spawn();
    const inbound = decodeFrames(child.stdout)[Symbol.asyncIterator]();
    const timeoutMs = options.timeoutMs ?? 30_000;
    const register = await waitForFrame(
      inbound,
      (frame) => frame.method === "ora/register",
      "ora/register",
      timeoutMs,
      [],
    );
    const params = (register.params ?? {}) as Record<string, JsonValue>;
    const registration: PluginRegistration = {
      methods: (params.methods as string[] | undefined) ?? [],
      emits: (params.emits as string[] | undefined) ?? [],
      sdkVersion: params.sdkVersion as string | undefined,
      contracts: params.contracts as Record<string, number> | undefined,
    };
    return new HostSimulator(child, inbound, registration, timeoutMs);
  }

  /** Sends one host request and waits for the response carrying the same id. */
  async request(method: string, params: JsonValue = {}): Promise<HostResponse> {
    const id = this.#nextId++;
    await this.#send({ jsonrpc: "2.0", id, method, params });
    const frame = await this.#waitFor(
      (candidate) => candidate.id === id,
      method,
    );
    return {
      id,
      result: frame.result,
      error: frame.error as HostResponse["error"],
    };
  }

  /** Sends one host notification; nothing is awaited beyond the write. */
  notify(method: string, params: JsonValue = {}): Promise<void> {
    return this.#send({ jsonrpc: "2.0", method, params });
  }

  /**
   * Forwards one ACP request through `agent/acp` and waits for the ACP response with its id.
   *
   * The ACP id space is the plugin's own, independent of the host request ids, so the caller
   * supplies it explicitly.
   */
  async acpRequest(
    id: RequestId,
    method: string,
    params: JsonValue = {},
  ): Promise<Frame> {
    await this.notify("agent/acp", { jsonrpc: "2.0", id, method, params });
    const frame = await this.#waitFor(
      (candidate) =>
        candidate.method === "agent/acp" && acpPayload(candidate).id === id,
      `ACP ${method}`,
    );
    return acpPayload(frame);
  }

  /** Forwards one ACP notification through `agent/acp`. */
  acpNotify(method: string, params: JsonValue = {}): Promise<void> {
    return this.notify("agent/acp", { jsonrpc: "2.0", method, params });
  }

  /** Waits for the next `agent/acp` frame the plugin emits that satisfies `match`. */
  async nextAcp(
    match: (payload: Frame) => boolean = () => true,
    label = "agent/acp",
  ): Promise<Frame> {
    const frame = await this.#waitFor(
      (candidate) =>
        candidate.method === "agent/acp" && match(acpPayload(candidate)),
      label,
    );
    return acpPayload(frame);
  }

  /** Sends `ora/shutdown`, closes stdin, and returns the plugin's exit code. */
  async shutdown(): Promise<number> {
    await this.notify("ora/shutdown");
    await this.#writer.close();
    const status = await this.#child.status;
    return status.code;
  }

  /** Kills the plugin without a handshake, for tests that exercise failure paths. */
  async kill(): Promise<void> {
    try {
      this.#child.kill();
    } catch {
      // Already gone.
    }
    await this.#child.status;
  }

  #send(message: JsonValue): Promise<void> {
    return this.#writer.write(encodeFrame(message));
  }

  #waitFor(match: (frame: Frame) => boolean, label: string): Promise<Frame> {
    return waitForFrame(
      this.#inbound,
      match,
      label,
      this.#timeoutMs,
      this.passedFrames,
    );
  }
}

/** Reads frames until one satisfies `match`, parking the others so the stream never desyncs. */
async function waitForFrame(
  inbound: AsyncIterator<unknown>,
  match: (frame: Frame) => boolean,
  label: string,
  timeoutMs: number,
  passed: Frame[],
): Promise<Frame> {
  const deadline = Date.now() + timeoutMs;
  while (true) {
    const remaining = deadline - Date.now();
    if (remaining <= 0) {
      throw new Error(`timed out waiting for ${label}`);
    }
    const next = await Promise.race([
      inbound.next(),
      new Promise<never>((_, reject) =>
        setTimeout(
          () => reject(new Error(`timed out waiting for ${label}`)),
          remaining,
        )
      ),
    ]);
    if (next.done) {
      throw new Error(`plugin closed stdout while waiting for ${label}`);
    }
    const frame = next.value as Frame;
    if (match(frame)) {
      return frame;
    }
    passed.push(frame);
  }
}

/** Extracts the nested ACP message from one `agent/acp` frame. */
function acpPayload(frame: Frame): Frame {
  return (frame.params ?? {}) as Frame;
}

/** Converts a module URL into a host path, including a Windows drive prefix. */
function moduleUrlToPath(url: URL): string {
  return decodeURIComponent(url.pathname).replace(/^\/([A-Za-z]:)/, "$1");
}
